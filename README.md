# Easy Hotspot

The simplest way to turn your computer into a Wi-Fi hotspot, on **Linux** and **Windows 10/11**. Press one button, scan a QR code with your phone, and you're connected. (Formerly *Linux Easy Hotspot*.)

> **Status:** early development (0.2.0). On Linux, Router mode, per-device control, startup options and the `.deb` package work on real hardware; Bridge and Local-only still need field testing.
>
> **New interface:** the app is moving from GTK to [GPUI](https://gpui.rs) so the same interface runs on Linux and Windows (`crates/easyhotspot`, binary `easyhotspot`). The Windows version has only been compile-checked so far and still needs testing on real hardware. The GTK app (`linuxeasyhotspot`) stays in the repository until the new one does everything it does.

## Features

- One-click start/stop, with a QR code so phones can join by scanning.
- An auto-generated, easy-to-read password and a name based on your computer's hostname.
- **Router** mode: devices share this computer's internet (NAT).
- **Bridge** mode: devices join your wired network directly. This computer keeps its address on the LAN (the bridge takes over the Ethernet card's MAC and IP settings).
- **Local-only** mode: devices can talk to each other but not to the internet (needs NetworkManager 1.52 or later).
- Choice of Wi-Fi adapter, band (2.4/5 GHz) and channel, plus a hidden-network option. *Automatic* prefers a USB adapter; adapters plugged in while the app runs are picked up, and ones that can't be used (no AP mode, no driver, USB stick still in "storage mode") are explained in Settings.
- Clients: the hotspot's current speed and data used, devices waiting for approval, and every known device (connected or not) in *Allowed* and *Blocked* lists, each with its own settings.
- Per-device speed limits (type any value and pick kbps, Mbps, Gbps, KB/s or MB/s), priority, approval of new devices and data caps.
- A tray icon, launch at login, and an option to start the hotspot at login.
- English and Spanish interface.

## How it works

### Linux

Easy Hotspot doesn't run its own access-point daemon. It creates and manages a NetworkManager connection profile over D-Bus, so:

- creating and stopping the hotspot needs no root password,
- it doesn't conflict with your existing network setup,
- the hotspot keeps running if you close the app, and you can stop it from your desktop's network menu too.

Only the optional per-device features (speed limits, approval, data caps) need root, for Linux traffic control (`tc`). That job is done by `leh-shaper`, a small polkit-protected helper that D-Bus starts on demand; see [docs/helper.md](docs/helper.md).

**Requirements:** Linux with NetworkManager 1.30 or later, and a Wi-Fi adapter that supports AP mode. To check your adapter, run `nmcli -f WIFI-PROPERTIES.AP device show <wifi-iface>`. The GPUI interface also needs a working Vulkan driver (Mesa provides one for Intel, AMD and software rendering).

### Windows

On Windows the app drives the built-in **Mobile Hotspot** (WinRT `NetworkOperatorTetheringManager`), so it needs no administrator rights or drivers, and the hotspot keeps running if you close the app; you can also stop it from the quick settings.

| Works | Not available on Windows (yet) |
|---|---|
| Start/stop, QR code, name and password | Bridge and Local-only modes |
| Choosing the Wi-Fi adapter and the connection to share | Channel and hidden network |
| Band (Windows 11) | Per-device speed limits, approval and data caps |
| Connected devices (name, address), renaming them, the list of known devices | |
| Total speed and data used, session log | |
| Tray icon, launch at login, English/Spanish | |

The app turns off Windows' "power saving" for the hotspot (which stops it after 5 minutes without devices) while it runs, and restores it afterwards.

Per-device control on Windows (blocking and speed limits) needs a packet filter running with administrator rights; it's planned as a separate service built on [WinDivert](https://reqrypt.org/windivert.html), like the Linux helper.

**Requirements:** Windows 10 or 11 (band selection: Windows 11), a Wi-Fi adapter Windows' Mobile Hotspot can use (if *Settings → Network & internet → Mobile hotspot* works, the app will), and a connection to share.

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

### Linux

```sh
# Debian/Ubuntu/Mint
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libxkbcommon-x11-dev libxcb1-dev libfontconfig-dev gettext
# Fedora
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel libxkbcommon-x11-devel libxcb-devel fontconfig-devel gettext
# Arch
sudo pacman -S base-devel gtk4 libadwaita libxkbcommon-x11 libxcb fontconfig gettext

# Rust toolchain: https://rustup.rs
cargo build --release
```

(GTK and libadwaita are only needed for the GTK app; the GPUI app needs the xkbcommon, xcb and fontconfig packages.)

### Windows

Install [Rust](https://rustup.rs) with the Visual Studio Build Tools (the "Desktop development with C++" workload, which includes the Windows SDK), then:

```powershell
cargo build --release -p easyhotspot     # target\release\easyhotspot.exe
```

Release builds compile GPUI's shaders with `fxc.exe` from the Windows SDK; set `GPUI_FXC_PATH` if it isn't found. Only `easyhotspot`, `leh-win` and `leh-model` build on Windows (the others are Linux-only), so use `-p` instead of `--workspace`. CI builds the `.exe` on every push (artifact `easyhotspot-windows-x64`).

To check the Windows build from Linux: `rustup target add x86_64-pc-windows-msvc`, then `cargo check -p easyhotspot --target x86_64-pc-windows-msvc` with `llvm-rc` on the `PATH` (e.g. `PATH=/usr/lib/llvm-18/bin:$PATH`).

### Running it

```sh
cargo run -p easyhotspot                 # the new interface (Linux or Windows)
cargo run -p linuxeasyhotspot            # the GTK interface (Linux)
```

Launching `easyhotspot` again while it runs brings up the running window. `RUST_LOG=info` shows what the app and GPUI are doing.

Working today:
- start/stop, the QR code and password, the network name,
- Router mode (Bridge if Spike B passes),
- choosing the adapter, band, channel, internet source and hidden network,
- the connected-devices list,
- per-device control (with the helper), the tray, startup options and the language setting.

Settings are saved to `~/.config/linuxeasyhotspot/` on Linux (`config.toml`, `devices.toml`, both mode 0600) and to `%APPDATA%\Easy Hotspot\` on Windows.

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
cargo run -p easyhotspot -- --demo
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
| `crates/leh-win` | The Windows backend: the built-in Mobile Hotspot through WinRT (types build everywhere, the implementation only on Windows). |
| `crates/easyhotspot` | The GPUI app (`easyhotspot`) for Linux and Windows: model, pages, tray, autostart, single instance, translations. |
| `crates/leh-gtk` | The GTK4 + libadwaita app (`linuxeasyhotspot`), kept until the GPUI app replaces it. |

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
