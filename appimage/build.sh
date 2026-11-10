#!/usr/bin/env bash
# Build FastClipboard-<version>-x86_64.AppImage (+ .zsync for delta updates) in the repo root.
# Needs: the system deps from AGENTS.md, a Rust toolchain, and `appimagetool` on PATH.
# Host libraries (fontconfig, xkbcommon, X11) are not bundled: every X11 desktop has them.
set -euo pipefail
cd "$(dirname "$0")/.."
id=io.github.evandro777.FastClipboard
version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
out=FastClipboard-$version-x86_64.AppImage

cargo build --release --locked

rm -rf appimage/AppDir
d=appimage/AppDir
install -Dm755 target/release/fastclipboard "$d/usr/bin/fastclipboard"
install -Dm644 packaging/$id.desktop "$d/usr/share/applications/$id.desktop"
install -Dm644 packaging/$id.svg "$d/usr/share/icons/hicolor/scalable/apps/$id.svg"
install -Dm644 packaging/$id.metainfo.xml "$d/usr/share/metainfo/$id.metainfo.xml"
# appimagetool wants the desktop file and a PNG icon at the root too (it builds .DirIcon from the PNG, not the SVG).
cp "$d/usr/share/applications/$id.desktop" "$d/usr/share/icons/hicolor/scalable/apps/$id.svg" "$d/"
install -m644 packaging/$id.png "$d/$id.png"
ln -s usr/bin/fastclipboard "$d/AppRun"

# Update info: AppImageUpdate-style tools read it from the file and fetch the .zsync from the latest release.
ARCH=x86_64 appimagetool --no-appstream \
  -u "gh-releases-zsync|evandro777|FastClipboard|latest|FastClipboard-*x86_64.AppImage.zsync" "$d" "$out"
echo ">> $out and $out.zsync"
