# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed
- Bridge mode: connected devices showed as "Unknown device". The app now finds their IP (one-off ARP probe of the subnet), asks the upstream router's DNS for their name, and gets the name the device itself sends in its DHCP request from the helper (new `ClientNames` method, helper API level 2). In Router mode the helper also reads NM's root-only lease file.

### Added
- `.deb` package (`scripts/build-deb.sh`) with the app, the helper, D-Bus/polkit files, icons and the desktop entry. Depends on GTK 4, libadwaita, NetworkManager, iproute2 and polkit.
- Typed speed limits and data caps: a text field plus a unit picker (kbps, Mbps, Gbps, KB/s, MB/s; MB, GB, TB) instead of spin buttons. Units can also be typed ("2,5 MB/s"); empty means no limit.
- Startup options: launch at login (XDG autostart, `--hidden`), start the hotspot at login, and a tray icon with start/stop, approve/block for waiting devices, show window and quit. Closing the window hides it to the tray.
- Spanish translation and a language setting (System / English / Español). Strings live in `po/*.po`; `scripts/update-po.py` refreshes them.
- The helper removes its rules on SIGTERM (e.g. when the package is removed).
- Bandwidth management:
  - the `leh-shaper` root helper (system D-Bus, polkit-checked, limited to the hotspot interface),
  - `leh-model::tc` (validated HTB + IFB + flower rules),
  - per-device limits, priority, approval and data caps, with live speeds and usage,
  - total hotspot speed,
  - saved device settings in `devices.toml`,
  - `scripts/install-helper.sh`.
- The internet source selector lists every connection (Ethernet, Wi-Fi, mobile) by name and state. Bridge mode offers only wired ones. In Router mode, the chosen source is connected when the hotspot starts.
- The hotspot password is hidden on Home while running, with a show/hide button.
- Mode and source drop-downs show short names; the explanations are tooltips and subtitles.
- App icon (full color and symbolic) and a `.desktop` file.
- `leh-model`: config with versioning, typed NM settings for Router, Bridge and Local-only modes, the hotspot state machine, Wi-Fi QR payload, password generator, SSID sanitizer, dnsmasq lease parser, DNS PTR codec and MAC vendor lookup.
- `leh-model::policy`: per-device policies (limits, priority, approval, data cap) and defaults for new devices, with `resolve()`.
- GTK4/libadwaita app (`--demo`): Home, Settings, Clients, a device dialog, status pages, the helper banner, adaptive narrow and wide layouts, the QR code, shortcuts and an About dialog.
- `scripts/screenshots.sh`: headless screenshots of the demo UI.
- `leh-nm`: NetworkManager D-Bus backend: device discovery, a controller actor (start/stop, adopting an already-running hotspot, bridge rollback) and connected-client discovery.
