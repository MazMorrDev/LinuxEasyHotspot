# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-11-12

A larger update: clearer device management, usage at a glance, a session log, USB Wi-Fi adapters, a Bridge mode that keeps this computer on the local network, and the end of the repeated authorization banner.

**Upgrading:** the helper changed (API level 3) and must be updated together with the app. Installing the new `.deb` does it; from source, use **Update now** on the banner or run `./scripts/install-helper.sh`. Until then, per-device control is paused and the app asks for the update.

### Added
- Home: a *Session activity* log under the QR code and buttons while the hotspot runs: start and stop (with the data used), devices connecting and disconnecting, approval requests, devices allowed, blocked or forgotten, data caps reached, Wi-Fi adapters plugged in or out, errors and authorization changes. It starts fresh with each session.
- Clients: the hotspot's current speed and the data used since it started (all devices together, read from the Wi-Fi interface's counters, no helper needed).
- Clients: every known device, connected or not, listed under *Allowed* and *Blocked* (and *Waiting for approval*), each opening its settings. Works while the hotspot is off. `devices.toml` now remembers each device's name and when it was last connected.
- Wi-Fi adapters plugged in or out while the app runs are picked up (NetworkManager `DeviceAdded`/`DeviceRemoved`), with a notice.
- Settings explains adapters that can't run a hotspot: no access-point mode in the driver, not managed by NetworkManager, unavailable, and USB Wi-Fi sticks with no driver or still in "storage mode" (with the `usb-modeswitch` fix). Also shown on the "can't create hotspots" page.
- Hotspot adapter *Automatic* prefers a free USB adapter; adapters are shown as "USB adapter (wlx…)" / "Built-in Wi-Fi (wlp…)" with their bands and product name. Home shows when the hotspot runs on a USB adapter.

### Changed
- The app is now called "Linux Easy Hotspot" (window title, About, tray, menus, desktop entry, NetworkManager profiles). Profiles named "LinuxEasyHotspot…" are still recognized and renamed on the next start.
- Helper API level 3; the app asks to update an older helper.
- Clients: long device names are shortened with "…" instead of wrapping letter by letter, and every device (also without the helper) opens its settings.
- The Bridge mode dialog explains what happens to the wired connection and how to keep sharing files (e.g. with Warpinator).
- `scripts/build-deb.sh` checks for the GTK 4 and libadwaita development packages before building, instead of failing at the link step.
- Spanish translation updated for all new and renamed strings.

### Fixed
- The "Allow Linux Easy Hotspot to manage speed limits" banner kept coming back. The helper answered `AccessDenied` (which the app reads as "not authorized") whenever its check of the hotspot interface hit a transient D-Bus error, e.g. another connection disappearing mid-scan, or when polkit couldn't be reached. Only polkit refusals are `AccessDenied` now; the check skips vanished connections and retries; the app retries temporary failures quietly and a refusal once before showing the banner. The helper's watchdog also no longer removes all rules after a single failed check.
- Bridge mode cut this computer off the local network: the bridge got a new MAC, so the router gave it a different IP, and static IP/DNS settings of the wired connection were dropped. The bridge now takes over the Ethernet card's MAC and the wired profile's IP settings, and STP is off (it blocked all traffic for several seconds after each start).

## [0.1.1] - 2026-11-10

### Fixed
- Renaming a device: the name field was reset to the current name every few seconds while typing.
- Bridge mode: connected devices showed as "Unknown device". The app now finds their IP (one-off ARP probe of the subnet), asks the upstream router's DNS for their name, and gets the name the device itself sends in its DHCP request from the helper (new `ClientNames` method, helper API level 2). In Router mode the helper also reads NM's root-only lease file.

## [0.1.0] - 2026-11-10

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
