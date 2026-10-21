//! Control socket: `$XDG_RUNTIME_DIR/layoutless.sock`, one text command per
//! connection, one reply line back. `subscribe` instead keeps the connection
//! open and streams JSON events, one per line (used by the bar widget).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::daemon::Msg;

/// Open `subscribe` connections.
#[derive(Clone, Default)]
pub struct Subscribers(Arc<Mutex<Vec<UnixStream>>>);

impl Subscribers {
    /// Send one event line to every subscriber, dropping the ones that left.
    pub fn send(&self, event: &serde_json::Value) {
        let line = event.to_string();
        self.0.lock().unwrap().retain_mut(|s| writeln!(s, "{line}").is_ok());
    }
}

fn socket_path() -> std::io::Result<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(std::io::Error::other)?;
    Ok(PathBuf::from(runtime).join("layoutless.sock"))
}

fn connect() -> std::io::Result<UnixStream> {
    let path = socket_path()?;
    UnixStream::connect(&path)
        .map_err(|e| std::io::Error::other(format!("daemon not running? {}: {e}", path.display())))
}

/// Send a command to the running daemon and return its reply.
pub fn request(cmd: &str) -> std::io::Result<String> {
    let mut s = connect()?;
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    writeln!(s, "{cmd}")?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    Ok(reply.trim_end().to_string())
}

/// Print the daemon's events to stdout until it goes away.
pub fn subscribe() -> std::io::Result<()> {
    let mut s = connect()?;
    writeln!(s, "subscribe")?;
    let mut out = std::io::stdout().lock();
    for line in BufReader::new(s).lines() {
        writeln!(out, "{}", line?)?;
        out.flush()?;
    }
    Ok(())
}

/// Listen for commands. Fails if another daemon already owns the socket.
pub fn serve(tx: Sender<Msg>, subscribers: Subscribers) -> std::io::Result<()> {
    let path = socket_path()?;
    if UnixStream::connect(&path).is_ok() {
        return Err(std::io::Error::other("another layoutless daemon is already running"));
    }
    let _ = std::fs::remove_file(&path); // stale socket from a crash
    let listener = UnixListener::bind(&path)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // One thread per connection, so a slow client never holds up the
            // next command (fast Alt+Space presses arrive back to back).
            let (tx, subscribers) = (tx.clone(), subscribers.clone());
            thread::spawn(move || {
                if let Err(e) = handle(stream, &tx, &subscribers) {
                    eprintln!("control: {e}");
                }
            });
        }
    });
    Ok(())
}

fn ask(tx: &Sender<Msg>, cmd: &str) -> std::io::Result<String> {
    let (rtx, rrx) = mpsc::channel();
    tx.send(Msg::Command(cmd.to_string(), rtx)).map_err(std::io::Error::other)?;
    Ok(rrx.recv_timeout(Duration::from_secs(2)).unwrap_or_else(|_| "timeout".into()))
}

fn handle(stream: UnixStream, tx: &Sender<Msg>, subscribers: &Subscribers) -> std::io::Result<()> {
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let cmd = line.trim();
    if cmd == "subscribe" {
        // Start with the current state, then events as they happen. A
        // subscriber that stops reading must not block the daemon.
        writeln!(&stream, "{}", ask(tx, "state")?)?;
        stream.set_write_timeout(Some(Duration::from_millis(200)))?;
        subscribers.0.lock().unwrap().push(stream);
        return Ok(());
    }
    writeln!(&stream, "{}", ask(tx, cmd)?)
}
