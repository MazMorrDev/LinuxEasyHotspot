# LinuxEasyHotspot

The simplest way to turn your Linux computer into a Wi-Fi hotspot. Press one button, scan a QR code with your phone, and you're connected.

> **Status:** early development. The NetworkManager backend works; the GTK interface is being built.

## Features

- One-click start/stop, with a QR code so phones can join by scanning.
- An auto-generated, easy-to-read password and a name based on your computer's hostname.
- **Router** mode: devices share this computer's internet (NAT).
- **Bridge** mode: devices join your wired network directly.
- **Local-only** mode: devices can talk to each other but not to the internet (needs NetworkManager 1.52 or later).
- Choice of Wi-Fi adapter, band (2.4/5 GHz) and channel, plus a hidden-network option.
- A list of connected devices.
- A tray icon and an option to start at login.

## How it works

LinuxEasyHotspot doesn't run its own access-point daemon. It creates and manages a NetworkManager connection profile over D-Bus, so:

- it needs no root password and no helper programs,
- it doesn't conflict with your existing network setup,
- the hotspot keeps running if you close the app, and you can stop it from your desktop's network menu too.

**Requirements:** Linux with NetworkManager 1.30 or later, and a Wi-Fi adapter that supports AP mode. To check your adapter, run `nmcli -f WIFI-PROPERTIES.AP device show <wifi-iface>`.

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
- choosing the adapter, band, channel and hidden network,
- the connected-devices list.

Settings are saved to `~/.config/linuxeasyhotspot/` (`config.toml`, `devices.toml`, both mode 0600).

**Speed limits and device control** need the small root helper. Install it with `./scripts/install-helper.sh` (see [docs/helper.md](docs/helper.md)). With it, you get:
- download and upload limits, priority and data caps per device,
- approval of new devices,
- defaults for new devices,
- total hotspot speed,
- live speeds and usage.

Not yet available: the tray and autostart.

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
| `crates/leh-gtk` | The GTK4 + libadwaita app (`linuxeasyhotspot`). |

See [docs/spikes.md](docs/spikes.md) for the feasibility checks behind the design.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Logs: run with `RUST_LOG=debug`. Passwords are always redacted.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
