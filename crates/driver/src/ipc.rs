use std::io::BufReader;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use protocol::frame::{read_frame, write_frame};
use protocol::{DriverToGui, GuiToDriver};

use crate::apply::{SideEffects, apply_delta};
use crate::error::{DriverError, DriverResult};
use crate::settings::Settings;
use crate::shared_settings::SharedSettings;

/// Capacity of each client's outbound frame queue. Responses (Ack/snapshot) use
/// blocking sends and tolerate brief back-pressure; events use `try_send` and are
/// dropped once this many frames are queued, so a stalled-but-alive client can
/// never make the realtime producer threads grow memory without bound.
const OUT_QUEUE_CAP: usize = 256;

/// Upper bound on a single blocking write to a client. A client that stops
/// reading must not pin the writer thread (and through it the serial accept loop)
/// indefinitely; on timeout the write errors, the writer exits, and the client is
/// dropped.
const IDLE_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Lock-free slot holding the current subscriber's outbound sender, if any.
/// The loop and the midir callback read it to push events.
pub type EventSubscriber = Arc<ArcSwapOption<SyncSender<DriverToGui>>>;

/// Create an empty subscriber slot.
pub fn new_subscriber() -> EventSubscriber {
    Arc::new(ArcSwapOption::empty())
}

/// Push an event to the current subscriber, if one is registered. Best-effort:
/// the event is dropped if the client's queue is full (a stalled socket must not
/// let the realtime HID loop / midir callback threads block or grow memory).
pub fn emit_event(subscriber: &EventSubscriber, msg: DriverToGui) {
    if let Some(tx) = subscriber.load_full() {
        let _ = tx.try_send(msg);
    }
}

fn bind_singleton(path: &Path) -> DriverResult<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DriverError::Ipc(format!("create socket dir {parent:?}: {e}")))?;
    }
    // Serialize the stale-socket cleanup + bind across processes with an flock on
    // a sidecar lock file. This closes the TOCTOU window between the liveness
    // probe and the bind when two drivers start concurrently — without it both
    // could see a stale socket, both remove it, and both bind.
    let lock_path = path.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| DriverError::Ipc(format!("open lock {lock_path:?}: {e}")))?;
    // SAFETY: `lock` owns a valid fd for the duration of the call; LOCK_EX blocks
    // until the lock is acquired and is released when `lock` drops.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(DriverError::Ipc(format!(
            "flock {lock_path:?}: {}",
            std::io::Error::last_os_error()
        )));
    }

    if path.exists() {
        // A live server answering on the path means another instance is running.
        if UnixStream::connect(path).is_ok() {
            return Err(DriverError::Ipc(format!(
                "driver already running at {}",
                path.display()
            )));
        }
        let _ = std::fs::remove_file(path);
    }
    let listener = UnixListener::bind(path)
        .map_err(|e| DriverError::Ipc(format!("bind {}: {e}", path.display())))?;
    drop(lock);
    Ok(listener)
}

/// Running IPC server. Dropping it removes the socket file.
pub struct IpcServer {
    socket_path: PathBuf,
    _accept: JoinHandle<()>,
}

impl IpcServer {
    /// Bind the socket (singleton) and start accepting clients.
    ///
    /// `persist_base` is the read-only persistence base (`defaults ∘ -c seed`)
    /// and `persist_path` the XDG file GUI edits are written to.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        handle: SharedSettings,
        persist_base: Arc<Settings>,
        persist_path: PathBuf,
        effects_tx: Sender<SideEffects>,
        subscriber: EventSubscriber,
        socket_path: PathBuf,
        device_present: Arc<AtomicBool>,
    ) -> DriverResult<Self> {
        let listener = bind_singleton(&socket_path)?;
        let accept = thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => handle_client(
                        stream,
                        &handle,
                        &persist_base,
                        &persist_path,
                        &effects_tx,
                        &subscriber,
                        &device_present,
                    ),
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            socket_path,
            _accept: accept,
        })
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_client(
    stream: UnixStream,
    handle: &SharedSettings,
    persist_base: &Settings,
    persist_path: &Path,
    effects_tx: &Sender<SideEffects>,
    subscriber: &EventSubscriber,
    device_present: &Arc<AtomicBool>,
) {
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    // A stalled-but-alive client fills its socket buffer; cap how long a write may
    // block so the writer thread can't wedge here (and, since clients are served
    // serially, stall the accept loop) waiting on a reader that never drains.
    let _ = write_stream.set_write_timeout(Some(IDLE_WRITE_TIMEOUT));
    let (out_tx, out_rx) = mpsc::sync_channel::<DriverToGui>(OUT_QUEUE_CAP);
    let writer = thread::spawn(move || {
        let mut w = write_stream;
        for msg in out_rx {
            if write_frame(&mut w, &msg).is_err() {
                break;
            }
        }
    });

    let mut reader = BufReader::new(stream);
    while let Ok(Some(req)) = read_frame::<_, GuiToDriver>(&mut reader) {
        if dispatch(
            req,
            handle,
            persist_base,
            persist_path,
            effects_tx,
            subscriber,
            device_present,
            &out_tx,
        )
        .is_err()
        {
            break;
        }
    }

    // Client gone: clear the subscriber and end the writer by dropping the last
    // sender. Clearing unconditionally is correct only because clients are
    // served serially (the accept loop runs one `handle_client` to completion at
    // a time), so the slot can only ever hold THIS client's sender. Supporting
    // concurrent clients would require keying the subscriber by client so a
    // disconnect doesn't unsubscribe another client and pin its writer thread.
    subscriber.store(None);
    drop(out_tx);
    let _ = writer.join();
}

fn snapshot(handle: &SharedSettings) -> DriverToGui {
    DriverToGui::Settings(Box::new((*handle.load_full()).clone()))
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    req: GuiToDriver,
    handle: &SharedSettings,
    persist_base: &Settings,
    persist_path: &Path,
    effects_tx: &Sender<SideEffects>,
    subscriber: &EventSubscriber,
    device_present: &Arc<AtomicBool>,
    out_tx: &SyncSender<DriverToGui>,
) -> Result<(), ()> {
    match req {
        GuiToDriver::GetSettings => out_tx.send(snapshot(handle)).map_err(|_| ())?,
        GuiToDriver::Apply {
            seq,
            delta,
            persist,
        } => match apply_delta(handle, *delta, persist_base, persist_path, persist) {
            Ok(effects) => {
                // Only queue hardware side effects when a device is present to
                // apply them. With no device the loop isn't draining the channel,
                // so queued effects would grow unbounded and be discarded on the
                // next connect anyway — startup preferences re-push the persisted
                // settings to a freshly opened device.
                if device_present.load(Ordering::Acquire) {
                    let _ = effects_tx.send(effects);
                }
                out_tx
                    .send(DriverToGui::Ack {
                        seq,
                        result: Ok(()),
                    })
                    .map_err(|_| ())?;
                // Only push the authoritative snapshot on a persisted (commit)
                // apply. Live-preview drags (persist=false) fire many applies per
                // second; the GUI already merged each optimistically, so echoing
                // a full snapshot per tick would flood the socket and could snap
                // the value back to a stale in-flight snapshot mid-drag.
                if persist {
                    out_tx.send(snapshot(handle)).map_err(|_| ())?;
                }
            }
            Err(message) => out_tx
                .send(DriverToGui::Ack {
                    seq,
                    result: Err(message),
                })
                .map_err(|_| ())?,
        },
        GuiToDriver::SubscribeEvents => {
            subscriber.store(Some(Arc::new(out_tx.clone())));
            out_tx
                .send(DriverToGui::DeviceConnected(
                    device_present.load(Ordering::Acquire),
                ))
                .map_err(|_| ())?;
        }
    }
    Ok(())
}
