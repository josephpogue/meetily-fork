#!/bin/bash
# Build the stand-in app bundle on the host (Xcode 26.3).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; out="${1:?output dir}"; clips="${2:-$here/../build/clips}"
app="$out/Zoom Stand-in.app"; rm -rf "$app"; mkdir -p "$app/Contents/MacOS"
swiftc -O -o "$app/Contents/MacOS/zoom-standin" "$here/main.swift"
mkdir -p "$app/Contents/Resources"
[ -f "$clips/zoom-call.wav" ] || { echo "zoom-standin/build.sh: $clips/zoom-call.wav missing, run clips/make-clips.sh first" >&2; exit 1; }
cp "$clips/zoom-call.wav" "$app/Contents/Resources/zoom-call.wav"
cp "$here/Info.plist" "$app/Contents/Info.plist"
codesign -s - --force "$app"
