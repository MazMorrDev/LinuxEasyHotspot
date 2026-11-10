# LinuxEasyHotspot

The simplest way to turn your Linux computer into a Wi-Fi hotspot. Press one button, scan a QR code with your phone, and you're connected.

> **Status:** early development (0.1.0). Router mode, the GTK interface, per-device control, startup options and the `.deb` package work on real hardware; Bridge and Local-only still need field testing.

## Features

- One-click start/stop, with a QR code so phones can join by scanning.
- An auto-generated, easy-to-read password and a name based on your computer's hostname.
- **Router** mode: devices share this computer's internet (NAT).
- **Bridge** mode: devices join your wired network directly.
- **Local-only** mode: devices can talk to each other but not to the internet (needs NetworkManager 1.52 or later).
- Choice of Wi-Fi adapter, band (2.4/5 GHz) and channel, plus a hidden-network option.
- A list of connected devices.
- Per-device speed limits (type any value and pick kbps, Mbps, Gbps, KB/s or MB/s), priority, approval of new devices and data caps.
- A tray icon, launch at login, and an option to start the hotspot at login.
- English and Spanish interface.

## How it works

LinuxEasyHotspot doesn't run its own access-point daemon. It creates and manages a NetworkManager connection profile over D-Bus, so:

- creating and stopping the hotspot needs no root password,
- it doesn't conflict with your existing network setup,
- the hotspot keeps running if you close the app, and you can stop it from your desktop's network menu too.

Only the optional per-device features (speed limits, approval, data caps) need root, for Linux traffic control (`tc`). That job is done by `leh-shaper`, a small polkit-protected helper that D-Bus starts on demand; see [docs/helper.md](docs/helper.md).

**Requirements:** Linux with NetworkManager 1.30 or later, and a Wi-Fi adapter that supports AP mode. To check your adapter, run `nmcli -f WIFI-PROPERTIES.AP device show <wifi-iface>`.

## Install (Debian, Ubuntu, Linux Mint)

Build the package once, then install it. It includes the speed-limit helper, so nothing else is needed:

```sh
./scripts/build-deb.sh
sudo apt install ./target/debian/linuxeasyhotspot_*_amd64.deb
```

Remove it with `sudo apt remove linuxeasyhotspot`. Building needs the packages listed below plus `dpkg-dev` and `fakeroot`.

- The package is built against the system it's built on: built on Mint 22 / Ubuntu 24.04 it needs those or newer (`libc6 >= 2.39`). Build on the oldest release you want to support.
- Set `DEB_REVISION=2` to bump the package revision without changing the app version.
- If you installed the helper earlier with `install-helper.sh`, the package simply takes over those files.

## Building

```sh
# Debian/Ubuntu/Mint
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev gettext
# Fedora
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel gettext
# Arch
sudo pacman -S base-devel gtk4 libadwaita gettext

# Rust toolchain: https://rustup.rs
cargo build --release
```

### Running it

```sh
cargo run -p linuxeasyhotspot            # real hardware, through NetworkManager
```

Working today:
- start/stop, the QR code and password, the network name,
- Router mode (Bridge if Spike B passes),
- choosing the adapter, band, channel, internet source and hidden network,
- the connected-devices list,
- per-device control (with the helper), the tray, startup options and the language setting.

Settings are saved to `~/.config/linuxeasyhotspot/` (`config.toml`, `devices.toml`, both mode 0600).

**Speed limits and device control** need the small root helper. The `.deb` installs it; when running from source, install it with `./scripts/install-helper.sh` (see [docs/helper.md](docs/helper.md)). With it, you get:
- download and upload limits, priority and data caps per device,
- approval of new devices,
- defaults for new devices,
- total hotspot speed,
- live speeds and usage.

**Startup** (Settings → Startup): *Launch at login* adds `~/.config/autostart/io.github.OWNER.LinuxEasyHotspot.desktop`, which runs the app with `--hidden`; *Start hotspot at login* then turns the hotspot on. With *Show in system tray*, closing the window keeps the app in the tray; without a tray, launching the app again shows the window.

**Language** (Settings → Language): System, English or Español. Translations are standard gettext files in `po/`; after changing UI strings run `python3 scripts/update-po.py` and fill in the new entries (for example with Poedit).

### Demo mode

`--demo` runs the same UI on fake data, including the features that aren't wired up yet:

```sh
cargo run -p linuxeasyhotspot -- --demo
```

- **Main menu (☰) → Demo scenarios:** switch between error pages, helper states, a new device joining, and a start that fails.
- **Flags for quick checks:** `--running`, `--page settings|clients`, `--dark` / `--light`, `--size 900x620`.
- **Screenshots:** `scripts/screenshots.sh` regenerates `data/screenshots/`. It needs Xvfb.

### Trying the backend without the GUI

```sh
cargo run -p leh-nm --example cli -- status
cargo run -p leh-nm --example cli -- start MyHotspot my-password   # Ctrl+C stops it
```

## Project layout

| Crate | What it does |
|---|---|
| `crates/leh-model` | Pure logic: config, NM settings builders, the state machine, QR, password and parsers. No I/O, so it's heavily tested. |
| `crates/leh-nm` | The NetworkManager D-Bus backend (zbus) and connected-client discovery. |
| `crates/leh-shaper` | The root helper for traffic control (system D-Bus service, polkit-checked). |
| `crates/leh-gtk` | The GTK4 + libadwaita app (`linuxeasyhotspot`): pages, tray, autostart, translations. |

| Folder | What's in it |
|---|---|
| `data/` | Desktop entry, icons, D-Bus and polkit files, screenshots. |
| `packaging/debian/` | Control template and maintainer scripts used by `scripts/build-deb.sh`. |
| `po/` | Translations (`es.po`) and the generated template `linuxeasyhotspot.pot`. |
| `scripts/` | `build-deb.sh`, `install-helper.sh` / `uninstall-helper.sh`, `update-po.py`, `screenshots.sh`. |

See [docs/spikes.md](docs/spikes.md) for the feasibility checks behind the design.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Logs: run with `RUST_LOG=debug`. Passwords are always redacted.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
