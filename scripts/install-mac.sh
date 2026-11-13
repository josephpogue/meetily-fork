#!/bin/bash
# Installs the built app into /Applications and fixes the Screen Recording grant
# when the signature changed.
#
# macOS pins a Screen Recording grant to the app's code signature. When the new
# build's designated requirement differs from the installed one (for example the
# first install after moving from ad hoc to the "meetily-fork local" identity),
# the old grant silently stops working while System Settings still shows it on.
# This script then runs `tccutil reset ScreenCapture`, opens the app, and tells
# you to click Allow on the macOS prompt.
#
# Usage: scripts/install-mac.sh [--build] [--reset-tcc] [--app PATH] [--dry-run]
#   --build      build first with the signing identity (needs setup-mac-signing.sh)
#   --reset-tcc  reset the Screen Recording grant even when the signature matches
#   --app PATH   the built .app (default: target/release/bundle/macos/meetily-fork.app)
#   --dry-run    print the tccutil and open commands instead of running them
# Env: MEETILY_SIGNING_IDENTITY (default "meetily-fork local"), MEETILY_INSTALL_DIR
# (default /Applications). Quit the app first: this never stops a running recorder.
set -euo pipefail

PATH="$HOME/.cargo/bin:$PATH"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
IDENTITY="${MEETILY_SIGNING_IDENTITY:-meetily-fork local}"
INSTALL_DIR="${MEETILY_INSTALL_DIR:-/Applications}"
BUNDLE_ID="com.meetily.ai"
APP_NAME="meetily-fork.app"
APP="$REPO/target/release/bundle/macos/$APP_NAME"
BUILD=0; FORCE_RESET=0; DRY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --build) BUILD=1 ;;
    --reset-tcc) FORCE_RESET=1 ;;
    --dry-run) DRY=1 ;;
    --app) APP="$2"; shift ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

# The designated requirement, one line, without codesign's "# designated =>" prefix.
requirement() { codesign -d -r- "$1" 2>&1 | sed -n 's/^[# ]*designated => //p'; }

if [ "$BUILD" = 1 ]; then
  security find-identity -v -p codesigning | grep -q "\"$IDENTITY\"" || {
    echo "No signing identity \"$IDENTITY\". Run scripts/setup-mac-signing.sh once first." >&2; exit 1; }
  # tauri build exits 1 at the very end ("public key, no private key") because the updater
  # artifacts need a signing key this Mac does not have, after the app is already built and
  # signed. So a failure only counts when the app is not newer than the start of this build.
  stamp="$(mktemp)"; trap 'rm -f "$stamp"' EXIT
  (cd "$REPO/frontend" && APPLE_SIGNING_IDENTITY="$IDENTITY" pnpm run tauri:build) || echo "tauri build exited non-zero, checking the app was produced."
  [ "$APP" -nt "$stamp" ] || { echo "The build did not produce a new $APP." >&2; exit 1; }
fi

[ -d "$APP" ] || { echo "No built app at $APP. Run with --build, or pass --app." >&2; exit 1; }

new_req="$(requirement "$APP")"
case "$new_req" in
  *cdhash*|"")
    echo "The built app is signed ad hoc, so its Screen Recording grant breaks on every rebuild." >&2
    echo "Run scripts/setup-mac-signing.sh once, then build with --build (or APPLE_SIGNING_IDENTITY=\"$IDENTITY\")." >&2
    exit 1 ;;
esac

if pgrep -x "${MEETILY_PROCESS_NAME:-meetily}" >/dev/null 2>&1; then
  echo "meetily is running (it may be recording). Quit it, then run this again." >&2
  exit 1
fi

dest="$INSTALL_DIR/$APP_NAME"
old_req=""
[ -d "$dest" ] && old_req="$(requirement "$dest")"

rm -rf "$dest"
ditto "$APP" "$dest"
echo "Installed $dest"

run() { if [ "$DRY" = 1 ]; then echo "[dry run] $*"; else "$@"; fi; }

if [ "$FORCE_RESET" = 1 ] || [ "$old_req" != "$new_req" ]; then
  [ "$FORCE_RESET" = 1 ] && echo "Resetting the Screen Recording grant (asked for)." \
    || echo "The signature changed (was: ${old_req:-not installed}), so the old Screen Recording grant is stale. Resetting it."
  run tccutil reset ScreenCapture "$BUNDLE_ID"
  run open "$dest"
  echo
  echo "ACTION: click Allow on the macOS Screen Recording prompt, then restart the recorder."
else
  echo "Same signature as the installed app: the Screen Recording grant still applies."
  run open "$dest"
fi
