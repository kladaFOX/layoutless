#!/usr/bin/env bash
# Build the layoutless daemon from this checkout and install it for the current user.
#
#   ./install.sh               install or update
#   ./install.sh --uninstall   remove the daemon (keeps your settings and learned words)
#   --yes                      don't ask before using sudo
#
# User files: ~/.local/bin/layoutless, ~/.config/systemd/user/layoutless.service.
# System files (sudo): /etc/udev/rules.d/60-layoutless.rules, /etc/modules-load.d/layoutless.conf,
# and your user added to the `input` group.

set -euo pipefail
cd "$(dirname "$0")"

BIN="$HOME/.local/bin/layoutless"
UNIT="$HOME/.config/systemd/user/layoutless.service"
UDEV_RULE=/etc/udev/rules.d/60-layoutless.rules
MODULES_CONF=/etc/modules-load.d/layoutless.conf

yes=false
action=do_install
for arg in "$@"; do
  case "$arg" in
    --yes | -y) yes=true ;;
    --uninstall) action=do_uninstall ;;
    -h | --help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

confirm() {
  $yes && return 0
  read -r -p "$1 [y/N] " answer
  [[ $answer == [yY]* ]]
}

in_input_group() { id -nG "$USER" | tr ' ' '\n' | grep -qx input; }

do_install() {
  command -v hyprctl >/dev/null || die "Hyprland (hyprctl) not found"
  command -v cargo >/dev/null || die "Rust is needed to build: omarchy pkg add rustup && rustup default stable"

  say "Building the daemon"
  cargo build --release --locked --manifest-path daemon/Cargo.toml

  say "Installing $BIN and the systemd user service"
  install -Dm755 daemon/target/release/layoutless "$BIN"
  sed "s|/usr/bin/layoutless|%h/.local/bin/layoutless|" dist/layoutless.service | install -Dm644 /dev/stdin "$UNIT"

  echo
  echo "layoutless needs to read your keyboards and type through /dev/uinput. With sudo this will:"
  echo "  - install $UDEV_RULE and $MODULES_CONF"
  echo "  - load the uinput module now"
  in_input_group || echo "  - add $USER to the 'input' group (takes effect after you log out and back in)"
  confirm "Continue?" || die "stopped; nothing was changed outside your home directory"

  sudo install -Dm644 dist/60-layoutless.rules "$UDEV_RULE"
  sudo install -Dm644 dist/modules-load.conf "$MODULES_CONF"
  sudo modprobe uinput
  sudo udevadm control --reload
  sudo udevadm trigger --subsystem-match=misc --sysname-match=uinput

  local relogin=false
  if ! in_input_group; then
    sudo usermod -aG input "$USER"
    relogin=true
  fi

  systemctl --user daemon-reload
  systemctl --user enable layoutless >/dev/null
  # This login session only gets the input group after a re-login.
  if id -nG | tr ' ' '\n' | grep -qx input; then
    systemctl --user restart layoutless
    say "layoutless is running"
  else
    relogin=true
  fi

  $relogin && say "Log out and back in: the daemon starts by itself afterwards"
  if command -v omarchy >/dev/null; then
    say "Bar widget: omarchy plugin add https://github.com/kladaFOX/layoutless --enable"
  fi
}

do_uninstall() {
  say "Stopping and removing the daemon"
  systemctl --user disable --now layoutless 2>/dev/null || true
  rm -f "$BIN" "$UNIT"
  systemctl --user daemon-reload

  if [[ -e $UDEV_RULE || -e $MODULES_CONF ]]; then
    confirm "Remove $UDEV_RULE and $MODULES_CONF with sudo?" && sudo rm -f "$UDEV_RULE" "$MODULES_CONF"
  fi

  say "Kept: ~/.config/layoutless (settings) and ~/.local/share/layoutless (learned words)"
  say "Bar widget, if installed: omarchy plugin remove kladafox.layoutless"
  say "Leave the input group, if nothing else needs it: sudo gpasswd -d $USER input"
}

"$action"
