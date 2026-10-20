# layoutless

Automatic keyboard layout correction for [Omarchy](https://omarchy.org/) / Hyprland, in the spirit of Punto Switcher.

Type without thinking about the layout. When a word comes out in the wrong one (`ghbdtn` → `привет`, `руддщ` → `hello`), layoutless retypes it in the right layout and switches the layout for you.

> Status: early development. Manual flip with `Alt+Space` works. Auto-correction works but is off by default (`layoutless auto on`) until the lock-screen and password-field guards are in.

## Features

- **Auto-correct the last word.** When you finish a word with Space or Shift+Enter, layoutless checks whether it reads better in the other layout. If it does, it erases the word, retypes it, and leaves the corrected layout active so you can keep typing.
- **Force flip with `Alt+Space`.** If layoutless guessed wrong or missed a word, press `Alt+Space` to flip the last word. Right after an auto-correction, `Alt+Space` undoes it instead.
- **Learns your words.** If you flip the same word by hand twice, layoutless remembers which layout it belongs to. For example, `next` typed in the Russian layout comes out as `туче`, which is a valid Russian word, so a dictionary alone would never flip it. After two manual flips it always does. Undoing an auto-correction twice teaches it to leave that word alone.
- **Works everywhere you type.** It works in browsers, terminals, Electron and JetBrains apps, because it uses only Backspace and replays the same physical keys after switching the layout.
- **Omarchy bar widget.** The widget shows the current layout and whether auto mode is on. Click it to pause auto mode, and open its panel to manage learned words and excluded apps.

Supported layouts: `us` + `ru` for now. The design keeps layout pairs pluggable.

## How it works

```
physical keyboard ──evdev (read-only)──▶ layoutless daemon ──uinput──▶ Backspace × N, replay keys
                                            │  ▲
                Hyprland IPC (socket2) ─────┘  └── `layoutless flip` (Alt+Space bind)
                activelayout / activewindow       `layoutless ctl …` (bar widget)
```

1. The daemon passively reads key events from your physical keyboards and keeps the physical keys of the word being typed. It never grabs the keyboard, so typing is never delayed.
2. On a word boundary it scores the word under both layouts. It checks your learned rules first, then dictionaries, then a letter n-gram model.
3. To correct a word it sends Backspace once per key, switches the layout with `hyprctl switchxkblayout`, and replays the same physical keys. Because the keys are the same, no character mapping is needed on output.

## Privacy and permissions

layoutless has to see what you type, so its rules are strict:

- It keeps only the current and previous word, in memory only. It never logs or sends keystrokes anywhere. The exception is `LAYOUTLESS_DEBUG=1`, a debugging mode that prints every key to stderr: never leave it on.
- Learned rules (`~/.local/share/layoutless/rules.json`) store only words you flipped by hand. The file is plain JSON, so you can read and edit it.
- Reading keyboards requires membership in the `input` group, and typing requires write access to `/dev/uinput`. Any program running as your user with these permissions can read every keystroke. That's the cost of a global layout corrector on Wayland.
- Corrections are off on the lock screen, in password fields (detected via AT-SPI), and in excluded apps (password managers by default). Password fields switch to `us`, like macOS.

## Install

_Not released yet._ The planned install has two parts:

```bash
yay -S layoutless                                        # daemon, systemd user unit, udev rule
omarchy plugin add https://github.com/kladaFOX/layoutless  # bar widget
```

### From source (for now)

You need Rust and these permissions (then re-login):

```bash
sudo usermod -aG input "$USER"
echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' | sudo tee /etc/udev/rules.d/60-layoutless.rules
echo uinput | sudo tee /etc/modules-load.d/layoutless.conf
```

```bash
cargo build --release --manifest-path daemon/Cargo.toml
install -Dm755 daemon/target/release/layoutless ~/.local/bin/layoutless
install -Dm644 dist/layoutless.service ~/.config/systemd/user/layoutless.service
systemctl --user daemon-reload
systemctl --user enable --now layoutless
```

Then bind the flip key in `~/.config/hypr/bindings.lua`:

```lua
o.bind("ALT + SPACE", "Layoutless: flip last word", "layoutless flip")
```

## License

MIT for the code. The bundled word lists in `daemon/data/` are derived from
[hermitdave/FrequencyWords](https://github.com/hermitdave/FrequencyWords) (OpenSubtitles 2018)
and are licensed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/);
see [daemon/data/README.md](daemon/data/README.md).
