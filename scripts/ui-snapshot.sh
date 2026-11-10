#!/usr/bin/env bash
# Renders pages of the app on a private Xvfb display with canned data (FASTCLIPBOARD_FIXTURES) and saves PNGs to
# target/ui-snapshots/<page>-<mode>.png. The real system is never scanned or changed.
# Usage: scripts/ui-snapshot.sh [page ...]   (names or numbers; default all)   MODES="empty single many" to narrow.
set -euo pipefail
cd "$(dirname "$0")/.."

# Add one entry per sidebar page, with the number the app's `page` property uses.
declare -A PAGES=([main]=0 [preferences]=1)
MODES=${MODES:-"empty single many"}
OUT=target/ui-snapshots
DISPLAY_NUM=:99
mkdir -p "$OUT"

cargo build --quiet

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
APP=
cleanup() { [ -n "$APP" ] && kill "$APP" 2>/dev/null || true; kill "$XVFB" 2>/dev/null || true; }
trap cleanup EXIT
sleep 1

[ $# -gt 0 ] && names=("$@") || names=("${!PAGES[@]}")

for name in "${names[@]}"; do
  page=${PAGES[$name]:-$name}
  for mode in $MODES; do
    DISPLAY=$DISPLAY_NUM FASTCLIPBOARD_FIXTURES=$mode FASTCLIPBOARD_PAGE=$page target/debug/fastclipboard >/dev/null 2>&1 &
    APP=$!
    for _ in $(seq 50); do
      DISPLAY=$DISPLAY_NUM xdotool search --name FastClipboard >/dev/null 2>&1 && break
      sleep 0.2
    done
    sleep 1
    DISPLAY=$DISPLAY_NUM import -window root "$OUT/$name-$mode.png"
    kill "$APP" 2>/dev/null || true
    wait "$APP" 2>/dev/null || true
    APP=
    echo "$OUT/$name-$mode.png"
  done
done
