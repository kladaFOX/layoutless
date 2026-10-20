//! M0 spike: read keyboards via evdev, print each finished word as both
//! layouts, and auto-correct the hard-coded word `ghbdtn` via uinput replay.

mod hypr;
mod keymap;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventSummary, EventType, InputEvent, KeyCode};

const VIRTUAL_NAME: &str = "layoutless virtual keyboard";
/// How Hyprland names our uinput device in IPC events.
const VIRTUAL_HYPR_NAME: &str = "layoutless-virtual-keyboard";

/// One keystroke of the word buffer: physical key plus Shift state.
#[derive(Clone, Copy, Debug)]
struct Stroke {
    key: KeyCode,
    shift: bool,
}

enum Msg {
    Key(KeyCode, i32),
    /// Mouse button press: the caret probably moved.
    Click,
}

#[derive(Default)]
struct Mods {
    shift_l: bool,
    shift_r: bool,
    ctrl: u8,
    alt: u8,
    meta: u8,
}

impl Mods {
    fn shift(&self) -> bool {
        self.shift_l || self.shift_r
    }
    fn chord(&self) -> bool {
        self.ctrl > 0 || self.alt > 0 || self.meta > 0
    }
    /// Returns true if the key is a modifier (state updated).
    fn update(&mut self, key: KeyCode, value: i32) -> bool {
        let down = value != 0;
        let set = |c: &mut u8| *c = if down { 1 } else { 0 };
        match key {
            KeyCode::KEY_LEFTSHIFT => self.shift_l = down,
            KeyCode::KEY_RIGHTSHIFT => self.shift_r = down,
            KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => set(&mut self.ctrl),
            KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => set(&mut self.alt),
            KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => set(&mut self.meta),
            _ => return false,
        }
        true
    }
}

fn main() -> std::io::Result<()> {
    let mut out = create_virtual_keyboard()?;
    let (tx, rx) = mpsc::channel::<Msg>();

    for (path, dev) in evdev::enumerate() {
        let name = dev.name().unwrap_or("").to_string();
        if name == VIRTUAL_NAME {
            continue;
        }
        let Some(keys) = dev.supported_keys() else { continue };
        let is_kbd = keys.contains(KeyCode::KEY_A) && keys.contains(KeyCode::KEY_SPACE);
        let is_mouse = keys.contains(KeyCode::BTN_LEFT);
        if !is_kbd && !is_mouse {
            continue;
        }
        eprintln!(
            "reading {} ({name}){}",
            path.display(),
            if is_kbd { "" } else { " [mouse]" }
        );
        spawn_reader(dev, tx.clone());
    }
    drop(tx);

    let layouts = hypr::LayoutWatcher::start(VIRTUAL_HYPR_NAME)?;
    eprintln!("ready. type `ghbdtn ` to test the correction; Ctrl+C to quit");

    let mut mods = Mods::default();
    let mut word: Vec<Stroke> = Vec::new();
    // Non-modifier keys physically held right now.
    let mut held: Vec<KeyCode> = Vec::new();
    // A finished word waiting for its keys to be released before correction.
    let mut pending: Option<Pending> = None;
    let debug = std::env::var_os("LAYOUTLESS_DEBUG").is_some();
    let t_start = Instant::now();

    loop {
        let msg = match &pending {
            Some(p) => {
                let wait = p.deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(m) => Some(m),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            },
        };

        if let Some(msg) = msg {
            if debug {
                let t = ms(t_start.elapsed());
                match &msg {
                    Msg::Click => eprintln!("[dbg {t:9.1}] click → reset"),
                    Msg::Key(k, v) => eprintln!(
                        "[dbg {t:9.1}] {k:?} {} word_len={} held={held:?}",
                        ["up", "down", "repeat"][(*v).clamp(0, 2) as usize],
                        word.len(),
                    ),
                }
            }
            let continued = handle(msg, &mut mods, &mut word, &mut held, &layouts, &mut pending);
            if !continued {
                if let Some(p) = pending.take() {
                    eprintln!("  correction of {:?} aborted: typing went on", keymap::render(&p.word).0);
                }
            }
        }

        let ready = pending
            .as_ref()
            .is_some_and(|p| held.is_empty() || Instant::now() >= p.deadline);
        if ready {
            let p = pending.take().unwrap();
            if !held.is_empty() {
                eprintln!("  still held at deadline: {held:?}");
            }
            // Letters typed after the boundary are the start of the next word.
            if let Err(e) = correct(&p, &word, &mut out, &layouts) {
                eprintln!("correction failed: {e}");
            }
        }
    }
    Ok(())
}

struct Pending {
    word: Vec<Stroke>,
    boundary: KeyCode,
    target: usize,
    deadline: Instant,
}

/// How long a correction may wait for the word's keys to be released.
/// Replaying a key the user still holds makes apps drop it as a duplicate press.
const RELEASE_WAIT: Duration = Duration::from_millis(200);

/// Process one input event. Returns false if a pending correction must be
/// dropped because the user typed something other than letters after it.
fn handle(
    msg: Msg,
    mods: &mut Mods,
    word: &mut Vec<Stroke>,
    held: &mut Vec<KeyCode>,
    layouts: &hypr::LayoutWatcher,
    pending: &mut Option<Pending>,
) -> bool {
    let (key, value) = match msg {
        Msg::Click => {
            word.clear();
            return false;
        }
        Msg::Key(k, v) => (k, v),
    };
    if mods.update(key, value) {
        return true;
    }
    match value {
        0 => {
            held.retain(|k| *k != key);
            return true;
        }
        1 if !held.contains(&key) => held.push(key),
        _ => {}
    }
    // Shortcuts (Ctrl+C, Super+1, ...) only clear the buffer. Letters typed
    // after the modifiers are released start a fresh word.
    if mods.chord() {
        word.clear();
        return false;
    }

    if keymap::is_word_key(key) {
        word.push(Stroke { key, shift: mods.shift() });
        return true;
    }
    if pending.is_some() {
        return false;
    }

    match key {
        KeyCode::KEY_BACKSPACE => {
            word.pop();
        }
        KeyCode::KEY_SPACE => {
            *pending = on_boundary(word, key, layouts);
            word.clear();
        }
        KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER if mods.shift() => {
            *pending = on_boundary(word, key, layouts);
            word.clear();
        }
        KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER | KeyCode::KEY_TAB => {
            if !word.is_empty() {
                let (en, ru) = keymap::render(word);
                println!("{:<20} → en: {en:<16} ru: {ru}   (reset, no correction)", keys_str(word));
            }
            word.clear();
        }
        _ => {
            // Navigation, Esc, F-keys, Shift+Arrow selection, ...: caret moved.
            word.clear();
        }
    }
    true
}

fn keys_str(word: &[Stroke]) -> String {
    word.iter()
        .map(|s| {
            let n = format!("{:?}", s.key);
            let n = n.trim_start_matches("KEY_").to_lowercase();
            if s.shift { n.to_uppercase() } else { n }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Log the finished word and decide whether it needs a correction.
fn on_boundary(word: &[Stroke], boundary: KeyCode, layouts: &hypr::LayoutWatcher) -> Option<Pending> {
    if word.is_empty() {
        return None;
    }
    let (en, ru) = keymap::render(word);
    println!("{:<20} → en: {en:<16} ru: {ru}", keys_str(word));

    if en.to_lowercase() != "ghbdtn" {
        return None;
    }
    let cur = layouts.current_index();
    if cur != 0 {
        // Typed in ru already: it's literally "ghbdtn"-keys but showing "привет".
        eprintln!("  skip: layout is already {cur}");
        return None;
    }
    Some(Pending {
        word: word.to_vec(),
        boundary,
        target: 1,
        deadline: Instant::now() + RELEASE_WAIT,
    })
}

/// Erase the word, its boundary char and the letters typed after it, switch
/// layout, replay all of it.
fn correct(
    p: &Pending,
    tail: &[Stroke],
    out: &mut VirtualDevice,
    layouts: &hypr::LayoutWatcher,
) -> std::io::Result<()> {
    let t0 = Instant::now();

    for _ in 0..p.word.len() + 1 + tail.len() {
        tap(out, KeyCode::KEY_BACKSPACE, false)?;
    }
    let t_erase = t0.elapsed();

    let seq = layouts.seq();
    hypr::switch_layout("all", p.target)?;
    let t_switch = t0.elapsed();
    let confirmed = layouts.wait_event(seq, Duration::from_millis(100));
    let t_wait = t0.elapsed();

    for s in &p.word {
        tap(out, s.key, s.shift)?;
    }
    tap(out, p.boundary, p.boundary != KeyCode::KEY_SPACE)?;
    for s in tail {
        tap(out, s.key, s.shift)?;
    }
    let t_done = t0.elapsed();

    eprintln!(
        "  corrected in {:.1} ms (erase {:.1}, switch {:.1}, wait {:.1}{}, replay {:.1}), tail {}",
        ms(t_done),
        ms(t_erase),
        ms(t_switch - t_erase),
        ms(t_wait - t_switch),
        if confirmed { "" } else { " TIMEOUT" },
        ms(t_done - t_wait),
        tail.len(),
    );
    Ok(())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn key_event(key: KeyCode, value: i32) -> InputEvent {
    InputEvent::new(EventType::KEY.0, key.0, value)
}

fn tap(out: &mut VirtualDevice, key: KeyCode, shift: bool) -> std::io::Result<()> {
    if shift {
        out.emit(&[key_event(KeyCode::KEY_LEFTSHIFT, 1)])?;
    }
    out.emit(&[key_event(key, 1)])?;
    out.emit(&[key_event(key, 0)])?;
    if shift {
        out.emit(&[key_event(KeyCode::KEY_LEFTSHIFT, 0)])?;
    }
    Ok(())
}

fn create_virtual_keyboard() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for k in keymap::WORD_KEYS {
        keys.insert(*k);
    }
    for k in [
        KeyCode::KEY_BACKSPACE,
        KeyCode::KEY_SPACE,
        KeyCode::KEY_ENTER,
        KeyCode::KEY_LEFTSHIFT,
    ] {
        keys.insert(k);
    }
    let dev = VirtualDevice::builder()?.name(VIRTUAL_NAME).with_keys(&keys)?.build()?;
    // Give the compositor time to pick up the new device.
    thread::sleep(Duration::from_millis(300));
    Ok(dev)
}

fn spawn_reader(mut dev: Device, tx: mpsc::Sender<Msg>) {
    let name = dev.name().unwrap_or("").to_string();
    thread::spawn(move || loop {
        let events = match dev.fetch_events() {
            Ok(ev) => ev,
            Err(e) => {
                eprintln!("device {name} gone: {e}");
                return;
            }
        };
        for ev in events {
            if let EventSummary::Key(_, key, value) = ev.destructure() {
                let msg = match key {
                    KeyCode::BTN_LEFT | KeyCode::BTN_RIGHT | KeyCode::BTN_MIDDLE if value == 1 => Msg::Click,
                    KeyCode::BTN_LEFT | KeyCode::BTN_RIGHT | KeyCode::BTN_MIDDLE => continue,
                    _ => Msg::Key(key, value),
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }
    });
}
