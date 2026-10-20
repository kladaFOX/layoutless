//! uinput virtual keyboard that retypes words.

use std::thread;
use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};

use crate::keymap::{self, Stroke};

pub const VIRTUAL_NAME: &str = "layoutless virtual keyboard";
/// How Hyprland names our uinput device in IPC events.
pub const VIRTUAL_HYPR_NAME: &str = "layoutless-virtual-keyboard";

pub struct Output {
    dev: VirtualDevice,
}

impl Output {
    pub fn new() -> std::io::Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for k in keymap::WORD_KEYS {
            keys.insert(*k);
        }
        for k in [
            KeyCode::KEY_BACKSPACE,
            KeyCode::KEY_SPACE,
            KeyCode::KEY_ENTER,
            KeyCode::KEY_KPENTER,
            KeyCode::KEY_LEFTSHIFT,
        ] {
            keys.insert(k);
        }
        let dev = VirtualDevice::builder()?.name(VIRTUAL_NAME).with_keys(&keys)?.build()?;
        // Give the compositor time to pick up the new device.
        thread::sleep(Duration::from_millis(300));
        Ok(Self { dev })
    }

    pub fn erase(&mut self, n: usize) -> std::io::Result<()> {
        for _ in 0..n {
            self.tap(Stroke { key: KeyCode::KEY_BACKSPACE, shift: false })?;
        }
        Ok(())
    }

    pub fn replay(&mut self, strokes: &[Stroke]) -> std::io::Result<()> {
        for s in strokes {
            self.tap(*s)?;
        }
        Ok(())
    }

    fn tap(&mut self, s: Stroke) -> std::io::Result<()> {
        let ev = |key: KeyCode, value| InputEvent::new(EventType::KEY.0, key.0, value);
        if s.shift {
            self.dev.emit(&[ev(KeyCode::KEY_LEFTSHIFT, 1)])?;
        }
        self.dev.emit(&[ev(s.key, 1)])?;
        self.dev.emit(&[ev(s.key, 0)])?;
        if s.shift {
            self.dev.emit(&[ev(KeyCode::KEY_LEFTSHIFT, 0)])?;
        }
        Ok(())
    }
}
