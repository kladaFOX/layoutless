//! The daemon: tracks the word being typed and retypes it on `flip`.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use evdev::KeyCode;

use crate::keymap::{self, Stroke};
use crate::output::{Output, VIRTUAL_HYPR_NAME};
use crate::{control, hypr, input};

pub enum Msg {
    Key(KeyCode, i32),
    /// Mouse button press: the caret probably moved.
    Click,
    /// Another window got focus.
    FocusChanged,
    /// `layoutless flip`; the reply goes back to the client.
    Flip(mpsc::Sender<String>),
    /// Hyprland's event socket closed (compositor exited or restarted).
    HyprGone,
}

/// How long a flip may wait for the hotkey to be released. Replaying while
/// keys are physically held mixes their state into the replay.
const RELEASE_WAIT: Duration = Duration::from_millis(500);

pub fn run() -> std::io::Result<()> {
    let debug = std::env::var_os("LAYOUTLESS_DEBUG").is_some();
    let (tx, rx) = mpsc::channel::<Msg>();

    // Bind first: it fails if another daemon is already running.
    control::serve(tx.clone())?;
    let out = Output::new()?;
    input::watch(tx.clone());
    let layouts = hypr::LayoutWatcher::start(VIRTUAL_HYPR_NAME, tx)?;
    eprintln!("ready");

    let mut core = Core {
        out,
        layouts,
        mods: Mods::default(),
        held: Vec::new(),
        word: Vec::new(),
        last: None,
        pending: None,
        debug,
    };
    let t_start = Instant::now();

    loop {
        let msg = match &core.pending {
            Some(p) => match rx.recv_timeout(p.deadline.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            },
        };
        if let Some(msg) = msg {
            if debug {
                core.debug_log(&msg, t_start.elapsed());
            }
            match msg {
                Msg::Key(key, value) => core.key(key, value),
                Msg::Click | Msg::FocusChanged => core.reset(),
                Msg::Flip(reply) => core.request_flip(reply),
                Msg::HyprGone => return Err(std::io::Error::other("Hyprland event socket closed")),
            }
        }
        core.run_pending();
    }
    Ok(())
}

#[derive(Default)]
struct Mods {
    shift_l: bool,
    shift_r: bool,
    ctrl: bool,
    alt: bool,
    meta: bool,
}

impl Mods {
    fn shift(&self) -> bool {
        self.shift_l || self.shift_r
    }

    fn chord(&self) -> bool {
        self.ctrl || self.alt || self.meta
    }

    fn any(&self) -> bool {
        self.shift() || self.chord()
    }

    fn alt_only(&self) -> bool {
        self.alt && !self.ctrl && !self.meta && !self.shift()
    }

    /// Returns true if the key is a modifier (state updated).
    fn update(&mut self, key: KeyCode, value: i32) -> bool {
        let down = value != 0;
        match key {
            KeyCode::KEY_LEFTSHIFT => self.shift_l = down,
            KeyCode::KEY_RIGHTSHIFT => self.shift_r = down,
            KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => self.ctrl = down,
            KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => self.alt = down,
            KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => self.meta = down,
            _ => return false,
        }
        true
    }
}

struct PendingFlip {
    reply: mpsc::Sender<String>,
    deadline: Instant,
}

struct Core {
    out: Output,
    layouts: hypr::LayoutWatcher,
    mods: Mods,
    /// Non-modifier keys physically held right now.
    held: Vec<KeyCode>,
    /// The word being typed.
    word: Vec<Stroke>,
    /// The word finished just before, with its boundary key, as long as
    /// nothing was typed after the boundary.
    last: Option<(Vec<Stroke>, Stroke)>,
    pending: Option<PendingFlip>,
    debug: bool,
}

impl Core {
    fn reset(&mut self) {
        self.word.clear();
        self.last = None;
        self.cancel_pending();
    }

    fn cancel_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            let _ = p.reply.send("cancelled".into());
        }
    }

    fn key(&mut self, key: KeyCode, value: i32) {
        if self.mods.update(key, value) {
            return;
        }
        if value == 0 {
            self.held.retain(|k| *k != key);
            return;
        }
        if !self.held.contains(&key) {
            self.held.push(key);
        }
        // Alt+Space is the flip hotkey. Hyprland consumes it, so the app
        // never sees it and the word state stays as it was.
        if key == KeyCode::KEY_SPACE && self.mods.alt_only() {
            return;
        }
        self.cancel_pending();

        // Shortcuts (Ctrl+C, Super+1, ...) only clear the buffer. Letters typed
        // after the modifiers are released start a fresh word.
        if self.mods.chord() {
            self.word.clear();
            self.last = None;
            return;
        }
        if keymap::is_word_key(key) {
            self.word.push(Stroke { key, shift: self.mods.shift() });
            self.last = None;
            return;
        }

        let boundary = Stroke { key, shift: self.mods.shift() };
        match key {
            KeyCode::KEY_BACKSPACE => {
                if self.word.is_empty() {
                    // Erasing the boundary puts the caret back at the end of
                    // the previous word: it's the word being typed again.
                    if let Some((w, _)) = self.last.take() {
                        self.word = w;
                    }
                } else {
                    self.word.pop();
                }
            }
            KeyCode::KEY_SPACE => self.finish_word(boundary),
            KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER if self.mods.shift() => self.finish_word(boundary),
            // Plain Enter and Tab may have sent the message or completed
            // something: never touch what came before.
            _ => {
                // Navigation, Esc, F-keys, Shift+Arrow selection, ...: caret moved.
                self.word.clear();
                self.last = None;
            }
        }
    }

    fn finish_word(&mut self, boundary: Stroke) {
        // A second space in a row leaves nothing flippable: `last` would
        // have to erase two boundary chars.
        self.last = if self.word.is_empty() {
            None
        } else {
            Some((std::mem::take(&mut self.word), boundary))
        };
    }

    fn request_flip(&mut self, reply: mpsc::Sender<String>) {
        self.cancel_pending();
        self.pending = Some(PendingFlip { reply, deadline: Instant::now() + RELEASE_WAIT });
    }

    fn run_pending(&mut self) {
        let Some(p) = &self.pending else { return };
        let released = self.held.is_empty() && !self.mods.any();
        if !released && Instant::now() < p.deadline {
            return;
        }
        let p = self.pending.take().unwrap();
        let reply = match self.flip() {
            Ok(r) => r,
            Err(e) => format!("error: {e}"),
        };
        if self.debug {
            eprintln!("  flip: {reply}");
        }
        let _ = p.reply.send(reply);
    }

    /// Retype the current or last word in the other layout. The word state
    /// is kept, so a second flip toggles back.
    fn flip(&mut self) -> std::io::Result<String> {
        let t0 = Instant::now();
        let (erase, retype, what): (usize, Vec<Stroke>, &str) = if !self.word.is_empty() {
            (self.word.len(), self.word.clone(), "word")
        } else if let Some((w, b)) = &self.last {
            let mut keys = w.clone();
            keys.push(*b);
            (keys.len(), keys, "last word")
        } else {
            // Don't switch the layout either: Alt+Space is not a layout switcher.
            return Ok("nothing to flip".into());
        };
        let target = if self.layouts.current_index() == 0 { 1 } else { 0 };

        self.out.erase(erase)?;
        let seq = self.layouts.seq();
        hypr::switch_layout("all", target)?;
        let confirmed = self.layouts.wait_event(seq, Duration::from_millis(100));
        self.out.replay(&retype)?;

        Ok(format!(
            "flipped {what} to layout {target} in {:.1} ms{}",
            t0.elapsed().as_secs_f64() * 1000.0,
            if confirmed { "" } else { " (layout event timeout)" },
        ))
    }

    fn debug_log(&self, msg: &Msg, t: Duration) {
        let t = t.as_secs_f64() * 1000.0;
        match msg {
            Msg::Key(k, v) => eprintln!(
                "[dbg {t:9.1}] {k:?} {} word={:?} last={:?}",
                ["up", "down", "repeat"][(*v).clamp(0, 2) as usize],
                keymap::render(&self.word),
                self.last.as_ref().map(|(w, _)| keymap::render(w).0),
            ),
            Msg::Click => eprintln!("[dbg {t:9.1}] click → reset"),
            Msg::FocusChanged => eprintln!("[dbg {t:9.1}] focus changed → reset"),
            Msg::Flip(_) => eprintln!("[dbg {t:9.1}] flip requested"),
            Msg::HyprGone => eprintln!("[dbg {t:9.1}] hyprland gone"),
        }
    }
}
