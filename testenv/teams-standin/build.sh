#!/bin/bash
# Build the stand-in app bundle on the host (Xcode 26.3).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; out="${1:?output dir}"; clips="${2:-$here/../build/clips}"
app="$out/Teams Stand-in.app"; rm -rf "$app"; mkdir -p "$app/Contents/MacOS"
swiftc -O -o "$app/Contents/MacOS/teams-standin" "$here/main.swift"
mkdir -p "$app/Contents/Resources"
[ -f "$clips/teams-call.wav" ] || { echo "teams-standin/build.sh: $clips/teams-call.wav missing, run clips/make-clips.sh first" >&2; exit 1; }
cp "$clips/teams-call.wav" "$app/Contents/Resources/teams-call.wav"
cp "$here/Info.plist" "$app/Contents/Info.plist"
codesign -s - --force "$app"
