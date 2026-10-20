//! Read keyboards and mice via evdev (read-only, never grabbed), with hot-plug.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use evdev::{Device, EventSummary, KeyCode};

use crate::daemon::Msg;
use crate::output::VIRTUAL_NAME;

/// How often /dev/input is rescanned for new devices (wireless keyboards
/// reconnect, udev may set permissions a moment after the node appears).
const RESCAN: Duration = Duration::from_secs(1);

/// Open every relevant input device now and whenever a new one appears.
pub fn watch(tx: Sender<Msg>) {
    let open: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    // Devices that are neither keyboards nor mice; forgotten once the node is gone.
    let mut ignored: HashSet<PathBuf> = HashSet::new();
    scan(&tx, &open, &mut ignored);
    thread::spawn(move || loop {
        thread::sleep(RESCAN);
        scan(&tx, &open, &mut ignored);
    });
}

fn scan(tx: &Sender<Msg>, open: &Arc<Mutex<HashSet<PathBuf>>>, ignored: &mut HashSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir("/dev/input") else { return };
    let present: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event")))
        .collect();
    ignored.retain(|p| present.contains(p));

    for path in present {
        if ignored.contains(&path) || open.lock().unwrap().contains(&path) {
            continue;
        }
        // Permission errors are retried on the next scan.
        let Ok(dev) = Device::open(&path) else { continue };
        let Some(kind) = classify(&dev) else {
            ignored.insert(path);
            continue;
        };
        eprintln!("reading {} ({}){kind}", path.display(), dev.name().unwrap_or(""));
        open.lock().unwrap().insert(path.clone());
        spawn_reader(path, dev, tx.clone(), open.clone());
    }
}

/// `Some("")` for a keyboard, `Some(" [mouse]")` for a mouse, `None` otherwise.
fn classify(dev: &Device) -> Option<&'static str> {
    if dev.name() == Some(VIRTUAL_NAME) {
        return None; // our own output
    }
    let keys = dev.supported_keys()?;
    if keys.contains(KeyCode::KEY_A) && keys.contains(KeyCode::KEY_SPACE) {
        Some("")
    } else if keys.contains(KeyCode::BTN_LEFT) {
        Some(" [mouse]")
    } else {
        None
    }
}

fn spawn_reader(path: PathBuf, mut dev: Device, tx: Sender<Msg>, open: Arc<Mutex<HashSet<PathBuf>>>) {
    thread::spawn(move || {
        if let Err(e) = read_events(&mut dev, &tx) {
            eprintln!("{} gone: {e}", path.display());
        }
        open.lock().unwrap().remove(Path::new(&path));
    });
}

fn read_events(dev: &mut Device, tx: &Sender<Msg>) -> std::io::Result<()> {
    loop {
        for ev in dev.fetch_events()? {
            let EventSummary::Key(_, key, value) = ev.destructure() else { continue };
            let msg = match key {
                KeyCode::BTN_LEFT | KeyCode::BTN_RIGHT | KeyCode::BTN_MIDDLE if value == 1 => Msg::Click,
                KeyCode::BTN_LEFT | KeyCode::BTN_RIGHT | KeyCode::BTN_MIDDLE => continue,
                _ => Msg::Key(key, value),
            };
            if tx.send(msg).is_err() {
                return Ok(());
            }
        }
    }
}
