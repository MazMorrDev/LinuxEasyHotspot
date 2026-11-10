# Spikes

Feasibility checks done before building on an assumption.

## A: AP + shared hotspot on NetworkManager 1.46 (PASSED, 2026-11-10)

Test system: Linux Mint 22.3 (Ubuntu 24.04 base), NM 1.46.0. Wi-Fi `wlp2s0` reports AP, 2.4 GHz and 5 GHz support; upstream is Ethernet `enp0s31f6`.

```sh
nmcli connection add type wifi ifname wlp2s0 con-name leh-spike-a autoconnect no \
  ssid LEH-SpikeA 802-11-wireless.mode ap 802-11-wireless.band bg \
  ipv4.method shared wifi-sec.key-mgmt wpa-psk wifi-sec.psk <pw> \
  connection.permissions "user:$USER"
nmcli --wait 40 connection up leh-spike-a
```

Results:
- The hotspot activated in about 3 s and got `10.42.0.1/24`.
- **No polkit prompt** for a normal desktop user, because of `connection.permissions=user:<me>`.
- **`/var/lib/NetworkManager/` is `drwx------ root`,** so the dnsmasq lease file can't be read as a user. The client list therefore uses the kernel neighbor table plus reverse DNS against NM's dnsmasq (`10.42.0.1:53`), and treats the lease file as an optional extra. This also means the Flatpak `--filesystem=/var/lib/NetworkManager:ro` permission is pointless on such distros.

Later, the same flow ran through `leh-nm`'s controller (`cargo run -p leh-nm --example cli -- start`):
- Idle → Activating → Active took about 3 s, and the gateway was detected.
- Ctrl+C stopped it cleanly.
- Starting again reused the same profile; nothing was duplicated.
- A hotspot started with `nmcli connection up LinuxEasyHotspot` was picked up as `Active`.

## B: AP as a bridge port alongside Ethernet (PENDING)

Bridging takes the Ethernet link down briefly, so this needs to be run deliberately by a person at the machine, not from a remote session:

```sh
cargo run -p leh-nm --example cli -- start LEH-Bridge <password> bridge:enp0s31f6
```

Pass criteria:
- A phone gets an IP from the home router's subnet.
- Ctrl+C restores `Conexión cableada 1` on `enp0s31f6`.

If NM refuses the AP profile as a bridge port, cut Bridge mode from the MVP.

## C: Local-only isolation on NM ≥ 1.52 (PENDING)

This needs a machine or VM with NM 1.52 or later. With `ipv4.forwarding=no` (D-Bus int32 `0`), check two things:
- From a connected phone, the internet is unreachable.
- The host's LAN (for example the home router) is unreachable too.

If either check fails, remove Local-only mode instead of hiding it.
