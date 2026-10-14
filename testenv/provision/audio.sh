#!/bin/bash
# Two separate virtual devices, so call audio never leaks into the microphone:
#   BlackHole 2ch  = the fake microphone (default input; `testenv play mic` writes into it with sox)
#   BlackHole 16ch = the fake speakers (default output; `testenv play call` plays there with afplay,
#                    and Meetily's system-audio capture hears it as Them)
# The VM runs with --no-audio, so these are its only audio devices.
set -euo pipefail
eval "$(/opt/homebrew/bin/brew shellenv)"
brew install --cask blackhole-2ch blackhole-16ch
brew install sox switchaudio-osx
# The cask says "reboot to take effect", but restarting coreaudiod loads the new drivers (SIP is off in this image).
sudo killall coreaudiod
for _ in $(seq 1 30); do
  devices=$(SwitchAudioSource -a 2>/dev/null || true)
  if grep -qx "BlackHole 2ch" <<<"$devices" && grep -qx "BlackHole 16ch" <<<"$devices"; then found=1; break; fi
  sleep 1
done
if [ "${found:-0}" != 1 ]; then
  echo "audio.sh: BlackHole 2ch and 16ch not listed 30s after restarting coreaudiod; devices seen:" >&2
  echo "$devices" >&2
  exit 1
fi
SwitchAudioSource -t input -s "BlackHole 2ch"
SwitchAudioSource -t output -s "BlackHole 16ch"
SwitchAudioSource -t system -s "BlackHole 16ch"
SwitchAudioSource -c -t input; SwitchAudioSource -c -t output
