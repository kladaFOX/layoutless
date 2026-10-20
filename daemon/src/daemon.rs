//! The daemon: tracks the word being typed, auto-corrects words typed in the
//! wrong layout, and retypes the last word on `flip`.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use evdev::KeyCode;

use crate::detect::{Detector, Params, Verdict};
use crate::keymap::{self, KeyClass, Lang, Stroke};
use crate::output::{Output, VIRTUAL_HYPR_NAME};
use crate::config::{self, Config};
use crate::control::Subscribers;
use crate::rules::{self, Rules};
use crate::{control, hypr, input};

pub enum Msg {
    Key(KeyCode, i32),
    /// Mouse button press: the caret probably moved.
    Click,
    /// Another window got focus; its class.
    FocusChanged(String),
    /// A layer-shell surface (shell popup) opened (true) or closed; its namespace.
    Layer(String, bool),
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
/// A pending correction gives up after this long or this many keys typed
/// after the word without a pause.
const MAX_PENDING: Duration = Duration::from_secs(2);
const MAX_TAIL: usize = 64;
/// How often a pending correction checks the held keys.
const POLL: Duration = Duration::from_millis(10);

pub fn run() -> std::io::Result<()> {
    let debug = std::env::var_os("LAYOUTLESS_DEBUG").is_some();
    let config = config::load();
    let auto = match std::env::var("LAYOUTLESS_AUTO").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => config.auto,
    };
    let (tx, rx) = mpsc::channel::<Msg>();

    // Bind first: it fails if another daemon is already running.
    let events = Subscribers::default();
    control::serve(tx.clone(), events.clone())?;
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
        app: hypr::active_class().unwrap_or_default(),
        open_layers: Vec::new(),
        config,
        rules: Rules::load(),
        events,
        last_action: None,
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
                Msg::Click => core.reset(),
                Msg::FocusChanged(class) => {
                    core.app = class;
                    core.reset();
                }
                Msg::Layer(ns, open) => {
                    if open {
                        core.open_layers.push(ns);
                    } else if let Some(i) = core.open_layers.iter().position(|l| *l == ns) {
                        core.open_layers.remove(i);
                    }
                }
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
    /// An auto-correction waiting until no key is held. Whatever is typed
    /// meanwhile (`tail`: letters, spaces, more words) was typed in the same
    /// wrong layout, so it's erased and retyped after the word too.
    Correct { word: Vec<Stroke>, boundary: Stroke, target: Lang, tail: Vec<Stroke>, started: Instant },
}

impl Pending {
    fn deadline(&self) -> Instant {
        match self {
            Pending::Flip { deadline, .. } => *deadline,
            Pending::Correct { .. } => Instant::now() + POLL,
        }
    }
}

struct Action {
    word: Vec<Stroke>,
    /// An auto-correction (vs a manual flip).
    auto: bool,
    /// The layout the word was retyped in.
    to: Lang,
    /// A manual flip that counted as a learning vote.
    voted: bool,
}

struct Core {
    out: Output,
    layouts: hypr::LayoutWatcher,
    detector: Detector,
    auto: bool,
    config: Config,
    /// Class of the focused window.
    app: String,
    /// Namespaces of the open layer-shell surfaces.
    open_layers: Vec<String>,
    rules: Rules,
    /// Event stream for the bar widget.
    events: Subscribers,
    /// The last retype, while nothing was typed after it: tells an undo or
    /// a toggle from a fresh flip, for learning.
    last_action: Option<Action>,
    /// How long an auto-correction waits for the keys to be released.
    correct_wait: Duration,
    mods: Mods,
    /// Non-modifier keys physically held right now, with press time.
    held: Vec<(KeyCode, Instant)>,
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
        self.last_action = None;
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
            Some(Pending::Correct { word, boundary, tail, .. }) => {
                if self.debug {
                    eprintln!("  correction cancelled: the text changed");
                }
                self.replay_state(word, boundary, &tail);
            }
            None => {}
        }
    }

    /// Set the word state as if `word` + `boundary` + `tail` had been typed
    /// with no auto-correction (none of these words is evaluated again).
    fn replay_state(&mut self, word: Vec<Stroke>, boundary: Stroke, tail: &[Stroke]) {
        self.clear_word();
        self.last = Some((word, boundary));
        for &s in tail {
            match keymap::classify(s) {
                KeyClass::Word | KeyClass::Taint => {
                    self.word.push(s);
                    self.tainted |= keymap::classify(s) == KeyClass::Taint;
                    self.last = None;
                }
                KeyClass::Boundary => {
                    self.last = (!self.word.is_empty()).then(|| (std::mem::take(&mut self.word), s));
                    self.tainted = false;
                }
                KeyClass::Reset => {
                    self.clear_word();
                    self.last = None;
                }
            }
        }
    }

    /// Give up on a pending correction whose text can't be trusted any more
    /// (a key held long enough for the app to repeat it).
    fn drop_pending(&mut self, why: &str) {
        if self.debug {
            eprintln!("  correction dropped: {why}");
        }
        self.pending = None;
        self.clear_word();
        self.last = None;
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
            "state" => self.state().to_string(),
            "rules list" => {
                let list: Vec<_> = self.rules.list().iter().map(rules::describe).collect();
                serde_json::json!({ "rules": list }).to_string()
            }
            c if c.starts_with("rules ") => self.rules_command(&c["rules ".len()..]),
            "status" => format!(
                "auto {}, layout {}, app {:?}{}",
                if self.auto { "on" } else { "off" },
                self.layouts.current_index(),
                self.app,
                if self.excluded() { " (excluded)" } else { "" },
            ),
            "reload" => {
                self.config = config::load();
                self.rules = Rules::load();
                self.events.send(&self.state());
                format!("reloaded: {} apps, {} layers excluded", self.config.exclude_apps.len(), self.config.exclude_layers.len())
            }
            other => format!("unknown command: {other}"),
        };
        let _ = reply.send(answer);
    }

    /// Auto-correction is off in excluded apps and while an excluded popup
    /// (the polkit password prompt) is open.
    fn excluded(&self) -> bool {
        self.config.exclude_apps.iter().any(|a| a.eq_ignore_ascii_case(&self.app))
            || self.open_layers.iter().any(|l| self.config.exclude_layers.contains(l))
    }

    fn set_auto(&mut self, on: bool) -> String {
        self.auto = on;
        self.events.send(&self.state());
        format!("auto {}", if on { "on" } else { "off" })
    }

    fn state(&self) -> serde_json::Value {
        serde_json::json!({ "event": "state", "auto": self.auto, "rules": self.rules.list().len() })
    }

    /// `add WORD`, `rm WORD`, `set KEYS en|ru`. A word given as text belongs
    /// to the layout of its script: `add next` keeps `next` (and turns
    /// `туче` into it), `add туче` does the opposite.
    fn rules_command(&mut self, args: &str) -> String {
        let parts: Vec<&str> = args.split_whitespace().collect();
        let keys_of = |word: &str, lang: Lang| {
            keymap::strokes_for_word(word, lang).and_then(|w| keymap::key_string(&w))
        };
        let changed = match parts.as_slice() {
            ["add", word] => {
                let lang = if word.chars().any(|c| c.is_ascii_alphabetic()) { Lang::En } else { Lang::Ru };
                match keys_of(word, lang) {
                    Some(keys) => self.rules.set(&keys, lang).keys,
                    None => return format!("error: {word:?} is not a word on the {lang:?} layout"),
                }
            }
            ["rm", word] => {
                let keys = [Lang::En, Lang::Ru].into_iter().filter_map(|l| keys_of(word, l)).find(|k| self.rules.remove(k));
                match keys {
                    Some(k) => k,
                    None => return format!("error: no rule for {word:?}"),
                }
            }
            ["set", keys, lang] => {
                let lang = match *lang {
                    "en" => Lang::En,
                    "ru" => Lang::Ru,
                    _ => return "error: layout must be en or ru".into(),
                };
                if keymap::from_key_string(keys).is_none() {
                    return format!("error: bad keys {keys:?}");
                }
                self.rules.set(keys, lang).keys
            }
            _ => return "usage: rules list | add WORD | rm WORD | set KEYS en|ru".into(),
        };
        self.events.send(&self.state());
        format!("ok {changed}")
    }

    /// A manual flip of `word` to `target`: count it as a learning vote,
    /// unless it just toggles back the previous manual flip.
    fn learn_from_flip(&mut self, word: Vec<Stroke>, target: Lang) {
        let prev = self
            .last_action
            .take()
            .filter(|a| keymap::key_string(&a.word) == keymap::key_string(&word));
        let voted = match prev {
            Some(a) if !a.auto => {
                if a.voted {
                    self.rules.unvote(&word, a.to);
                }
                false
            }
            // A fresh flip, or the undo of an auto-correction: either way the
            // word belongs to `target`. Never learn from excluded apps.
            _ if self.excluded() => false,
            _ => {
                if let Some(rule) = self.rules.vote(&word, target) {
                    let mut event = rules::describe(&rule);
                    if self.config.notify {
                        let (en, ru) = (event["en"].as_str().unwrap_or(""), event["ru"].as_str().unwrap_or(""));
                        let (from, to) = if rule.lang == Lang::En { (ru, en) } else { (en, ru) };
                        notify(&format!("Learned: {from} → {to}"));
                    }
                    event["event"] = "learned".into();
                    self.events.send(&event);
                    self.events.send(&self.state());
                }
                true
            }
        };
        self.last_action = Some(Action { word, auto: false, to: target, voted });
    }

    fn key(&mut self, key: KeyCode, value: i32) {
        if self.mods.update(key, value) {
            return;
        }
        if value == 0 {
            self.held.retain(|(k, _)| *k != key);
            return;
        }
        if !self.held.iter().any(|(k, _)| *k == key) {
            self.held.push((key, Instant::now()));
        }
        // Alt+Space is the flip hotkey. Hyprland consumes it, so the app
        // never sees it and the word state stays as it was.
        if key == KeyCode::KEY_SPACE && self.mods.alt_only() {
            return;
        }
        self.last_action = None;

        let stroke = Stroke { key, shift: self.mods.shift() };
        // Shortcuts (Ctrl+C, Super+1, ...) only clear the buffer. Letters typed
        // after the modifiers are released start a fresh word.
        let class = if self.mods.chord() { KeyClass::Reset } else { keymap::classify(stroke) };

        // A pending correction absorbs typing (letters, boundaries, Backspace
        // within what was typed after the word). Anything else means the
        // caret moved or the text was sent: cancel it.
        let absorbed = match &mut self.pending {
            Some(Pending::Correct { tail, .. }) if key == KeyCode::KEY_BACKSPACE => tail.pop().is_some(),
            Some(Pending::Correct { tail, .. }) if class != KeyClass::Reset && tail.len() < MAX_TAIL => {
                tail.push(stroke);
                true
            }
            _ => false,
        };
        if absorbed {
            return;
        }
        self.cancel_pending();

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

        if self.auto && !tainted && !self.excluded() {
            let cur = Lang::from_index(self.layouts.current_index());
            // A learned rule beats the detector.
            let verdict = match self.rules.lookup(&word) {
                Some(lang) if lang == cur => Verdict::Keep,
                Some(_) => Verdict::Flip,
                None => self.detector.decide(&word, cur),
            };
            if self.debug {
                let (en, ru) = keymap::render(&word);
                eprintln!("  {en} / {ru} typed in {cur:?}: {verdict:?}");
            }
            // Never type on the lock screen. If the state is unknown, don't either.
            if verdict == Verdict::Flip && !hypr::is_locked().unwrap_or(true) {
                let (target, tail, started) = (cur.other(), Vec::new(), Instant::now());
                self.pending = Some(Pending::Correct { word, boundary, target, tail, started });
                self.last = None;
                return;
            }
        }
        self.last = Some((word, boundary));
    }

    fn run_pending(&mut self) {
        match &self.pending {
            None => {}
            Some(Pending::Flip { deadline, .. }) => {
                let released = self.held.is_empty() && !self.mods.any();
                if released || Instant::now() >= *deadline {
                    let Some(Pending::Flip { reply, .. }) = self.pending.take() else { unreachable!() };
                    let answer = self.flip().unwrap_or_else(|e| format!("error: {e}"));
                    if self.debug {
                        eprintln!("  flip: {answer}");
                    }
                    let _ = reply.send(answer);
                }
            }
            Some(Pending::Correct { started, .. }) => {
                let now = Instant::now();
                if let Some((k, _)) = self.held.iter().find(|(_, t)| now - *t >= self.correct_wait) {
                    // The app is repeating it by now, and the daemon can't see that.
                    let why = format!("{k:?} held past the repeat delay");
                    self.drop_pending(&why);
                } else if now - *started >= MAX_PENDING {
                    self.drop_pending("no pause in typing");
                } else if self.held.is_empty() {
                    let Some(Pending::Correct { word, boundary, target, tail, .. }) = self.pending.take() else {
                        unreachable!()
                    };
                    let keys = [&word[..], &[boundary], &tail].concat();
                    match self.retype(keys.len(), &keys, target) {
                        Ok(answer) if self.debug => eprintln!("  corrected: {answer}, tail {}", tail.len()),
                        Ok(_) => {}
                        Err(e) => eprintln!("correction failed: {e}"),
                    }
                    // Alt+Space right after a correction flips the word back.
                    if tail.is_empty() {
                        self.last_action = Some(Action { word: word.clone(), auto: true, to: target, voted: false });
                    }
                    self.replay_state(word, boundary, &tail);
                }
            }
        }
    }

    /// Retype the current or last word in the other layout. The word state
    /// is kept, so a second flip toggles back.
    fn flip(&mut self) -> std::io::Result<String> {
        if hypr::is_locked().unwrap_or(true) {
            return Ok("screen is locked".into());
        }
        let (word, keys) = if !self.word.is_empty() {
            (self.word.clone(), self.word.clone())
        } else if let Some((w, b)) = &self.last {
            (w.clone(), [&w[..], &[*b]].concat())
        } else {
            // Don't switch the layout either: Alt+Space is not a layout switcher.
            return Ok("nothing to flip".into());
        };
        let target = Lang::from_index(self.layouts.current_index()).other();
        let answer = self.retype(keys.len(), &keys, target)?;
        self.learn_from_flip(word, target);
        Ok(answer)
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
            Msg::FocusChanged(class) => eprintln!("[dbg {t:9.1}] focus → {class:?}, reset"),
            Msg::Layer(ns, open) => eprintln!("[dbg {t:9.1}] layer {ns} {}", if *open { "open" } else { "closed" }),
            Msg::Command(c, _) => eprintln!("[dbg {t:9.1}] command {c:?}"),
            Msg::HyprGone => eprintln!("[dbg {t:9.1}] hyprland gone"),
        }
    }
}

/// Show a desktop notification without blocking the key path.
fn notify(body: &str) {
    let body = body.to_string();
    std::thread::spawn(move || {
        let status = std::process::Command::new("notify-send")
            .args(["--app-name=layoutless", "--expire-time=3000", "layoutless", &body])
            .status();
        if let Err(e) = status {
            eprintln!("notify-send: {e}");
        }
    });
}
