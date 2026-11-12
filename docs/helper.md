# The Linux Easy Hotspot helper (`leh-shaper`)

Per-device speed limits, priorities, approval of new devices, data caps and total hotspot speed all need Linux traffic control (`tc`), which needs root. Linux Easy Hotspot keeps the app unprivileged and puts that one job in a small helper.

## Install

**With the .deb package:** nothing to do. `linuxeasyhotspot_*.deb` (built by `./scripts/build-deb.sh`) contains the helper and its D-Bus and polkit files.

**From source:** in Linux Easy Hotspot, open Settings or Clients, press **Set up…** on the banner, then **Install now**. The app builds the helper and your desktop asks for your password once.

**From a terminal** (the same dialog has a **Copy** button for this command):


```sh
./scripts/install-helper.sh      # builds it in release mode, asks for sudo
./scripts/uninstall-helper.sh    # removes everything again
```

The install script copies these files:

| File | Purpose |
|---|---|
| `/usr/libexec/linuxeasyhotspot/leh-shaper` | The helper binary |
| `/usr/share/dbus-1/system.d/io.github.OWNER.LinuxEasyHotspot.Shaper.conf` | Lets only root own the bus name |
| `/usr/share/dbus-1/system-services/io.github.OWNER.LinuxEasyHotspot.Shaper.service` | D-Bus starts the helper on demand |
| `/usr/share/polkit-1/actions/io.github.OWNER.LinuxEasyHotspot.policy` | Who may use it |

The app notices the helper within a few seconds; there's no need to restart it.

**Troubleshooting:** if the install says *"this terminal can't use sudo (no new privileges)"*, you're in a restricted shell, such as an IDE or AI-assistant terminal. Run it from a normal terminal window instead, or use **Install now**. Note that **Install now** also needs the app itself to be started from a normal terminal or the app menu.

## What it can and can't do

- **Input:** it accepts only a *structured* request: an interface, totals, and a list of (MAC, access, down, up, priority). `leh_model::tc` validates the request and turns it into commands. It never accepts command strings, and nothing goes through a shell.
- **Scope:** it only touches an interface that currently belongs to the active `Linux Easy Hotspot` NetworkManager connection (or `LinuxEasyHotspot`, its name up to 0.1.1), plus its own `lehifb0` device.
- **Permissions:** every call is checked with polkit.
  - `…shape` (change limits) and `…monitor` (read usage, remove its rules) are allowed for the active local desktop user. This matches NetworkManager's own rule, which already lets that user create the hotspot.
  - Remote and inactive sessions are refused. Administrators can tighten this with a polkit rule.
  - The helper answers `AccessDenied` only when polkit refuses; that is the one case where the app shows **Authorize**. Temporary problems (the hotspot is starting or stopping, NetworkManager or polkit busy) are `Failed` errors, which the app retries quietly. The app needs helper API level 3 or newer and asks for an update otherwise.
- **Cleanup:** it removes its rules when the hotspot stops (even if the app crashed; it waits for two checks in a row, so a single failed read of NetworkManager doesn't wipe them) or when it gets SIGTERM (package removal, shutdown), and exits after a minute of idleness.

## Device names

The app names devices by itself where it can (reverse DNS against NM's dnsmasq, or against your router in Bridge mode). The helper fills the gaps through `ClientNames` (polkit `…monitor`, hotspot interface only, read-only):

- **Bridge mode:** your router hands out addresses, so the helper listens on UDP port 67 of the bridge for the DHCP requests devices broadcast when they join, and keeps the name they send. It never answers, and the socket shares the port (`SO_REUSEPORT`) like dnsmasq does. A device that was connected before the helper started is named when it reconnects.
- **Router mode:** it reads `/var/lib/NetworkManager/dnsmasq-<iface>.leases`, which is root-only on Ubuntu/Mint.

## How limits are applied

- **Download** (to a device): an HTB class per device on the hotspot interface, matched by MAC.
- **Upload** (from a device): incoming traffic is redirected to `lehifb0`, which has the same classes.
- **Devices waiting for approval, or blocked:** they get DHCP (so the app can show their name) and nothing else.
- **Usage and live speeds:** per device, read from the class byte counters every 3 seconds. The hotspot's total (all devices together, shown at the top of Clients) doesn't need the helper: the app reads the Wi-Fi interface's own counters in `/sys/class/net/<iface>/statistics`.
- **Units:** the app stores speeds in kbit/s and caps in MB. What you type in the app ("2,5 MB/s", "800 kbps", "1 GB") is converted by `leh_model::units`; speeds must be between 8 kbit/s and 10 Gbit/s.

To check what's applied:

```sh
tc -s class show dev wlp2s0        # download classes and counters
tc -s class show dev lehifb0       # upload classes and counters
tc filter show dev wlp2s0 ingress  # approval and block rules
```
