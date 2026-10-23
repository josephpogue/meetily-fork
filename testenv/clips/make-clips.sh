#!/bin/bash
# Two known sentences for the You and Them checks. Words chosen to be easy to find in a transcript.
set -euo pipefail
out="${1:?output dir}"; mkdir -p "$out"
say -v Samantha -o "$out/you.aiff" "The blue giraffe orders seven pancakes."
say -v Daniel -o "$out/them.aiff" "The orange submarine visits Tuesday harbor."
for c in you them; do afconvert -f WAVE -d LEI16@48000 "$out/$c.aiff" "$out/$c.wav"; rm "$out/$c.aiff"; done
# What each call stand-in plays in a loop: its sentence, then 4 s of silence (say's slnc). Neither voice is the
# one used above, and no word repeats, so a transcript shows which app's sound a recording heard.
say -v Fred -o "$out/zoom-call.aiff" "The purple kangaroo plays the violin. [[slnc 4000]]"
say -v Ralph -o "$out/teams-call.aiff" "The green tractor drives to Denver. [[slnc 4000]]"
for c in zoom-call teams-call; do afconvert -f WAVE -d LEI16@48000 "$out/$c.aiff" "$out/$c.wav"; rm "$out/$c.aiff"; done
