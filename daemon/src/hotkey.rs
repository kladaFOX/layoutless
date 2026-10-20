//! The flip hotkey. `flip_key` in the config uses Hyprland's syntax
//! (`"ALT + SPACE"`). The daemon binds it in Hyprland at runtime (so the app
//! never gets the keystroke) and recognizes it in the evdev stream (so the
//! keystroke doesn't reset the word it's about to flip).

use evdev::KeyCode;

use crate::hypr;
use crate::keymap::{self, Lang};

pub const DEFAULT: &str = "ALT + SPACE";

#[derive(Clone, Debug)]
pub struct Hotkey {
    /// As written in the config; passed to Hyprland verbatim.
    pub spec: String,
    pub key: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Hotkey {
    pub fn parse(spec: &str) -> Result<Self, String> {
        // Only what a key spec needs: it is pasted into Lua code.
        if !spec.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '+' | '_')) {
            return Err(format!("flip_key {spec:?}: unexpected characters"));
        }
        let mut hk = Hotkey { spec: spec.to_string(), key: KeyCode::KEY_RESERVED, ctrl: false, alt: false, shift: false, meta: false };
        let mut key = None;
        for part in spec.split('+').map(|p| p.trim().to_ascii_uppercase()) {
            match part.as_str() {
                "CTRL" | "CONTROL" => hk.ctrl = true,
                "ALT" => hk.alt = true,
                "SHIFT" => hk.shift = true,
                "SUPER" | "META" | "WIN" => hk.meta = true,
                name if key.is_none() => key = Some(key_code(name).ok_or(format!("flip_key {spec:?}: unknown key {name}"))?),
                _ => return Err(format!("flip_key {spec:?}: more than one non-modifier key")),
            }
        }
        hk.key = key.ok_or(format!("flip_key {spec:?}: no key"))?;
        Ok(hk)
    }

    /// Bind it in Hyprland to `layoutless flip`, replacing whatever the
    /// combination did before.
    pub fn bind(&self) -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        let cmd = format!("{} flip", exe.display());
        let lua = format!(
            "hl.unbind(\"{spec}\"); hl.bind(\"{spec}\", hl.dsp.exec_cmd({cmd:?}), {{ description = \"Layoutless: flip last word\" }})",
            spec = self.spec,
        );
        hypr::eval(&lua)
    }

    pub fn unbind(&self) -> std::io::Result<()> {
        hypr::eval(&format!("hl.unbind(\"{}\")", self.spec))
    }
}

/// evdev key for a Hyprland key name: letters, digits, F1–F12 and a few
/// names that make sense for a hotkey.
fn key_code(name: &str) -> Option<KeyCode> {
    use KeyCode as K;
    let named = match name {
        "SPACE" => Some(K::KEY_SPACE),
        "PAUSE" => Some(K::KEY_PAUSE),
        "SCROLL_LOCK" => Some(K::KEY_SCROLLLOCK),
        "INSERT" => Some(K::KEY_INSERT),
        "PRINT" => Some(K::KEY_SYSRQ),
        "GRAVE" => Some(K::KEY_GRAVE),
        "MENU" => Some(K::KEY_COMPOSE),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    const F: [KeyCode; 12] = [
        KeyCode::KEY_F1, KeyCode::KEY_F2, KeyCode::KEY_F3, KeyCode::KEY_F4, KeyCode::KEY_F5, KeyCode::KEY_F6,
        KeyCode::KEY_F7, KeyCode::KEY_F8, KeyCode::KEY_F9, KeyCode::KEY_F10, KeyCode::KEY_F11, KeyCode::KEY_F12,
    ];
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<usize>().ok()) {
        return F.get(n.checked_sub(1)?).copied();
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => keymap::stroke_for(c.to_ascii_lowercase(), Lang::En).map(|s| s.key),
        _ => None,
    }
}
