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
SwitchAudioSource -t input -s "BlackHole 2ch"
SwitchAudioSource -t output -s "BlackHole 16ch"
SwitchAudioSource -t system -s "BlackHole 16ch"
SwitchAudioSource -c -t input; SwitchAudioSource -c -t output
