#!/bin/bash
set -euo pipefail
eval "$(/opt/homebrew/bin/brew shellenv)"
cd ~/testenv-ui && npm install --omit=dev
