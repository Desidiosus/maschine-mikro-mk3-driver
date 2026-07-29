use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use protocol::DriverToGui;

use crate::apply::apply_delta;
use crate::ipc::{EventSubscriber, emit_event};
use crate::settings::persist::save_active_page;
use crate::settings::{PartialSettings, Settings};
use crate::shared_settings::SharedSettings;
use settings::partial::PartialPadPaging;

/// Serializes every settings write (IPC applies and hardware page applies) so the
/// two writer paths cannot lose a read-modify-write against `SharedSettings`.
pub type WriteLock = Arc<Mutex<()>>;

pub fn new_write_lock() -> WriteLock {
    Arc::new(Mutex::new(()))
}

/// A hardware page request from the realtime loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageApplyMsg {
    /// Apply this active page live (no disk) and push a fresh snapshot to the GUI.
    Preview(usize),
    /// Persist this page as the active one (sent once on `Group` release). Carries
    /// the page rather than persisting whatever is live, so a commit still queued
    /// at shutdown can't write out an unrelated value the GUI was only previewing.
    Commit(usize),
}

/// A minimal `PartialSettings` delta that sets only `pad_paging.active`.
pub fn active_page_delta(active: usize) -> PartialSettings {
    PartialSettings {
        pad_paging: Some(PartialPadPaging {
            active: Some(active),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn snapshot_msg(handle: &SharedSettings) -> DriverToGui {
    DriverToGui::Settings(Box::new((*handle.load_full()).clone()))
}

/// Spawn the page-apply writer. Returns the sender the loop uses and the join
/// handle. The thread exits when all senders drop (device gone / shutdown).
pub fn spawn_page_apply_writer(
    handle: SharedSettings,
    persist_base: Arc<Settings>,
    persist_path: PathBuf,
    lock: WriteLock,
    subscriber: EventSubscriber,
) -> (Sender<PageApplyMsg>, JoinHandle<()>) {
    spawn_writer(handle, persist_base, Some(persist_path), lock, subscriber)
}

/// Spawn a page-apply writer for a caller with no config file of its own
/// (`run_with_device`, which is handed its settings rather than loading them):
/// page switches apply live, and a commit has nothing to write.
pub fn spawn_live_page_applier(
    handle: SharedSettings,
    subscriber: EventSubscriber,
) -> (Sender<PageApplyMsg>, JoinHandle<()>) {
    let base = handle.load_full();
    spawn_writer(handle, base, None, new_write_lock(), subscriber)
}

fn spawn_writer(
    handle: SharedSettings,
    persist_base: Arc<Settings>,
    persist_path: Option<PathBuf>,
    lock: WriteLock,
    subscriber: EventSubscriber,
) -> (Sender<PageApplyMsg>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<PageApplyMsg>();
    let join = thread::spawn(move || {
        for msg in rx {
            match msg {
                PageApplyMsg::Preview(active) => {
                    let applied = {
                        let _guard = lock.lock().unwrap();
                        // A live apply reads neither the persistence base nor the
                        // path, so a writer with nowhere to persist to still works.
                        apply_delta(
                            &handle,
                            active_page_delta(active),
                            &persist_base,
                            persist_path.as_deref().unwrap_or(Path::new("")),
                            false,
                        )
                    };
                    // `merge_overrides` clamps an out-of-range target to the last
                    // page, so a valid live `Settings` yields `Ok`; a failed apply
                    // is skipped rather than tearing down the writer.
                    if applied.is_ok() {
                        emit_event(&subscriber, snapshot_msg(&handle));
                    }
                }
                PageApplyMsg::Commit(active) => {
                    if let Some(path) = &persist_path {
                        let _guard = lock.lock().unwrap();
                        let _ = save_active_page(path, active);
                    }
                }
            }
        }
    });
    (tx, join)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::new_subscriber;
    use crate::shared_settings::new_shared;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("mmk3-writer-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn two_page_settings() -> Settings {
        let mut s = Settings::default();
        s.pad_paging.pages.push(s.pad_paging.new_page());
        s
    }

    #[test]
    fn preview_updates_active_live_without_persisting() {
        let path = temp_path("preview");
        let handle = new_shared(two_page_settings());
        let base = Arc::new(two_page_settings());
        let (tx, join) = spawn_page_apply_writer(
            handle.clone(),
            base,
            path.clone(),
            new_write_lock(),
            new_subscriber(),
        );

        tx.send(PageApplyMsg::Preview(1)).unwrap();
        drop(tx); // end the thread
        join.join().unwrap();

        assert_eq!(handle.load().pad_paging.active, 1);
        assert!(!path.exists(), "Preview must not write to disk");
    }

    #[test]
    fn commit_persists_the_selected_page() {
        let path = temp_path("commit");
        let handle = new_shared(two_page_settings());
        let base = Arc::new(two_page_settings());
        let (tx, join) = spawn_page_apply_writer(
            handle.clone(),
            base,
            path.clone(),
            new_write_lock(),
            new_subscriber(),
        );

        tx.send(PageApplyMsg::Preview(1)).unwrap();
        tx.send(PageApplyMsg::Commit(1)).unwrap();
        drop(tx);
        join.join().unwrap();

        // Reload against the same two-page base the writer was given — not
        // `load_xdg`, which hardcodes the (one-page) `Settings::default()` and
        // would self-heal `active` back to the last page.
        let raw = std::fs::read_to_string(&path).unwrap();
        let overrides: PartialSettings = toml::from_str(&raw).unwrap();
        let reloaded = two_page_settings().merge_overrides(overrides);
        assert_eq!(
            reloaded.pad_paging.active, 1,
            "Commit persists the selected page"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn commit_leaves_live_only_edits_unpersisted() {
        // A GUI slider mid-drag is applied live with `persist = false`. A page
        // commit must not sweep that value onto disk: the commit can be drained
        // during shutdown, after the GUI is gone, with no later Persist to
        // correct it.
        let path = temp_path("commit-live-only");
        let handle = new_shared(two_page_settings());
        let base = Arc::new(two_page_settings());
        let (tx, join) = spawn_page_apply_writer(
            handle.clone(),
            base.clone(),
            path.clone(),
            new_write_lock(),
            new_subscriber(),
        );

        let mut previewed = two_page_settings();
        previewed.hardware.display_contrast = 42;
        handle.store(Arc::new(previewed));

        tx.send(PageApplyMsg::Commit(1)).unwrap();
        drop(tx);
        join.join().unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let overrides: PartialSettings = toml::from_str(&raw).unwrap();
        assert_eq!(
            overrides.pad_paging.and_then(|pp| pp.active),
            Some(1),
            "the page itself is persisted"
        );
        assert!(
            overrides.hardware.is_none(),
            "a value the GUI is only previewing must stay off disk"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_live_only_applier_switches_pages_without_touching_disk() {
        // `run_with_device` has no config file; page switching must still work
        // rather than queueing messages nothing ever reads.
        let handle = new_shared(two_page_settings());
        let (tx, join) = spawn_live_page_applier(handle.clone(), new_subscriber());

        tx.send(PageApplyMsg::Preview(1)).unwrap();
        tx.send(PageApplyMsg::Commit(1)).unwrap();
        drop(tx);
        join.join().unwrap();

        assert_eq!(handle.load().pad_paging.active, 1);
    }

    #[test]
    fn concurrent_writers_under_the_shared_lock_do_not_lose_updates() {
        // Two threads hammer apply_delta through the shared lock. The final state
        // must be internally consistent (a valid Settings), never a torn swap.
        let path = temp_path("race");
        let handle = new_shared(two_page_settings());
        let base = Arc::new(two_page_settings());
        let lock = new_write_lock();

        let mut joins = Vec::new();
        for target in [0usize, 1usize] {
            let (h, b, p, l) = (handle.clone(), base.clone(), path.clone(), lock.clone());
            joins.push(thread::spawn(move || {
                for _ in 0..200 {
                    let _g = l.lock().unwrap();
                    let _ = apply_delta(&h, active_page_delta(target), &b, &p, false);
                }
            }));
        }
        for j in joins {
            j.join().unwrap();
        }
        handle
            .load()
            .validate()
            .expect("no torn write under the lock");
        let _ = std::fs::remove_file(&path);
    }
}
