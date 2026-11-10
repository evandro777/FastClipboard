#!/usr/bin/env bash
# Copy path end to end on a private Xvfb: a small text, a 300 KB text (INCR) and a >1 MB PNG (INCR) are copied with
# xclip and must show up in history.toml / images/. Needs Xvfb, xdpyinfo, xclip and ImageMagick (convert).
# Usage: dev/e2e-copy.sh [path/to/fastclipboard]   (default: target/fast/fastclipboard, build it first)
set -u
cd "$(dirname "$0")/.."
bin=${1:-target/fast/fastclipboard}
[ -x "$bin" ] || { echo "build first: cargo build --profile fast"; exit 2; }

disp=:97
work=$(mktemp -d)
pids=()
cleanup() {
    for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done
    rm -rf "$work"
}
trap cleanup EXIT

Xvfb "$disp" -screen 0 800x600x24 >/dev/null 2>&1 &
pids+=($!)
sleep 1
# Never run anything against a display that is not ours (the real session would be hit).
size=$(DISPLAY=$disp xdpyinfo 2>/dev/null | awk '/dimensions:/ {print $2}')
[ "$size" = "800x600" ] || { echo "private Xvfb $disp did not start (got '$size')"; exit 2; }
export DISPLAY=$disp

mkdir -p "$work/data" "$work/run"
chmod 700 "$work/run"
# Own data dir, runtime dir and no session bus: the real history, instance socket and tray stay untouched.
export XDG_DATA_HOME="$work/data" XDG_RUNTIME_DIR="$work/run"
unset DBUS_SESSION_BUS_ADDRESS
"$bin" >"$work/app.log" 2>&1 &
pids+=($!)
sleep 2
kill -0 "${pids[-1]}" 2>/dev/null || { echo "fastclipboard exited:"; cat "$work/app.log"; exit 1; }

hist="$work/data/FastClipboard/history.toml"
fail=0
check() { # name, condition result
    if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi
}
# Each copy needs its own selection owner; xclip stays alive until the app has read it, then we stop it.
copy() { # args for xclip
    xclip -selection clipboard "$@" &
    local x=$!
    sleep 2
    kill "$x" 2>/dev/null
}

echo "hello fastclipboard e2e" | copy -i
grep -q "hello fastclipboard e2e" "$hist" 2>/dev/null
check "small text" $?

head -c 300000 /dev/zero | tr '\0' 'x' >"$work/big.txt"
copy -i "$work/big.txt"
[ "$(grep -c 'xxxxxxxxxx' "$hist" 2>/dev/null)" -ge 1 ] && [ "$(wc -c <"$hist")" -gt 300000 ]
check "300 KB text (INCR)" $?

convert -size 1200x1200 xc:gray +noise Random "$work/big.png"
[ "$(wc -c <"$work/big.png")" -gt 1000000 ] || { echo "test PNG is too small"; exit 2; }
copy -i -t image/png "$work/big.png"
ls "$work/data/FastClipboard/images/"*.png >/dev/null 2>&1 && ls "$work/data/FastClipboard/images/"*.thumb.png >/dev/null 2>&1
check ">1 MB PNG (INCR) stored with its thumbnail" $?
grep -q "image" "$hist" 2>/dev/null
check "PNG entry in history.toml" $?

kill -0 "${pids[-1]}" 2>/dev/null
check "app still running" $?
exit $fail
