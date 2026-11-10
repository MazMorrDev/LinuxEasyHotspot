#!/bin/bash
# Removes the leh-shaper helper installed by install-helper.sh.
set -euo pipefail
ID=io.github.OWNER.LinuxEasyHotspot
sudo pkill -x leh-shaper 2>/dev/null || true
sudo rm -f -- /usr/libexec/linuxeasyhotspot/leh-shaper \
  "/usr/share/dbus-1/system.d/$ID.Shaper.conf" \
  "/usr/share/dbus-1/system-services/$ID.Shaper.service" \
  "/usr/share/polkit-1/actions/$ID.policy"
sudo rmdir --ignore-fail-on-non-empty /usr/libexec/linuxeasyhotspot 2>/dev/null || true
sudo systemctl reload dbus 2>/dev/null || true
echo "Helper removed."
