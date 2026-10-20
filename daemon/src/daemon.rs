//! The daemon: tracks the word being typed, auto-corrects words typed in the
//! wrong layout, and retypes the last word on `flip`.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use evdev::KeyCode;

use crate::detect::{Detector, Params, Verdict};
use crate::keymap::{self, KeyClass, Lang, Stroke};
use crate::output::{Output, VIRTUAL_HYPR_NAME};
use crate::{control, hypr, input};

pub enum Msg {
    Key(KeyCode, i32),
    /// Mouse button press: the caret probably moved.
    Click,
    /// Another window got focus.
    FocusChanged,
    /// A control socket command; the reply goes back to the client.
    Command(String, mpsc::Sender<String>),
    /// Hyprland's event socket closed (compositor exited or restarted).
    HyprGone,
}

/// How long a flip may wait for the hotkey to be released. Replaying while
/// keys are physically held mixes their state into the replay.
const FLIP_RELEASE_WAIT: Duration = Duration::from_millis(500);
/// Safety margin before Hyprland's repeat delay: an auto-correction waits for
/// the keys to be released until `repeat_delay - margin`. Replaying a key the
/// user still holds makes apps drop it as a duplicate press, and a key held
/// past the repeat delay has already been repeated by the app (which the
/// daemon can't see), so then the correction is dropped.
const REPEAT_MARGIN: Duration = Duration::from_millis(30);

pub fn run() -> std::io::Result<()> {
    let debug = std::env::var_os("LAYOUTLESS_DEBUG").is_some();
    let auto = std::env::var("LAYOUTLESS_AUTO").is_ok_and(|v| v == "1");
    let (tx, rx) = mpsc::channel::<Msg>();

    // Bind first: it fails if another daemon is already running.
    control::serve(tx.clone())?;
    let detector = Detector::new(Params::default())?;
    let out = Output::new()?;
    input::watch(tx.clone());
    let layouts = hypr::LayoutWatcher::start(VIRTUAL_HYPR_NAME, tx)?;
    let correct_wait = hypr::repeat_delay()
        .unwrap_or(Duration::from_millis(250))
        .saturating_sub(REPEAT_MARGIN)
        .max(Duration::from_millis(50));
    eprintln!("ready, auto mode {}", if auto { "on" } else { "off" });

    let mut core = Core {
        out,
        layouts,
        detector,
        auto,
        correct_wait,
        mods: Mods::default(),
        held: Vec::new(),
        word: Vec::new(),
        tainted: false,
        last: None,
        pending: None,
        debug,
    };
    let t_start = Instant::now();

    loop {
        let msg = match &core.pending {
            Some(p) => match rx.recv_timeout(p.deadline().saturating_duration_since(Instant::now())) {
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
                Msg::Command(cmd, reply) => core.command(&cmd, reply),
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

enum Pending {
    /// `layoutless flip`, waiting for the hotkey to be released.
    Flip { reply: mpsc::Sender<String>, deadline: Instant },
    /// An auto-correction waiting for the word's keys to be released. Letters
    /// typed meanwhile go to `Core::word` and are retyped after the boundary.
    Correct { word: Vec<Stroke>, boundary: Stroke, target: Lang, deadline: Instant },
}

impl Pending {
    fn deadline(&self) -> Instant {
        match self {
            Pending::Flip { deadline, .. } | Pending::Correct { deadline, .. } => *deadline,
        }
    }
}

struct Core {
    out: Output,
    layouts: hypr::LayoutWatcher,
    detector: Detector,
    auto: bool,
    /// How long an auto-correction waits for the keys to be released.
    correct_wait: Duration,
    mods: Mods,
    /// Non-modifier keys physically held right now.
    held: Vec<KeyCode>,
    /// The word being typed.
    word: Vec<Stroke>,
    /// The word contains digits, `-` or `=`: never auto-correct it.
    tainted: bool,
    /// The word finished just before, with its boundary key, as long as
    /// nothing was typed after the boundary.
    last: Option<(Vec<Stroke>, Stroke)>,
    pending: Option<Pending>,
    debug: bool,
}

impl Core {
    fn reset(&mut self) {
        self.clear_word();
        self.last = None;
        self.cancel_pending();
    }

    fn clear_word(&mut self) {
        self.word.clear();
        self.tainted = false;
    }

    fn cancel_pending(&mut self) {
        match self.pending.take() {
            Some(Pending::Flip { reply, .. }) => {
                let _ = reply.send("cancelled".into());
            }
            Some(Pending::Correct { .. }) if self.debug => eprintln!("  correction cancelled: typing went on"),
            _ => {}
        }
    }

    fn command(&mut self, cmd: &str, reply: mpsc::Sender<String>) {
        let answer = match cmd {
            "flip" => {
                self.cancel_pending();
                let deadline = Instant::now() + FLIP_RELEASE_WAIT;
                self.pending = Some(Pending::Flip { reply, deadline });
                return;
            }
            "auto on" => self.set_auto(true),
            "auto off" => self.set_auto(false),
            "auto toggle" => self.set_auto(!self.auto),
            "status" => format!(
                "auto {}, layout {}",
                if self.auto { "on" } else { "off" },
                self.layouts.current_index()
            ),
            other => format!("unknown command: {other}"),
        };
        let _ = reply.send(answer);
    }

    fn set_auto(&mut self, on: bool) -> String {
        self.auto = on;
        format!("auto {}", if on { "on" } else { "off" })
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

        let stroke = Stroke { key, shift: self.mods.shift() };
        // Shortcuts (Ctrl+C, Super+1, ...) only clear the buffer. Letters typed
        // after the modifiers are released start a fresh word.
        let class = if self.mods.chord() { KeyClass::Reset } else { keymap::classify(stroke) };

        // A pending correction tolerates letters (the start of the next word);
        // anything else means the text changed under it.
        let tolerated = matches!(self.pending, Some(Pending::Correct { .. })) && class == KeyClass::Word;
        if !tolerated {
            self.cancel_pending();
        }

        if key == KeyCode::KEY_BACKSPACE {
            if self.word.is_empty() {
                // Erasing the boundary puts the caret back at the end of
                // the previous word: it's the word being typed again.
                if let Some((w, _)) = self.last.take() {
                    self.word = w;
                }
            } else {
                self.word.pop();
            }
            return;
        }

        match class {
            KeyClass::Word | KeyClass::Taint => {
                self.word.push(stroke);
                self.tainted |= class == KeyClass::Taint;
                self.last = None;
            }
            KeyClass::Boundary => self.finish_word(stroke),
            // Plain Enter and Tab may have sent the message or completed
            // something; navigation, Esc, F-keys moved the caret.
            KeyClass::Reset => {
                self.clear_word();
                self.last = None;
            }
        }
    }

    fn finish_word(&mut self, boundary: Stroke) {
        if self.word.is_empty() {
            // A second boundary in a row leaves nothing flippable: `last`
            // would have to erase two boundary chars.
            self.last = None;
            return;
        }
        let word = std::mem::take(&mut self.word);
        let tainted = std::mem::take(&mut self.tainted);

        if self.auto && !tainted {
            let cur = Lang::from_index(self.layouts.current_index());
            let verdict = self.detector.decide(&word, cur);
            if self.debug {
                let (en, ru) = keymap::render(&word);
                eprintln!("  {en} / {ru} typed in {cur:?}: {verdict:?}");
            }
            if verdict == Verdict::Flip {
                let deadline = Instant::now() + self.correct_wait;
                self.pending = Some(Pending::Correct { word, boundary, target: cur.other(), deadline });
                self.last = None;
                return;
            }
        }
        self.last = Some((word, boundary));
    }

    fn run_pending(&mut self) {
        let Some(p) = &self.pending else { return };
        let released = match p {
            Pending::Flip { .. } => self.held.is_empty() && !self.mods.any(),
            Pending::Correct { .. } => self.held.is_empty(),
        };
        if !released && Instant::now() < p.deadline() {
            return;
        }
        if !released && matches!(p, Pending::Correct { .. }) {
            if self.debug {
                eprintln!("  correction dropped: {:?} held past the repeat delay", self.held);
            }
            self.pending = None;
            return;
        }
        match self.pending.take().unwrap() {
            Pending::Flip { reply, .. } => {
                let answer = self.flip().unwrap_or_else(|e| format!("error: {e}"));
                if self.debug {
                    eprintln!("  flip: {answer}");
                }
                let _ = reply.send(answer);
            }
            Pending::Correct { word, boundary, target, .. } => {
                let tail = self.word.clone();
                match self.retype(word.len() + 1 + tail.len(), &[&word[..], &[boundary], &tail].concat(), target) {
                    Ok(answer) if self.debug => eprintln!("  corrected: {answer}, tail {}", tail.len()),
                    Ok(_) => {}
                    Err(e) => eprintln!("correction failed: {e}"),
                }
                // Alt+Space right after a correction flips the word back.
                self.last = if tail.is_empty() { Some((word, boundary)) } else { None };
            }
        }
    }

    /// Retype the current or last word in the other layout. The word state
    /// is kept, so a second flip toggles back.
    fn flip(&mut self) -> std::io::Result<String> {
        let keys = if !self.word.is_empty() {
            self.word.clone()
        } else if let Some((w, b)) = &self.last {
            [&w[..], &[*b]].concat()
        } else {
            // Don't switch the layout either: Alt+Space is not a layout switcher.
            return Ok("nothing to flip".into());
        };
        let target = Lang::from_index(self.layouts.current_index()).other();
        self.retype(keys.len(), &keys, target)
    }

    /// Erase `erase` chars, switch to `target`, type `keys`.
    fn retype(&mut self, erase: usize, keys: &[Stroke], target: Lang) -> std::io::Result<String> {
        let t0 = Instant::now();
        self.out.erase(erase)?;
        let seq = self.layouts.seq();
        hypr::switch_layout("all", target.index())?;
        let confirmed = self.layouts.wait_event(seq, Duration::from_millis(100));
        self.out.replay(keys)?;
        Ok(format!(
            "retyped to {target:?} in {:.1} ms{}",
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
            Msg::Command(c, _) => eprintln!("[dbg {t:9.1}] command {c:?}"),
            Msg::HyprGone => eprintln!("[dbg {t:9.1}] hyprland gone"),
        }
    }
}
