//! Hyprland IPC: layout switching (socket1) and layout events (socket2).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

fn socket_dir() -> std::io::Result<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(std::io::Error::other)?;
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").map_err(std::io::Error::other)?;
    Ok(PathBuf::from(runtime).join("hypr").join(sig))
}

/// Run a raw socket1 command and return the reply.
pub fn command(cmd: &str) -> std::io::Result<String> {
    let mut s = UnixStream::connect(socket_dir()?.join(".socket.sock"))?;
    s.write_all(cmd.as_bytes())?;
    let mut reply = String::new();
    s.read_to_string(&mut reply)?;
    Ok(reply)
}

pub fn switch_layout(device: &str, index: usize) -> std::io::Result<()> {
    let reply = command(&format!("switchxkblayout {device} {index}"))?;
    if reply.trim() != "ok" {
        return Err(std::io::Error::other(format!("switchxkblayout: {reply}")));
    }
    Ok(())
}

struct Snapshot {
    /// The real keyboard the current layout is read from.
    reference: String,
    reference_index: usize,
    watched_index: Option<usize>,
}

/// Read layout indices from `hyprctl devices`.
///
/// Don't use the keyboard flagged `main`: with fcitx5 running, `main` is its
/// virtual keyboard, which `switchxkblayout all` doesn't touch. Don't use our
/// own virtual keyboard either: it starts at index 0, and `switchxkblayout all
/// next` cycles each device separately, so it can be out of sync.
fn snapshot(watched: &str) -> std::io::Result<Snapshot> {
    let reply = command("j/devices")?;
    let v: serde_json::Value = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    let kbds = v["keyboards"].as_array().cloned().unwrap_or_default();
    let index = |k: &serde_json::Value| k["active_layout_index"].as_u64().unwrap_or(0) as usize;
    let reference = kbds
        .iter()
        .find(|k| {
            let name = k["name"].as_str().unwrap_or("");
            !name.contains("virtual") && name.ends_with("keyboard")
        })
        .ok_or_else(|| std::io::Error::other("no real keyboard found"))?;
    Ok(Snapshot {
        reference: reference["name"].as_str().unwrap_or("").to_string(),
        reference_index: index(reference),
        watched_index: kbds.iter().find(|k| k["name"] == watched).map(index),
    })
}

/// Read the current layout, and bring `watched` in line with it.
fn sync(watched: &str) -> std::io::Result<Snapshot> {
    let snap = snapshot(watched)?;
    if snap.watched_index.is_some_and(|i| i != snap.reference_index) {
        switch_layout(watched, snap.reference_index)?;
    }
    Ok(snap)
}

#[derive(Default)]
struct State {
    /// Bumped on every activelayout event of the watched (virtual) device.
    seq: u64,
    index: usize,
}

pub struct LayoutWatcher {
    state: Arc<(Mutex<State>, Condvar)>,
}

impl LayoutWatcher {
    /// `watched` is our virtual keyboard: it is kept on the same layout as the
    /// real keyboards, and `wait_event` waits for its layout events.
    pub fn start(watched: &'static str) -> std::io::Result<Self> {
        let snap = sync(watched)?;
        let reference = snap.reference;
        let state = Arc::new((
            Mutex::new(State { index: snap.reference_index, ..Default::default() }),
            Condvar::new(),
        ));
        let stream = UnixStream::connect(socket_dir()?.join(".socket2.sock"))?;
        let st = state.clone();
        thread::spawn(move || {
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { return };
                let Some(rest) = line.strip_prefix("activelayout>>") else { continue };
                let device = rest.split(',').next().unwrap_or("");
                let (m, cv) = &*st;
                if device == watched {
                    m.lock().unwrap().seq += 1;
                    cv.notify_all();
                } else if device == reference {
                    // Layout names aren't indices; re-query to get the index.
                    match sync(watched) {
                        Ok(snap) => m.lock().unwrap().index = snap.reference_index,
                        Err(e) => eprintln!("layout sync failed: {e}"),
                    }
                }
            }
        });
        Ok(Self { state })
    }

    pub fn current_index(&self) -> usize {
        self.state.0.lock().unwrap().index
    }

    pub fn seq(&self) -> u64 {
        self.state.0.lock().unwrap().seq
    }

    /// Wait until a layout event for the watched device arrives after `since`.
    pub fn wait_event(&self, since: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (m, cv) = &*self.state;
        let mut s = m.lock().unwrap();
        loop {
            if s.seq > since {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            s = cv.wait_timeout(s, deadline - now).unwrap().0;
        }
    }
}
