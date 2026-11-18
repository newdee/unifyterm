#!/usr/bin/env bash
# Makes uni-term-VERSION-x86_64.AppImage from a release build (run after
# `cargo build --release`): uni-term and ut, the .desktop file and the icon.
#
#   packaging/linux/appimage.sh 0.1.0
#
# linuxdeploy (https://github.com/linuxdeploy/linuxdeploy) must be on PATH, or
# LINUXDEPLOY must name it; CI downloads it. The libraries GPUI loads at run
# time (Vulkan, xkbcommon) are the system's, as for any desktop program.
set -euo pipefail
version="${1:?usage: appimage.sh VERSION}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
linuxdeploy="${LINUXDEPLOY:-linuxdeploy}"
app="$(mktemp -d)/AppDir"
mkdir -p "$app/usr/bin" "$app/usr/share/doc/uni-term"
cp "$root/target/release/uni-term" "$root/target/release/ut" "$app/usr/bin/"
cp "$root/LICENSE" "$root/THIRD-PARTY-NOTICES" "$app/usr/share/doc/uni-term/"
export VERSION="$version"
"$linuxdeploy" --appdir "$app" \
  --desktop-file "$root/packaging/linux/uni-term.desktop" \
  --icon-file "$root/packaging/icons/uni-term-256.png" --icon-filename uni-term \
  --output appimage
mv uni-term-*.AppImage "uni-term-$version-x86_64.AppImage"
echo "made uni-term-$version-x86_64.AppImage"
