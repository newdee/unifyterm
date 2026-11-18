#!/bin/sh
# Installs UnifyTerm on Linux (x86_64) or macOS (Apple silicon or Intel) from
# its latest GitHub release, for you alone (no root):
#
#   curl -fsSL https://raw.githubusercontent.com/newdee/unifyterm/main/install.sh | sh
#
# What it does: downloads unifyterm-<tag>-<system>.tar.gz and its .sha256,
# checks the checksum, puts `unifyterm` and `ut` in ~/.local/bin, and adds
# the program to the desktop: UnifyTerm.app in ~/Applications on macOS, a
# menu entry and icon in ~/.local/share on Linux. Run it again to update.
#
# Settings, by environment variable:
#   UNIFYTERM_VERSION  a version (0.1.0) instead of the latest
#   UNIFYTERM_BIN      where the two programs go (default ~/.local/bin)
#   UNIFYTERM_BASE     where the releases are (default the GitHub releases)
set -eu

repo="newdee/unifyterm"
bin="${UNIFYTERM_BIN:-$HOME/.local/bin}"
base="${UNIFYTERM_BASE:-https://github.com/$repo/releases/download}"

say() { printf 'unifyterm install: %s\n' "$*"; }
die() { say "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "it needs $1"; }
need curl
need tar

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=linux-x86_64 ;;
  Darwin-arm64) target=macos-aarch64 ;;
  Darwin-x86_64) target=macos-x86_64 ;;
  *) die "no build for $(uname -s) $(uname -m) yet: see https://github.com/$repo#install" ;;
esac

if [ -n "${UNIFYTERM_VERSION:-}" ]; then
  tag="v${UNIFYTERM_VERSION#v}"
else
  tag=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$tag" ] || die "could not find the latest release of $repo"
fi

name="unifyterm-$tag-$target"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
say "downloading $name"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$tag/$name.tar.gz"
curl -fsSL -o "$tmp/$name.tar.gz.sha256" "$base/$tag/$name.tar.gz.sha256"

# The .sha256 file is "<hash>  <file name>".
want=$(cut -d' ' -f1 "$tmp/$name.tar.gz.sha256")
if command -v sha256sum >/dev/null 2>&1; then
  got=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
else
  got=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1)
fi
[ "$want" = "$got" ] || die "the checksum does not match: expected $want, got $got"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
from="$tmp/$name"
mkdir -p "$bin"
for program in unifyterm ut; do
  cp "$from/$program" "$bin/$program.new"
  chmod 755 "$bin/$program.new"
  mv -f "$bin/$program.new" "$bin/$program"
done

case "$target" in
  macos-*)
    if [ -d "$from/UnifyTerm.app" ]; then
      mkdir -p "$HOME/Applications"
      rm -rf "$HOME/Applications/UnifyTerm.app"
      cp -R "$from/UnifyTerm.app" "$HOME/Applications/"
      say "UnifyTerm.app is in ~/Applications"
    fi
    ;;
  linux-*)
    share="${XDG_DATA_HOME:-$HOME/.local/share}"
    mkdir -p "$share/applications" "$share/icons/hicolor/256x256/apps"
    sed "s|^Exec=unifyterm|Exec=$bin/unifyterm|" "$from/unifyterm.desktop" > "$share/applications/unifyterm.desktop"
    cp "$from/unifyterm-256.png" "$share/icons/hicolor/256x256/apps/unifyterm.png"
    say "a menu entry is in $share/applications"
    ;;
esac

say "installed $tag: $bin/unifyterm and $bin/ut"
case ":$PATH:" in
  *":$bin:"*) ;;
  *) say "add $bin to your PATH, for one: echo 'export PATH=\"$bin:\$PATH\"' >> ~/.profile" ;;
esac
