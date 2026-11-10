#!/bin/bash
# Builds target/debian/linuxeasyhotspot_<version>-<rev>_<arch>.deb
# The package contains the app and the leh-shaper helper, so per-device
# speed limits work right after `sudo apt install ./linuxeasyhotspot_*.deb`.
#
# Needs: cargo, dpkg-deb, dpkg-shlibdeps (package dpkg-dev), fakeroot.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
ID=io.github.OWNER.LinuxEasyHotspot
REV=${DEB_REVISION:-1}
cd "$ROOT"

fail() { echo "✖ $*" >&2; exit 1; }
for t in dpkg-deb dpkg-shlibdeps fakeroot; do
  command -v "$t" >/dev/null || fail "$t not found (sudo apt install dpkg-dev fakeroot)"
done
CARGO=${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}
[ -x "$CARGO" ] || fail "cargo not found (install Rust from https://rustup.rs)"

VERSION=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)
[ -n "$VERSION" ] || fail "can't read the version from Cargo.toml"
ARCH=$(dpkg --print-architecture)

echo "Building release binaries…"
"$CARGO" build --release --locked -p linuxeasyhotspot -p leh-shaper

OUT=target/debian
PKG=$OUT/pkg
rm -rf "${PKG:?}"
mkdir -p "$PKG/DEBIAN"

# Files
install -Dm755 target/release/linuxeasyhotspot "$PKG/usr/bin/linuxeasyhotspot"
install -Dm755 target/release/leh-shaper "$PKG/usr/libexec/linuxeasyhotspot/leh-shaper"
strip --strip-unneeded "$PKG/usr/bin/linuxeasyhotspot" "$PKG/usr/libexec/linuxeasyhotspot/leh-shaper"
install -Dm644 "data/dbus/$ID.Shaper.conf" "$PKG/usr/share/dbus-1/system.d/$ID.Shaper.conf"
install -Dm644 "data/dbus/$ID.Shaper.service" "$PKG/usr/share/dbus-1/system-services/$ID.Shaper.service"
install -Dm644 "data/polkit/$ID.policy" "$PKG/usr/share/polkit-1/actions/$ID.policy"
install -Dm644 "data/$ID.desktop" "$PKG/usr/share/applications/$ID.desktop"
install -Dm644 "data/icons/hicolor/scalable/apps/$ID.svg" \
  "$PKG/usr/share/icons/hicolor/scalable/apps/$ID.svg"
install -Dm644 "data/icons/hicolor/symbolic/apps/$ID-symbolic.svg" \
  "$PKG/usr/share/icons/hicolor/symbolic/apps/$ID-symbolic.svg"
DOC=$PKG/usr/share/doc/linuxeasyhotspot
install -Dm644 packaging/debian/copyright "$DOC/copyright"
install -Dm644 README.md "$DOC/README.md"
gzip -9n -c CHANGELOG.md > "$DOC/changelog.gz"
cat > "$OUT/changelog.Debian" <<EOF
linuxeasyhotspot ($VERSION-$REV) unstable; urgency=medium

  * Release $VERSION. See /usr/share/doc/linuxeasyhotspot/changelog.gz.

 -- MazMorrDev <marconchelo12@gmail.com>  $(date -R)
EOF
gzip -9n -c "$OUT/changelog.Debian" > "$DOC/changelog.Debian.gz"

# Library dependencies (libgtk-4-1, libadwaita-1-0, libc6, …) from the binaries.
SHL=$OUT/shlibs
rm -rf "${SHL:?}"
mkdir -p "$SHL/debian"
printf 'Source: linuxeasyhotspot\n\nPackage: linuxeasyhotspot\nArchitecture: any\n' > "$SHL/debian/control"
SHLIBS=$(cd "$SHL" && dpkg-shlibdeps -O \
  "$ROOT/$PKG/usr/bin/linuxeasyhotspot" "$ROOT/$PKG/usr/libexec/linuxeasyhotspot/leh-shaper" \
  2>/dev/null | sed -n 's/^shlibs:Depends=//p')
[ -n "$SHLIBS" ] || fail "dpkg-shlibdeps found no library dependencies"

SIZE=$(du -sk --exclude=DEBIAN "$PKG" | cut -f1)
sed -e "s/@VERSION@/$VERSION-$REV/" -e "s/@ARCH@/$ARCH/" -e "s/@SIZE@/$SIZE/" \
    -e "s/@SHLIBS@/$SHLIBS/" packaging/debian/control.in > "$PKG/DEBIAN/control"
for s in postinst prerm postrm; do
  install -m755 "packaging/debian/$s" "$PKG/DEBIAN/$s"
done
chmod -R u=rwX,go=rX "$PKG"
(cd "$PKG" && find usr -type f -exec md5sum {} + | sort -k2) > "$PKG/DEBIAN/md5sums"

DEB=$OUT/linuxeasyhotspot_${VERSION}-${REV}_${ARCH}.deb
fakeroot dpkg-deb --build -Zxz "$PKG" "$DEB" >/dev/null
echo "✔ $DEB"
echo "  Install with:  sudo apt install ./$DEB"
