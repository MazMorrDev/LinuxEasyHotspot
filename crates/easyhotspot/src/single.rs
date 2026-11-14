//! One instance per user: launching the app again (or at login while it
//! runs) shows the running window instead of starting a second copy that
//! would fight over the hotspot.
//!
//! - Linux: a Unix socket in `$XDG_RUNTIME_DIR`.
//! - Windows: a loopback TCP port derived from the user name; the user name
//!   is checked, so another user's instance is never mistaken for ours.

use std::io::{BufRead as _, BufReader, Write as _};
use std::time::Duration;

/// Set in the environment of the process started by "Restart now": the old
/// one may still be listening for a moment.
pub const RESTARTED: &str = "EASYHOTSPOT_RESTARTED";

fn user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

fn hello() -> String {
    format!("show {}\n", user())
}

#[cfg(unix)]
mod imp {
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

    pub type Listener = UnixListener;
    pub type Stream = UnixStream;

    fn path() -> PathBuf {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(d) => PathBuf::from(d).join("easyhotspot.sock"),
            None => std::env::temp_dir().join(format!("easyhotspot-{}.sock", super::user())),
        }
    }

    pub fn connect() -> Option<Stream> {
        UnixStream::connect(path()).ok()
    }

    pub fn bind() -> Option<Listener> {
        let p = path();
        // Nobody answered, so a file left there is stale.
        let _ = std::fs::remove_file(&p);
        UnixListener::bind(&p).ok()
    }
}

#[cfg(windows)]
mod imp {
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    use std::time::Duration;

    pub type Listener = TcpListener;
    pub type Stream = TcpStream;

    fn addr() -> SocketAddr {
        // FNV-1a of the user name → a port in 41000..49000.
        let mut h: u32 = 0x811c_9dc5;
        for b in super::user().to_lowercase().bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
        SocketAddr::from((Ipv4Addr::LOCALHOST, 41_000 + (h % 8_000) as u16))
    }

    pub fn connect() -> Option<Stream> {
        TcpStream::connect_timeout(&addr(), Duration::from_millis(300)).ok()
    }

    pub fn bind() -> Option<Listener> {
        TcpListener::bind(addr()).ok()
    }
}

/// Listens for later launches.
pub struct Server(imp::Listener);

impl Server {
    /// Calls `on_show` (on a background thread) whenever the app is
    /// launched again by the same user.
    pub fn serve(self, on_show: impl Fn() + Send + 'static) {
        std::thread::spawn(move || {
            for stream in self.0.incoming().flatten() {
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let ok = line == hello();
                let mut w = &stream;
                let _ = w.write_all(if ok { b"ok\n" } else { b"no\n" });
                if ok {
                    on_show();
                }
            }
        });
    }
}

/// Asks a running instance to show itself. `Err` means it did (this process
/// should exit); otherwise returns the server to run as the first instance,
/// or `None` if the address can't be taken (the app then just runs).
pub fn claim() -> Result<Option<Server>, ()> {
    let restarted = std::env::var_os(RESTARTED).is_some();
    if !restarted && let Some(stream) = imp::connect() {
        let _ = stream_timeout(&stream);
        let mut w = &stream;
        if w.write_all(hello().as_bytes()).is_ok() {
            let mut reply = String::new();
            if BufReader::new(&stream).read_line(&mut reply).is_ok() && reply == "ok\n" {
                return Err(());
            }
        }
    }
    // After a restart the previous process may still hold the address.
    for _ in 0..if restarted { 30 } else { 1 } {
        if let Some(l) = imp::bind() {
            return Ok(Some(Server(l)));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(None)
}

fn stream_timeout(s: &imp::Stream) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.set_write_timeout(Some(Duration::from_secs(2)))
}
