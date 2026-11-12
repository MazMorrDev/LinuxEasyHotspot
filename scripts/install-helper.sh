#!/bin/bash
# Installs the leh-shaper helper (per-device speed limits) system-wide.
#
#   ./scripts/install-helper.sh              build, then install with sudo
#   pkexec install-helper.sh --as-root BIN   install a prebuilt binary (used by the app's "Install now")
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
ID=io.github.OWNER.LinuxEasyHotspot

fail() { echo "✖ $*" >&2; exit 1; }

install_files() { # <binary> ; must run as root
  local bin=$1
  [ -x "$bin" ] || fail "helper binary not found: $bin"
  install -Dm755 "$bin" /usr/libexec/linuxeasyhotspot/leh-shaper
  install -Dm644 "$ROOT/data/dbus/$ID.Shaper.conf" "/usr/share/dbus-1/system.d/$ID.Shaper.conf"
  install -Dm644 "$ROOT/data/dbus/$ID.Shaper.service" "/usr/share/dbus-1/system-services/$ID.Shaper.service"
  install -Dm644 "$ROOT/data/polkit/$ID.policy" "/usr/share/polkit-1/actions/$ID.policy"
  # Stop an older running copy, then make the system bus re-read its config.
  pkill -x leh-shaper 2>/dev/null || true
  systemctl reload dbus 2>/dev/null \
    || busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 \
    || true
}

verify() {
  for _ in 1 2 3 4 5; do
    if busctl get-property "$ID.Shaper" /io/github/OWNER/LinuxEasyHotspot/Shaper \
        "$ID.Shaper1" ApiLevel >/dev/null 2>&1; then
      echo "✔ Helper installed and responding. Linux Easy Hotspot will pick it up within a few seconds."
      return 0
    fi
    sleep 1
  done
  fail "files were installed, but the helper doesn't answer on the system bus. Try logging out and back in, or run: busctl get-property $ID.Shaper /io/github/OWNER/LinuxEasyHotspot/Shaper $ID.Shaper1 ApiLevel"
}

if [ "${1:-}" = "--as-root" ]; then
  [ "$(id -u)" = 0 ] || fail "--as-root must run as root (use pkexec)"
  install_files "${2:?binary path}"
  verify
  exit 0
fi

# sudo can't work from shells started with the "no new privileges" flag
# (for example some IDE/AI-assistant terminals and sandboxes).
if grep -q '^NoNewPrivs:[[:space:]]*1' /proc/self/status 2>/dev/null; then
  fail "this terminal can't use sudo (\"no new privileges\" is set). Open a normal terminal and run:
    cd '$ROOT' && ./scripts/install-helper.sh
or use the \"Install now\" button in the app."
fi

CARGO=${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}
[ -x "$CARGO" ] || fail "cargo not found (install Rust from https://rustup.rs)"
echo "Building leh-shaper (release)…"
"$CARGO" build --release -p leh-shaper --manifest-path "$ROOT/Cargo.toml"

echo "Installing (sudo will ask for your password)…"
sudo ROOT="$ROOT" bash -c "$(declare -f install_files fail); ID=$ID; install_files '$ROOT/target/release/leh-shaper'"
verify
