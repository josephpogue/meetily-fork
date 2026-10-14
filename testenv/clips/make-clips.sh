#!/bin/bash
# Two known sentences for the You and Them checks. Words chosen to be easy to find in a transcript.
set -euo pipefail
out="${1:?output dir}"; mkdir -p "$out"
say -v Samantha -o "$out/you.aiff" "The blue giraffe orders seven pancakes."
say -v Daniel -o "$out/them.aiff" "The orange submarine visits Tuesday harbor."
for c in you them; do afconvert -f WAVE -d LEI16@48000 "$out/$c.aiff" "$out/$c.wav"; rm "$out/$c.aiff"; done
