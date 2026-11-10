#!/bin/bash
# Renders the --demo UI on a virtual X server and saves PNGs to data/screenshots/.
# Needs: Xvfb, xwd (x11-apps), python3-pil. Build first: cargo build -p linuxeasyhotspot
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN="$ROOT/target/debug/linuxeasyhotspot"
OUT="$ROOT/data/screenshots"
mkdir -p "$OUT"

shot() { # name WxH [flags...]
  local name=$1 size=$2; shift 2
  Xvfb :99 -screen 0 1400x1100x24 >/dev/null 2>&1 & local xp=$!
  sleep 1
  DISPLAY=:99 GSK_RENDERER=cairo "$BIN" --demo --size "$size" "$@" >/dev/null 2>&1 & local ap=$!
  sleep "${WAIT:-3}"
  DISPLAY=:99 xwd -root -silent -out "$OUT/$name.xwd"
  kill "$ap" "$xp" 2>/dev/null || true; wait 2>/dev/null || true
  python3 -I "$ROOT/scripts/xwd2png.py" "$OUT/$name.xwd" "$OUT/$name.png" "$size"
  rm -f -- "$OUT/$name.xwd"
  echo "$OUT/$name.png"
}

shot home-off        420x700  --light
shot home-on         420x760  --running --light
shot home-on-dark    420x760  --running --dark
shot home-wide       900x620  --running --light
shot settings        420x1100 --page settings --light
shot clients         420x760  --running --page clients --light
shot clients-dark    420x760  --running --page clients --dark
WAIT=4 shot device   420x900  --running --page clients --device 3c:28:6d:11:22:33 --light
shot helper-missing  420x500  --page settings --scenario helper-missing --light
shot no-adapter      420x560  --scenario no-adapter --light
