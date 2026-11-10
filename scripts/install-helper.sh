#!/bin/bash
# Installs the leh-shaper helper (per-device speed limits) system-wide.
# Usage: ./scripts/install-helper.sh   (asks for sudo)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CARGO=${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}

echo "Building leh-shaper (release)..."
"$CARGO" build --release -p leh-shaper --manifest-path "$ROOT/Cargo.toml"

ID=io.github.OWNER.LinuxEasyHotspot
echo "Installing (sudo)..."
sudo install -Dm755 "$ROOT/target/release/leh-shaper" /usr/libexec/linuxeasyhotspot/leh-shaper
sudo install -Dm644 "$ROOT/data/dbus/$ID.Shaper.conf" "/usr/share/dbus-1/system.d/$ID.Shaper.conf"
sudo install -Dm644 "$ROOT/data/dbus/$ID.Shaper.service" "/usr/share/dbus-1/system-services/$ID.Shaper.service"
sudo install -Dm644 "$ROOT/data/polkit/$ID.policy" "/usr/share/polkit-1/actions/$ID.policy"
# Make the system bus pick up the new policy and service file.
sudo systemctl reload dbus 2>/dev/null || sudo busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig || true
echo "Done. In LinuxEasyHotspot, press 'Check again' (or restart the app)."
