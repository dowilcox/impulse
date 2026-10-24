#!/bin/bash
# Screenshot impulse-app under Xvfb (Linux, software Vulkan via lavapipe).
# usage: scripts/spike/linux-shot.sh <out.png> [impulse-app args...]
# Needs: Xvfb, xdotool, ImageMagick (import), mesa-vulkan-drivers.
# GPUI shows its first frame under Xvfb only after the window gets an
# event, so the window is moved and resized before the capture.
OUT=$1; shift
XDG=$(mktemp -d)
Xvfb :95 -screen 0 1400x900x24 >/dev/null 2>&1 & XP=$!
sleep 1
DISPLAY=:95 XDG_RUNTIME_DIR=$XDG RUST_LOG=warn target/debug/impulse-app "$@" > /dev/null 2>&1 & AP=$!
sleep 4
W=$(DISPLAY=:95 xdotool search --pid $AP | head -1)
DISPLAY=:95 xdotool windowmove $W 0 0 windowsize $W 1280 820
sleep 4
DISPLAY=:95 import -window root -crop 1280x820+0+0 "$OUT"
kill $AP $XP 2>/dev/null; wait 2>/dev/null
