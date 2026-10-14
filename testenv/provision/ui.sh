#!/bin/bash
# The base image installs Node as keg-only node@24 and links it into /opt/homebrew/bin, but only ~/.zprofile
# sets the keg on PATH, and Packer's shell is not a login shell. Put both on PATH here.
set -euo pipefail
export PATH="/opt/homebrew/opt/node@24/bin:/opt/homebrew/bin:$PATH"
cd ~/testenv-ui && npm ci --omit=dev
# `testenv exec` (tart exec) runs with this PATH and no profile; `node ~/testenv-ui/ui.mjs` must resolve there with no symlink of ours.
exec_path="/bin:/usr/bin:/usr/sbin:/usr/local/bin:/opt/homebrew/bin"
env -i PATH="$exec_path" HOME="$HOME" bash -c 'command -v node && node -v' \
  || { echo "ui.sh: node is not on the testenv exec PATH ($exec_path)" >&2; exit 1; }
out=$(env -i PATH="$exec_path" HOME="$HOME" node "$HOME/testenv-ui/ui.mjs" bogus 2>&1 || true)
grep -q usage <<<"$out" \
  || { echo "ui.sh: ui.mjs does not start under the testenv exec PATH" >&2; exit 1; }
