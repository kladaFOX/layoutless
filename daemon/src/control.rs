//! Control socket: `$XDG_RUNTIME_DIR/layoutless.sock`, one text command per
//! connection, one reply line back.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use crate::daemon::Msg;

fn socket_path() -> std::io::Result<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(std::io::Error::other)?;
    Ok(PathBuf::from(runtime).join("layoutless.sock"))
}

/// Send a command to the running daemon and return its reply.
pub fn request(cmd: &str) -> std::io::Result<String> {
    let path = socket_path()?;
    let mut s = UnixStream::connect(&path)
        .map_err(|e| std::io::Error::other(format!("daemon not running? {}: {e}", path.display())))?;
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    writeln!(s, "{cmd}")?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    Ok(reply.trim_end().to_string())
}

/// Listen for commands. Fails if another daemon already owns the socket.
pub fn serve(tx: Sender<Msg>) -> std::io::Result<()> {
    let path = socket_path()?;
    if UnixStream::connect(&path).is_ok() {
        return Err(std::io::Error::other("another layoutless daemon is already running"));
    }
    let _ = std::fs::remove_file(&path); // stale socket from a crash
    let listener = UnixListener::bind(&path)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Err(e) = handle(stream, &tx) {
                eprintln!("control: {e}");
            }
        }
    });
    Ok(())
}

fn handle(stream: UnixStream, tx: &Sender<Msg>) -> std::io::Result<()> {
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let (rtx, rrx) = mpsc::channel();
    tx.send(Msg::Command(line.trim().to_string(), rtx)).map_err(std::io::Error::other)?;
    let reply = rrx.recv_timeout(Duration::from_secs(2)).unwrap_or_else(|_| "timeout".into());
    writeln!(&stream, "{reply}")
}
