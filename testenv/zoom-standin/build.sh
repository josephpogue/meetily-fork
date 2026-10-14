#!/bin/bash
# Build the stand-in app bundle on the host (Xcode 26.3).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; out="${1:?output dir}"
app="$out/Zoom Stand-in.app"; rm -rf "$app"; mkdir -p "$app/Contents/MacOS"
swiftc -O -o "$app/Contents/MacOS/zoom-standin" "$here/main.swift"
cp "$here/Info.plist" "$app/Contents/Info.plist"
codesign -s - --force "$app"
