#!/usr/bin/env bash
# Makes unifyterm-VERSION-x86_64.AppImage from a release build (run after
# `cargo build --release`): unifyterm and ut, the .desktop file and the icon.
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
mkdir -p "$app/usr/bin" "$app/usr/share/doc/unifyterm"
cp "$root/target/release/unifyterm" "$root/target/release/ut" "$app/usr/bin/"
cp "$root/LICENSE" "$root/THIRD-PARTY-NOTICES" "$app/usr/share/doc/unifyterm/"
export LINUXDEPLOY_OUTPUT_VERSION="$version"
# linuxdeploy writes into the current directory and names the file after the
# .desktop file's Name (UnifyTerm-...): build in a directory of its own and
# take whatever it made, so the name here does not depend on that.
out="$(mktemp -d)"
( cd "$out" && "$linuxdeploy" --appdir "$app" \
  --desktop-file "$root/packaging/linux/unifyterm.desktop" \
  --icon-file "$root/packaging/icons/unifyterm-256.png" --icon-filename unifyterm \
  --output appimage )
made="$(find "$out" -maxdepth 1 -name '*.AppImage' | head -n 1)"
[ -n "$made" ] || { echo "linuxdeploy made no AppImage" >&2; exit 1; }
mv "$made" "unifyterm-$version-x86_64.AppImage"
echo "made unifyterm-$version-x86_64.AppImage"
