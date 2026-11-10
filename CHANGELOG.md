# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
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
