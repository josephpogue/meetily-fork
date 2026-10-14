#!/bin/bash
# Two separate virtual devices, so call audio never leaks into the microphone:
#   BlackHole 16ch = the fake microphone (default input). `testenv play mic` writes the clip on channels 1-2 and its
#                    inverse on 3-4. Meetily's system-audio capture is a global process tap that hears every process's
#                    output, sox's included; its mono mixdown of the 16 channels cancels to silence, while Meetily's mic
#                    reads channels 1-2 (audio_to_mono uses the first two of a >2-channel device) and hears the clip as You.
#   BlackHole 2ch  = the fake speakers (default output). `testenv play call` plays there with afplay, which fills both
#                    channels, so the tap hears it at full level as Them. On the 16ch device afplay filled one channel
#                    of 16 and the tap heard it 24 dB down, too quiet for the speech detector to catch every time.
# The VM runs with --no-audio, so these are its only audio devices.
set -euo pipefail
eval "$(/opt/homebrew/bin/brew shellenv)"
brew install --cask blackhole-2ch blackhole-16ch
brew install sox switchaudio-osx
# The cask says "reboot to take effect", but restarting coreaudiod loads the new drivers (SIP is off in this image).
sudo killall coreaudiod || true
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
SwitchAudioSource -t input -s "BlackHole 16ch"
SwitchAudioSource -t output -s "BlackHole 2ch"
SwitchAudioSource -t system -s "BlackHole 2ch"
SwitchAudioSource -c -t input; SwitchAudioSource -c -t output
