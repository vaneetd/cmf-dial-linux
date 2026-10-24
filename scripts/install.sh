#!/usr/bin/env bash
# Builds Dial for Linux and installs it for the current user.
#
#   ./scripts/install.sh           build, install the binary, the udev rule and the autostart entry
#   ./scripts/install.sh --remove  undo all of that
#
# The udev rule needs sudo (it is the only system-wide change). Everything else lives in $HOME.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$HOME/.local/bin/dial-daemon"
autostart="$HOME/.config/autostart/dial-daemon.desktop"
rule=/etc/udev/rules.d/70-dial.rules

if [[ "${1:-}" == "--remove" ]]; then
    rm -f "$bin" "$autostart"
    sudo rm -f "$rule"
    sudo udevadm control --reload
    echo "Removed. A running dial-daemon keeps running until you stop it (pkill dial-daemon)."
    exit 0
fi

export PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null || { echo "cargo not found. Install Rust first: https://rustup.rs"; exit 1; }

echo "==> Building (release)"
cargo build --release --manifest-path "$root/daemon/Cargo.toml"

echo "==> Installing the binary to $bin"
install -Dm755 "$root/daemon/target/release/dial-daemon" "$bin"

echo "==> Installing the autostart entry"
install -Dm644 "$root/packaging/dial-daemon.desktop" "$autostart"

echo "==> Installing the udev rule (sudo)"
sudo install -Dm644 "$root/packaging/70-dial.rules" "$rule"
sudo udevadm control --reload
sudo udevadm trigger --subsystem-match=input --subsystem-match=misc

cat <<MSG

Done. Dial starts at your next login. To start it now:

    nohup dial-daemon >/dev/null 2>&1 &

Make sure ~/.local/bin is on your PATH. Close the case lid; the dial only talks with the lid closed.
MSG
