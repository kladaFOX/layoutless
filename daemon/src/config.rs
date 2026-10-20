//! `~/.config/layoutless/config.toml`.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Auto-correction on start. `layoutless auto on|off` changes it at runtime.
    pub auto: bool,
    /// A desktop notification when a word is learned.
    pub notify: bool,
    /// How many manual flips (or undos of an auto-correction) of the same
    /// word make it a rule. 1 learns on the first flip.
    pub learn_threshold: u32,
    /// Flip hotkey in Hyprland's syntax (`"ALT + SPACE"`, `"CTRL + SHIFT + Q"`,
    /// `"PAUSE"`). The daemon binds it in Hyprland itself.
    pub flip_key: String,
    /// Window classes (as in `hyprctl activewindow`) where words are never
    /// auto-corrected. Case-insensitive.
    pub exclude_apps: Vec<String>,
    /// Layer-shell namespaces (shell popups) that block auto-correction while
    /// open, such as the polkit password prompt.
    pub exclude_layers: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto: true,
            notify: true,
            learn_threshold: 2,
            flip_key: crate::hotkey::DEFAULT.into(),
            exclude_apps: [
                "org.keepassxc.KeePassXC",
                "1Password",
                "Bitwarden",
                "gcr-prompter",
                "pinentry-qt",
                "pinentry-gtk",
            ]
            .map(String::from)
            .to_vec(),
            exclude_layers: vec!["omarchy-polkit".into()],
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("layoutless").join("config.toml"))
}

/// Read the config. A missing file gives the defaults; a broken one is
/// reported and also gives the defaults, so a typo never stops the daemon.
pub fn load() -> Config {
    let Some(path) = path() else { return Config::default() };
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("{}: {e}; using defaults", path.display());
            Config::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => {
            eprintln!("{}: {e}; using defaults", path.display());
            Config::default()
        }
    }
}
