use std::io::BufReader;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use arc_swap::ArcSwapOption;
use protocol::frame::{read_frame, write_frame};
use protocol::{DriverToGui, GuiToDriver};

use crate::apply::{SideEffects, apply_delta};
use crate::error::{DriverError, DriverResult};
use crate::shared_settings::SharedSettings;

/// Lock-free slot holding the current subscriber's outbound sender, if any.
/// The loop and the midir callback read it to push events.
pub type EventSubscriber = Arc<ArcSwapOption<Sender<DriverToGui>>>;

/// Create an empty subscriber slot.
pub fn new_subscriber() -> EventSubscriber {
    Arc::new(ArcSwapOption::empty())
}

/// Push an event to the current subscriber, if one is registered. Never blocks
/// meaningfully and ignores send errors (a dead client is cleaned up by its
/// reader loop).
pub fn emit_event(subscriber: &EventSubscriber, msg: DriverToGui) {
    if let Some(tx) = subscriber.load_full() {
        let _ = tx.send(msg);
    }
}

fn bind_singleton(path: &Path) -> DriverResult<UnixListener> {
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DriverError::Ipc(format!("create socket dir {parent:?}: {e}")))?;
    }
    UnixListener::bind(path).map_err(|e| DriverError::Ipc(format!("bind {}: {e}", path.display())))
}

/// Running IPC server. Dropping it removes the socket file.
pub struct IpcServer {
    socket_path: PathBuf,
    _accept: JoinHandle<()>,
}

impl IpcServer {
    /// Bind the socket (singleton) and start accepting clients.
    pub fn start(
        handle: SharedSettings,
        config_path: PathBuf,
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
                        &config_path,
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

fn handle_client(
    stream: UnixStream,
    handle: &SharedSettings,
    config_path: &Path,
    effects_tx: &Sender<SideEffects>,
    subscriber: &EventSubscriber,
    device_present: &Arc<AtomicBool>,
) {
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let (out_tx, out_rx) = mpsc::channel::<DriverToGui>();
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
            config_path,
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

fn dispatch(
    req: GuiToDriver,
    handle: &SharedSettings,
    config_path: &Path,
    effects_tx: &Sender<SideEffects>,
    subscriber: &EventSubscriber,
    device_present: &Arc<AtomicBool>,
    out_tx: &Sender<DriverToGui>,
) -> Result<(), ()> {
    match req {
        GuiToDriver::GetSettings => out_tx.send(snapshot(handle)).map_err(|_| ())?,
        GuiToDriver::Apply {
            seq,
            delta,
            persist,
        } => match apply_delta(handle, *delta, config_path, persist) {
            Ok(effects) => {
                let _ = effects_tx.send(effects);
                out_tx
                    .send(DriverToGui::Ack {
                        seq,
                        result: Ok(()),
                    })
                    .map_err(|_| ())?;
                out_tx.send(snapshot(handle)).map_err(|_| ())?;
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
