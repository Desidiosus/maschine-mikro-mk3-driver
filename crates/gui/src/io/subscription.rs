use std::io::BufReader;
use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use futures::channel::mpsc as fmpsc;
use protocol::frame::{read_frame, write_frame};
use protocol::{DriverToGui, GuiToDriver};

use crate::io::connection::{connect_or_spawn, resolve_driver_bin};
use crate::message::Message;

/// Sleep without blocking the executor: a throwaway thread does the wait and
/// wakes the awaiting task, so this works on any runtime (iced doesn't expose a
/// portable timer for use inside a `stream::channel`).
async fn sleep(dur: Duration) {
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(dur);
        let _ = tx.send(());
    });
    let _ = rx.await;
}

/// One connect-or-spawn request handed to the long-lived connector thread.
pub(crate) struct ConnectRequest {
    pub(crate) socket_path: std::path::PathBuf,
    pub(crate) driver_bin: std::path::PathBuf,
    pub(crate) reply:
        futures::channel::oneshot::Sender<std::io::Result<crate::io::connection::Connection>>,
}

/// Serve connect-or-spawn requests on a single long-lived thread.
///
/// `connect_or_spawn` sets `PR_SET_PDEATHSIG` on the spawned driver keyed to the
/// calling *thread*, so spawning MUST happen on a thread that outlives the driver
/// — a per-connect throwaway thread would SIGTERM the driver the instant it
/// exited, and the reconnect loop would respawn it forever. This thread serves
/// every (re)connect for a subscription run and ends only when the request sender
/// drops (the subscription is gone), which is exactly when the last spawned
/// driver should receive its parent-death signal.
pub(crate) fn connector_thread(requests: std::sync::mpsc::Receiver<ConnectRequest>) {
    for req in requests {
        let result = connect_or_spawn(&req.socket_path, &req.driver_bin, Duration::from_secs(5));
        let _ = req.reply.send(result);
    }
}

/// Dispatch a connect-or-spawn to the connector thread and await its result, so
/// the up-to-5s connect/spawn poll never blocks the iced executor (freezing the
/// UI) yet still runs on a thread that outlives the spawned driver.
async fn connect_via(
    requests: &std::sync::mpsc::Sender<ConnectRequest>,
    socket_path: std::path::PathBuf,
    driver_bin: std::path::PathBuf,
) -> std::io::Result<crate::io::connection::Connection> {
    let (reply, rx) = futures::channel::oneshot::channel();
    if requests
        .send(ConnectRequest {
            socket_path,
            driver_bin,
            reply,
        })
        .is_err()
    {
        return Err(std::io::Error::other("connector thread gone"));
    }
    rx.await
        .unwrap_or_else(|_| Err(std::io::Error::other("connect request cancelled")))
}

/// The connection lifecycle: connect-or-spawn, bridge the socket to iced
/// messages, and reconnect with backoff after the link drops so the GUI recovers
/// when the driver restarts. Runs on the iced executor (a long-lived thread).
pub fn driver_connection() -> impl futures::Stream<Item = Message> {
    iced::stream::channel(100, async move |mut output| {
        let socket_path = match protocol::socket_path() {
            Ok(p) => p,
            Err(e) => {
                let _ = output.send(Message::Error(e)).await;
                return;
            }
        };
        let driver_bin = resolve_driver_bin();

        // One long-lived connector thread for this subscription run. The driver's
        // PR_SET_PDEATHSIG keys on the thread that spawns it, so spawning must
        // happen here (not on a per-connect throwaway thread, which would SIGTERM
        // the driver the moment it exited and trip the reconnect loop into an
        // endless respawn storm). The thread ends when the subscription drops the
        // request sender.
        let (req_tx, req_rx) = std::sync::mpsc::channel::<ConnectRequest>();
        std::thread::spawn(move || connector_thread(req_rx));

        const MIN_BACKOFF: Duration = Duration::from_millis(250);
        // A connection must stay up at least this long to count as healthy and
        // reset the backoff. A driver that binds its socket then exits at once
        // (crash loop) connects successfully every time; resetting on every
        // connect would pin the respawn cadence at ~4x/sec forever, so we only
        // reset after a connection that actually lasted.
        const HEALTHY: Duration = Duration::from_secs(2);
        let mut backoff = MIN_BACKOFF;
        loop {
            match connect_via(&req_tx, socket_path.clone(), driver_bin.clone()).await {
                Ok(conn) => {
                    let started = std::time::Instant::now();
                    let end = run_connection(conn, &mut output).await;
                    if started.elapsed() >= HEALTHY {
                        backoff = MIN_BACKOFF;
                    }
                    if output.send(end).await.is_err() {
                        return; // GUI dropped the subscription
                    }
                }
                Err(e) => {
                    if output.send(Message::Error(e.to_string())).await.is_err() {
                        return;
                    }
                }
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    })
}

/// Bridge one live connection to iced messages until it ends. Returns the
/// terminal message (`Disconnected` on a clean close, `Error` on a protocol/IO
/// failure). The spawned driver (if any) is torn down when this returns.
async fn run_connection<S>(conn: crate::io::connection::Connection, output: &mut S) -> Message
where
    S: futures::Sink<Message> + Unpin,
{
    let read_stream = match conn.stream.try_clone() {
        Ok(s) => s,
        Err(e) => return Message::Error(e.to_string()),
    };
    let write_stream = conn.stream;
    // Bound how long a write can block so a half-open socket (driver gone, no RST
    // yet) can't pin the detached writer thread — and leak it plus its fd — for the
    // rest of the process on every reconnect.
    let _ = write_stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _child = conn.child; // kept alive for the connection's lifetime → teardown on drop

    // Reader OS thread: blocking frame reads → a futures channel. A frame error
    // (decode/oversize/IO) is recorded so it surfaces as a distinct Error rather
    // than looking like a clean disconnect.
    let (in_tx, mut in_rx) = fmpsc::unbounded::<DriverToGui>();
    let read_err = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let reader_err = read_err.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(read_stream);
        loop {
            match read_frame::<_, DriverToGui>(&mut reader) {
                Ok(Some(msg)) => {
                    if in_tx.unbounded_send(msg).is_err() {
                        break;
                    }
                }
                Ok(None) => break, // clean EOF: peer closed
                Err(e) => {
                    *reader_err.lock().unwrap() = Some(e.to_string());
                    break;
                }
            }
        }
    });

    // Writer OS thread: drains a std channel of requests → frames.
    let (out_tx, out_rx) = std::sync::mpsc::channel::<GuiToDriver>();
    std::thread::spawn(move || {
        let mut w = write_stream;
        for msg in out_rx {
            if write_frame(&mut w, &msg).is_err() {
                break;
            }
        }
    });

    if output.send(Message::Ready(out_tx)).await.is_err() {
        return Message::Disconnected;
    }
    while let Some(frame) = in_rx.next().await {
        if output.send(Message::Frame(frame)).await.is_err() {
            return Message::Disconnected;
        }
    }

    match read_err.lock().unwrap().take() {
        Some(e) => Message::Error(format!("connection lost: {e}")),
        None => Message::Disconnected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    fn unique_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("mmk3-connector-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// Fake "driver": binds `sock` and sleeps, so `connect_or_spawn` spawns it and
    /// connects (returning a child). Run with no args, like the real driver.
    fn fake_driver(sock: &std::path::Path) -> PathBuf {
        let dir = std::env::temp_dir().join("mmk3-connector-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("fake-driver-{}.sh", std::process::id()));
        let body = format!(
            "#!/bin/sh\nexec python3 -c '\nimport socket, time\n\
             s = socket.socket(socket.AF_UNIX)\n\
             s.bind(\"{}\")\n\
             s.listen(16)\n\
             time.sleep(30)\n'\n",
            sock.display()
        );
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A driver spawned through the connector thread must stay alive after its
    /// connect request completes — the regression: spawning on a per-connect
    /// throwaway thread keyed the driver's `PR_SET_PDEATHSIG` to a thread that
    /// exited immediately, SIGTERM-ing the driver and tripping the reconnect loop
    /// into an endless respawn storm. The connector thread outlives the driver,
    /// and only its exit (subscription gone) delivers the parent-death signal.
    #[test]
    fn connector_keeps_spawned_driver_alive_until_it_exits() {
        let sock = unique_socket("alive");
        let fake = fake_driver(&sock);
        let (req_tx, req_rx) = std::sync::mpsc::channel::<ConnectRequest>();
        let connector = std::thread::spawn(move || connector_thread(req_rx));

        // Spawn + connect via the long-lived connector (poll the reply without an
        // executor).
        let (reply, mut rx) = futures::channel::oneshot::channel();
        req_tx
            .send(ConnectRequest {
                socket_path: sock.clone(),
                driver_bin: fake.clone(),
                reply,
            })
            .unwrap();
        let conn = loop {
            match rx.try_recv() {
                Ok(Some(res)) => break res.expect("connector spawns and connects"),
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => panic!("connector dropped the reply"),
            }
        };
        assert!(conn.child.is_some(), "fake driver was spawned");

        // The connect request has returned. The driver must still be up.
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            UnixStream::connect(&sock).is_ok(),
            "driver was killed right after spawn (PDEATHSIG keyed to a dead thread)"
        );

        // Ending the connector thread (subscription dropped) delivers the
        // parent-death signal, tearing the driver down.
        drop(req_tx);
        connector.join().unwrap();
        let dead = (0..50).any(|_| {
            if UnixStream::connect(&sock).is_err() {
                true
            } else {
                std::thread::sleep(Duration::from_millis(20));
                false
            }
        });
        assert!(
            dead,
            "driver should get PDEATHSIG when the connector thread exits"
        );

        drop(conn);
        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_file(&fake);
    }
}
