#!/bin/bash
# Build the release app with the WebDriver add-on, for test Macs only. It lands in target/testenv/, never in the
# release bundle path, and the last line printed is its path.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"; cd "$root/frontend"
export PATH="$HOME/.cargo/bin:$PATH"
conf=$(mktemp -t tauri-testenv); mv "$conf" "$conf.json"; conf="$conf.json"
trap 'rm -f "$conf"' EXIT
# createUpdaterArtifacts off: the updater signing key is Zackriya's and a test build needs no updater bundle (without this, tauri build exits 1 after the .app is made).
jq '.app.security.capabilities |= map(if .identifier == "main" then .permissions += ["wdio-webdriver:default"] else . end) | .bundle.createUpdaterArtifacts = false' src-tauri/tauri.conf.json > "$conf"
jq -e '[.app.security.capabilities[] | select(.identifier == "main") | .permissions[] | select(. == "wdio-webdriver:default")] | length == 1' "$conf" >/dev/null \
  || { echo 'tauri.conf.json has no "main" capability to add wdio-webdriver:default to' >&2; exit 1; }
# Same GPU feature the release build picks (scripts/tauri-auto.js), plus the add-on.
gpu=$(node scripts/auto-detect-gpu.js); features="testenv"; [ -n "$gpu" ] && [ "$gpu" != none ] && features="$gpu,testenv"
# Only the .app bundle, no DMG: a test build must never leave an installable file under the release name.
pnpm tauri build --config "$conf" --bundles app -- --features "$features"
# The repo root is a cargo workspace, so bundles land in <root>/target. Move the test app out of the release
# bundle path into its own folder, so the release path never holds a build with the WebDriver add-on.
built="$root/target/release/bundle/macos/meetily-fork.app" app="$root/target/testenv/meetily-fork.app"
rm -rf "$app"; mkdir -p "$(dirname "$app")"; mv "$built" "$app"
LC_ALL=C grep -aq 'wdio-webdriver' "$app/Contents/MacOS/meetily" || { echo "built app has no WebDriver add-on" >&2; exit 1; }
otool -L "$app/Contents/MacOS/meetily" | grep -q /opt/homebrew && { echo "built app links Homebrew" >&2; exit 1; }
echo "$app"
