#!/bin/sh
# build-deb.sh <oxlet binary> <version> <output.deb>
set -eu
here=$(dirname "$0")
root=$(mktemp -d)
chmod 755 "$root"
trap 'rm -rf "$root"' EXIT
install -Dm755 "$1" "$root/usr/bin/oxlet"
install -Dm644 "$here/oxlet.desktop" "$root/usr/share/applications/dev.oxlet.app.desktop"
install -Dm644 "$here/../icons/oxlet-256.png" "$root/usr/share/icons/hicolor/256x256/apps/oxlet.png"
install -d "$root/DEBIAN"
sed "s|__VERSION__|$2|" "$here/control" > "$root/DEBIAN/control"
dpkg-deb --build --root-owner-group "$root" "$3"
