# The LinuxEasyHotspot helper (`leh-shaper`)

Per-device speed limits, priorities, approval of new devices, data caps and total hotspot speed all need Linux traffic control (`tc`), which needs root. LinuxEasyHotspot keeps the app unprivileged and puts that one job in a small helper.

## Install (from the source folder)

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

Then press **Check again** in the app, or restart it.

## What it can and can't do

- **Input:** it accepts only a *structured* request: an interface, totals, and a list of (MAC, access, down, up, priority). `leh_model::tc` validates the request and turns it into commands. It never accepts command strings, and nothing goes through a shell.
- **Scope:** it only touches an interface that currently belongs to the active `LinuxEasyHotspot` NetworkManager connection, plus its own `lehifb0` device.
- **Permissions:** every call is checked with polkit.
  - `…shape` (change limits) and `…monitor` (read usage, remove its rules) are allowed for the active local desktop user. This matches NetworkManager's own rule, which already lets that user create the hotspot.
  - Remote and inactive sessions are refused. Administrators can tighten this with a polkit rule.
- **Cleanup:** it removes its rules when the hotspot stops (even if the app crashed), and exits after a minute of idleness.

## How limits are applied

- **Download** (to a device): an HTB class per device on the hotspot interface, matched by MAC.
- **Upload** (from a device): incoming traffic is redirected to `lehifb0`, which has the same classes.
- **Devices waiting for approval, or blocked:** they get DHCP (so the app can show their name) and nothing else.
- **Usage and live speeds:** read from the class byte counters every 3 seconds.

To check what's applied:

```sh
tc -s class show dev wlp2s0        # download classes and counters
tc -s class show dev lehifb0       # upload classes and counters
tc filter show dev wlp2s0 ingress  # approval and block rules
```
