#!/usr/bin/env bash
# Makes uni-term.app and uni-term-VERSION-macos-ARCH.dmg from a release build
# (run on macOS after `cargo build --release`).
#
#   packaging/macos/make-app.sh 0.1.0
#
# The .app holds uni-term and ut in Contents/MacOS, Info.plist and the icon.
# It is not signed: that needs the user's Apple developer certificate, and
# notarizing needs their account (docs/release.md). Unsigned, macOS asks once:
# open it with right-click > Open.
set -euo pipefail
version="${1:?usage: make-app.sh VERSION}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
arch="$(uname -m)"
out="$root/target/release/macos"
app="$out/uni-term.app"
rm -rf "$out"; mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/uni-term" "$root/target/release/ut" "$app/Contents/MacOS/"
sed "s/VERSION/$version/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
cp "$root/LICENSE" "$root/THIRD-PARTY-NOTICES" "$app/Contents/Resources/"
# The icon: every size iconutil wants, from the 512-pixel picture.
set_dir="$out/uni-term.iconset"; mkdir -p "$set_dir"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$root/packaging/icons/uni-term-512.png" --out "$set_dir/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2)); [ "$double" -le 512 ] && sips -z "$double" "$double" "$root/packaging/icons/uni-term-512.png" --out "$set_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set_dir" -o "$app/Contents/Resources/uni-term.icns"
rm -rf "$set_dir"
dmg="$root/target/release/uni-term-$version-macos-$arch.dmg"
hdiutil create -volname uni-term -srcfolder "$app" -ov -format UDZO "$dmg" >/dev/null
echo "made $app and $dmg"