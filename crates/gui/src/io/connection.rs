use std::io;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Capped tail of the driver's stderr retained for error reporting.
const STDERR_TAIL_CAP: usize = 16 * 1024;

/// Owns a driver process the GUI spawned, terminating it on drop (SIGTERM, then
/// SIGKILL after a grace period). A `PR_SET_PDEATHSIG` set on the child is a
/// backup for a hard GUI crash.
#[derive(Debug)]
pub struct ChildGuard {
    child: Child,
    /// Capped tail of the child's stderr, filled by `drain`.
    stderr: Arc<Mutex<String>>,
    /// Thread draining the child's stderr so its pipe never fills (a full pipe
    /// would block the driver's writes and wedge its single-threaded loop).
    drain: Option<JoinHandle<()>>,
}

impl ChildGuard {
    /// Child PID as a `pid_t`, or `None` if it doesn't fit (a negative value
    /// would make `libc::kill` target a process group, not the child).
    fn pid(&self) -> Option<libc::pid_t> {
        libc::pid_t::try_from(self.child.id()).ok()
    }

    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    /// The captured stderr tail. Call only after the child has exited: its
    /// stderr is then at EOF, so joining the drain thread captures everything.
    fn captured_stderr(&mut self) -> String {
        if let Some(handle) = self.drain.take() {
            let _ = handle.join();
        }
        self.stderr
            .lock()
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.pid() {
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A live connection to the driver, plus the spawned child (if the GUI spawned
/// it). Dropping `child` tears the driver down; a pre-existing driver leaves
/// `child` `None`.
#[derive(Debug)]
pub struct Connection {
    pub stream: UnixStream,
    pub child: Option<ChildGuard>,
}

fn spawn_driver(driver_bin: &Path) -> io::Result<ChildGuard> {
    let mut cmd = Command::new(driver_bin);
    cmd.stderr(Stdio::piped());
    // SAFETY: pre_exec runs in the forked child before exec; prctl is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGTERM as libc::c_ulong,
                0,
                0,
                0,
            );
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;

    // Continuously drain the child's piped stderr on a thread: echo it to the
    // GUI's stderr (so driver logs are visible) and keep a capped tail for error
    // reporting. Without this, a steadily-logging driver fills the pipe buffer
    // and blocks on write, wedging its loop.
    let stderr = Arc::new(Mutex::new(String::new()));
    let drain = child.stderr.take().map(|err| {
        let buf = Arc::clone(&stderr);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(err);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        eprint!("[driver] {line}");
                        if let Ok(mut b) = buf.lock() {
                            b.push_str(&line);
                            if b.len() > STDERR_TAIL_CAP {
                                // Snap to the next char boundary: a raw byte offset
                                // can land mid-codepoint (driver logs are UTF-8, not
                                // ASCII), and `split_off` panics off a boundary.
                                let mut at = b.len() - STDERR_TAIL_CAP / 2;
                                while !b.is_char_boundary(at) {
                                    at += 1;
                                }
                                let tail = b.split_off(at);
                                *b = tail;
                            }
                        }
                    }
                }
            }
        })
    });

    Ok(ChildGuard {
        child,
        stderr,
        drain,
    })
}

/// Connect to the driver at `socket_path`. If nothing is listening, spawn
/// `driver_bin` and poll for the socket up to `timeout`.
///
/// Spawn `pre_exec`/`PR_SET_PDEATHSIG` keys on the calling thread — call this
/// from a long-lived thread (the iced executor/subscription task).
pub fn connect_or_spawn(
    socket_path: &Path,
    driver_bin: &Path,
    timeout: Duration,
) -> io::Result<Connection> {
    if let Ok(stream) = UnixStream::connect(socket_path) {
        return Ok(Connection {
            stream,
            child: None,
        });
    }

    // The spawned child may already have exited (e.g. it lost the driver's
    // singleton bind to a pre-existing instance). Benign: `ChildGuard::drop`
    // reaps the zombie, and the not-yet-reaped PID can't be reused.
    let mut child = spawn_driver(driver_bin)?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(stream) = UnixStream::connect(socket_path) {
            return Ok(Connection {
                stream,
                child: Some(child),
            });
        }
        if let Some(status) = child.try_wait()? {
            // The spawned driver exited — possibly because it lost the singleton
            // bind to a pre-existing instance. Try once more to attach to whoever
            // owns the socket before reporting failure.
            if let Ok(stream) = UnixStream::connect(socket_path) {
                return Ok(Connection {
                    stream,
                    child: None,
                });
            }
            let stderr = child.captured_stderr();
            return Err(io::Error::other(format!(
                "driver exited ({status}) before creating its socket: {stderr}"
            )));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "driver did not create socket {} within {timeout:?} \
                     (is the driver binary current? rebuild with `cargo build -p driver`)",
                    socket_path.display()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Resolve the driver binary: a sibling of the GUI executable, else `driver` on
/// `PATH`.
pub fn resolve_driver_bin() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("driver");
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from("driver")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    fn unique(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("mmk3-gui-conn-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn connects_to_existing_socket_without_spawning() {
        let path = unique("existing");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut byte = [0u8; 1];
                let _ = s.read(&mut byte);
                let _ = s.write_all(&byte);
            }
        });

        let conn = connect_or_spawn(
            &path,
            Path::new("/nonexistent/driver"),
            Duration::from_secs(1),
        )
        .expect("connects to existing socket");
        assert!(conn.child.is_none(), "must not spawn when socket is live");

        let mut stream = conn.stream;
        stream.write_all(&[0x42]).unwrap();
        let mut back = [0u8; 1];
        stream.read_exact(&mut back).unwrap();
        assert_eq!(back[0], 0x42);

        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn errors_when_no_socket_and_driver_missing() {
        let path = unique("missing");
        let result = connect_or_spawn(
            &path,
            Path::new("/nonexistent/driver-binary"),
            Duration::from_millis(200),
        );
        assert!(result.is_err(), "spawn of a missing binary must error");
    }

    /// Write an executable shell script to a temp path and return it.
    fn script(name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join("mmk3-gui-spawn-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}-{}.sh", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh\n{body}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn surfaces_driver_stderr_when_spawned_driver_exits() {
        let sock = unique("exits"); // no socket will ever appear
        let fake = script("boom", "echo 'HID I/O error: device not found' >&2\nexit 3");
        let err = connect_or_spawn(&sock, &fake, Duration::from_secs(2))
            .expect_err("must error when the driver exits");
        let msg = err.to_string();
        assert!(msg.contains("device not found"), "stderr surfaced: {msg}");
    }

    #[test]
    fn times_out_with_hint_when_driver_runs_without_binding() {
        let sock = unique("nobind");
        // Runs longer than the timeout, never binds the socket.
        let fake = script("hang", "sleep 5");
        let err =
            connect_or_spawn(&sock, &fake, Duration::from_millis(400)).expect_err("must time out");
        let msg = err.to_string();
        assert!(
            msg.contains("did not create socket"),
            "timeout message: {msg}"
        );
    }
}
