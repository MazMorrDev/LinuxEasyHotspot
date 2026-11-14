#!/bin/bash
# Collects what is needed to debug network problems while the hotspot runs
# (e.g. "the wired connection drops when the hotspot starts").
#
#   ./scripts/diagnose-network.sh [output-file]
#
# It takes a snapshot of the network, asks you to start the hotspot from the
# app, takes another one, asks you to stop it, takes a last one and appends
# NetworkManager's log for that time. Nothing is changed on the system.
# No passwords are collected (nmcli hides secrets), but IP and MAC addresses
# are, so share the file only with whoever is helping you.
set -u
export LC_ALL=C.UTF-8
OUT=${1:-$HOME/leh-network-$(date +%Y%m%d-%H%M%S).txt}
START=$(date '+%F %T')

profile() {
  echo "--- profile: $1"
  nmcli -f connection,802-3-ethernet,802-1x,bridge,bridge-port,ipv4,ipv6 \
    connection show "$1" 2>&1 | grep -vE ':[[:space:]]+(--|\[\])?$'
}

snapshot() {
  echo
  echo "=================== $1 ($(date '+%T'))"
  echo "--- addresses"; ip -br addr
  echo "--- links"; ip -br link
  echo "--- routes"; ip route
  echo "--- bridge ports"; bridge link 2>/dev/null
  echo "--- devices"; nmcli -f DEVICE,TYPE,STATE,CONNECTION device
  echo "--- active connections"; nmcli -f NAME,UUID,TYPE,DEVICE connection show --active
  local gw
  gw=$(ip route | awk '/^default/ {print $3; exit}')
  echo "--- gateway ${gw:-none}"
  [ -n "$gw" ] && ping -c 3 -W 1 "$gw" | tail -2
  echo "--- internet"
  getent hosts debian.org >/dev/null && echo "DNS ok" || echo "DNS failed"
  ping -c 2 -W 2 1.1.1.1 >/dev/null && echo "ping 1.1.1.1 ok" || echo "ping 1.1.1.1 failed"
}

{
  echo "Linux Easy Hotspot network report, $START"
  echo "app: $(dpkg-query -W -f='${Version}' linuxeasyhotspot 2>/dev/null || echo 'no .deb installed')"
  echo "system: $(lsb_release -ds 2>/dev/null) / kernel $(uname -r) / desktop ${XDG_CURRENT_DESKTOP:-?}"
  nmcli --version
  echo "--- network cards"
  for d in /sys/class/net/*; do
    [ -e "$d/device" ] || continue
    echo "$(basename "$d") driver=$(basename "$(readlink -f "$d/device/driver")") bus=$(basename "$(readlink -f "$d/device/subsystem")") mac=$(cat "$d/address")"
  done
  echo "--- saved profiles"
  nmcli -f NAME,UUID,TYPE,AUTOCONNECT,DEVICE connection show
  # Wired profiles and the app's bridge profiles (not the Wi-Fi ones).
  nmcli -t -f NAME,TYPE connection show | while IFS=: read -r name type; do
    case "$type" in
      802-3-ethernet|bridge) profile "$name" ;;
    esac
  done
  snapshot "BEFORE (hotspot off)"
} >"$OUT" 2>&1

echo "1/2  Start the hotspot from the app now, in the mode where the wired"
echo "     connection drops. Wait ~20 seconds (until it has dropped), then"
read -rp "     press Enter here… " _
snapshot "WITH THE HOTSPOT ON" >>"$OUT" 2>&1
echo "     Write in one line what you see (e.g. 'no internet', 'Warpinator"
read -rp "     doesn't find my PC', 'applet says disconnected'): " what
echo "user says: $what" >>"$OUT"

echo "2/2  Stop the hotspot, wait ~10 seconds, then"
read -rp "     press Enter here… " _
{
  snapshot "AFTER STOPPING"
  echo
  echo "=================== NetworkManager log since $START"
  journalctl -u NetworkManager --since "$START" --no-pager -o short-precise
} >>"$OUT" 2>&1

echo
echo "Saved: $OUT"
