# Test Mac guide for user-mock (Meetily)

How user-mock uses Meetily in a throwaway test Mac. Walk the steps in this order. Every step gets a screenshot, and you `Read` the shot before you call the step passed.

## 1. Build

The build is not yours. You are handed the path of the built app. It comes from `bash testenv/build-test-app.sh`, whose last line is the app path. User-mock receives that path and does not build.

## 2. Start

- Start a fresh test Mac: `testenv up meetily-fork --app <app path> --recipe <repo>/testenv`. It prints the test Mac's id. Write that id down and type it literally in place of `<id>` in every later command, because shell variables do not carry over between your Bash calls.
- Give the `testenv up` Bash call a 900000 ms timeout. It can wait up to ten minutes for a free slot and two more for the test Mac to start, so the default two-minute timeout would cut it off and look like a failure.
- Open the app in it: `testenv exec <id> -- bash /Users/admin/launch-app.sh`
- Commands run inside the test Mac use absolute paths under `/Users/admin/`, never `~`, because your own shell expands `~` to the wrong home.
- The app takes a moment to start. Poll `testenv exec <id> -- node /Users/admin/testenv-ui/ui.mjs read` for up to a minute until it prints the first screen. Only after that does "the app does not answer" count as a failed step.

## 3. Drive

- Use the app with `testenv exec <id> -- node /Users/admin/testenv-ui/ui.mjs wait|click|type|read ...`
  - `wait "<text>"` waits until that text is on screen (30 seconds by default). `wait "<text>" 60` waits up to 60 seconds, for slow screens.
  - `click "<text>"` presses the element whose whole text, `aria-label` or placeholder is that text. Case and extra spaces do not matter, but part of a label does not count: `click "Start"` does not press "Get Started". Copy the label from `read`. A button that only shows while its row is hovered (the to-do and Ask dismiss X buttons) is pressed as it is, because this webview does not show hover from a script: `click "Dismiss to-do: <text>"` works, and prints `pressed as a hover-revealed control` when it took that path. That happens only when opacity 0 alone hides a real, enabled button or link that sits in view and is not covered. Anything else hidden is "nothing to click" (exit 1). There is no `hover` verb.
  - `type "<field>" "<text>"` types into the field whose whole label or placeholder is that text, matched the same way as `click`.
  - `wait` tries the same whole-label match first, then any text that contains yours.
  - `read` prints all the text on screen, then the labels of round icon buttons that have no text, one per line as `button: <label>`. Those lines are the buttons that look unlabeled on a screenshot but that `click` can press.
  - Start a recording with `click "Start recording"` and stop it with `click "Stop recording"`.
- If a `ui.mjs` verb fails (the text never appears, or the app does not answer), that is a failed step, not an environment failure. Screenshot it and report what is on screen. Exit 1 means the text was not found or the app answered too slowly ("app did not answer within 15s"), exit 3 means nothing answered the WebDriver port at all (refused, reset or closed), and exit 64 means you called `ui.mjs` wrongly: fix the command, it is not an app failure.
- After every step, take a screenshot: `testenv shot <id> <step-name>`

## 4. Audio

- Your voice: `testenv play <id> mic /Users/admin/clips/you.wav`. The line must contain "giraffe" and "pancakes" and be labelled You. Wait on `giraffe`.
- The other side of the call: `testenv play <id> call /Users/admin/clips/them.wav`. The line must contain "Submarine" and "harbor" and be labelled Them. Wait on `Submarine`.
- Play a clip only while a recording is running, and play each clip once. You can play them back to back in either order. Then give the line about 10 seconds to show up before you call it missing.
- How the audio gets in: the fake microphone is a 16-channel virtual device. `play mic` writes the clip on its first two channels and an inverted copy on the next two, so the app's microphone hears it, while the app's system-audio capture, which hears every program's output, gets silence. `play call` plays the clip on the 2-channel fake speakers at full level, which the app hears as Them. You do not need to do anything about this; it is why one play of each clip is enough.

## 5. Zoom

- Open the stand-in Zoom window: `testenv exec <id> -- open -a "Zoom Stand-in"`
- Opening it puts Zoom in front. Bring Meetily back before you click anything: `testenv exec <id> -- open -a meetily-fork`
- "See the Zoom or Teams window" is on by default, so you do not need to turn it on in the app's settings.
- During a recording, run `testenv exec <id> -- bash -c 'ls /Users/admin/Movies/meetily-recordings/*/zoom-frames/ 2>/dev/null | head'`. It must list picture files.
- The app saves the frames in `zoom-frames` inside the meeting folder (default `~/Movies/meetily-recordings/<meeting>/`). See `frames_dirs` in `frontend/src-tauri/src/assistant/core.rs`. You do not read that file; this is only where the pictures land.

## 5b. Teams

- Open the stand-in Teams window: `testenv exec <id> -- open -a "Teams Stand-in"`
- Opening it puts Teams in front. Bring Meetily back before you click anything: `testenv exec <id> -- open -a meetily-fork`
- "See the Zoom or Teams window" is on by default, so you do not need to turn it on in the app's settings.
- During a recording, run `testenv exec <id> -- bash -c 'ls /Users/admin/Movies/meetily-recordings/*/teams-frames/ 2>/dev/null | head'`. It must list `teams-*.jpg` files.
- The app saves the frames in `teams-frames` inside the meeting folder, next to `zoom-frames`. With only the Teams stand-in open, `zoom-frames` does not exist. If both stand-ins are open, the Zoom one is captured and `teams-frames` stays empty or missing.

## 5c. Meeting detection and one app's audio

Each stand-in is a call in progress for as long as it runs: its meeting window, its own process holding the microphone (the fake mic, BlackHole 16ch, data thrown away), and its own clip looping on the fake speakers (BlackHole 2ch). Zoom Stand-in says "The purple kangaroo plays the violin." and Teams Stand-in says "The green tractor drives to Denver.", each followed by about 4 s of silence, in different voices from `you.wav` and `them.wav`. The image already allows the microphone for both and allows Meetily's notifications as banners, so no permission prompt appears.

1. With Meetily open and no recording running, open both stand-ins: `testenv exec <id> -- open -a "Zoom Stand-in"` and `testenv exec <id> -- open -a "Teams Stand-in"`.
2. Bring Meetily back: `testenv exec <id> -- open -a meetily-fork`.
3. Within about 15 s take `testenv shot <id> banners` (and a second one a few seconds later if a banner is hidden). Expect two banners, "Zoom meeting started" and "Teams meeting started". Banners show only in a `testenv shot` screenshot, never in `ui.mjs read`.
4. `ui.mjs read` shows the in-app card for each app. Record from the Teams card (button "Record Teams meeting").
5. Play `them.wav` as call audio once (`testenv play <id> call /Users/admin/clips/them.wav`) and wait about 20 s, then stop the recording.
6. The Them lines must contain "tractor" or "Denver", and must not contain "kangaroo", "violin" or "submarine". `afplay` is a third process, so a Teams-only recording does not hear it, and the Zoom stand-in's clip is another app's sound.
7. A manual start with Call audio set to "All apps" hears all three: Them lines with kangaroo, tractor and submarine words. With a stand-in open, any all-apps recording also hears its clip as Them, so the standing You and Them check (section 6) still passes but the Them text has extra words.
8. `--home` is the app's home window, not a call: `testenv exec <id> -- open -a "Zoom Stand-in" --args --home` shows a window titled "Zoom Workplace", and the Teams one a window titled "Chat | Acme | Microsoft Teams". Neither opens the microphone or makes sound, and neither gives a notification or a card.
9. Quit a stand-in to end its call: `testenv exec <id> -- osascript -e 'quit app "Zoom Stand-in"'`.

## 6. The three standing checks

Every Meetily run makes these first three checks:
1. **Opens on a clean Mac:** the app launches and reaches the first screen, with no crash report and no permission prompt. For the crash report, run `testenv exec <id> -- ls /Users/admin/Library/Logs/DiagnosticReports`: it must list no file whose name starts with `meetily`.
2. **You and Them:** during a recording, `you.wav` played into the fake microphone shows up as a You line, and `them.wav` played as call audio shows up as a Them line, each with its words.
3. **Zoom window seen:** with the stand-in open during a recording, pictures appear in the meeting folder's `zoom-frames/`.

The test Mac has a Claude stand-in at `/Users/admin/.local/bin/claude`, the way it has a Zoom stand-in. It lets the assistant session open during a recording, but every answer it gives is canned. It makes a one-line card for Joseph's own kinds: a question you type in the Ask box (`type "Ask a question" "<question>"`, then `click "Send question"`) gets one fixed reply card, and Explain and Catch up get a fixed one-line card each. Nothing else makes a card, and the note is a fixed placeholder. The Ask lane, after Refresh questions, shows a quick list (one question, "What storage do you run today?", and one for you, "Is pricing in scope for this call?"), then, about 4 seconds later, a checked list that adds a second question ("When does your support contract end?"). Once the call clip has said "submarine" ("The orange submarine visits Tuesday harbor."), the next Ask lane turn answers the storage question: it moves to the answered list with the answer "The orange submarine visits on Tuesday at the harbor." (how: on the call) and stays off the open list. The To do list rides on the same turns. Once the mic clip has said "pancakes" ("The blue giraffe orders seven pancakes."), the next Ask lane turn adds a to-do for you, "Order seven pancakes for the blue giraffe" (due Friday). Once the call clip has said "submarine", the next turn adds a to-do for them, "Send the Tuesday harbor schedule" (Dana, due Tuesday), and moves the pancakes to-do to done (how: "said on the call it is done"). Answer the For-you question yourself (type the answer, then click its "Save answer" check button) to see it move to the answered list as "you". The stand-in keeps no state, so each reply lists the same questions and to-dos, and the app keeps an answered question off the list and a to-do from being added twice. Assistant answers are not tested in the test Mac, because real ones need Joseph's Claude login inside it and the app under test could read it. Skip the user stories that judge what Claude says.

## 7. Finish

Always, even after a failure: `testenv collect <id>`, then `testenv down <id>`.

## 8. Environment failures

Exit code 2 from any testenv command is an environment failure. Stop, collect and tear down if you can, and report RESULT: ENVIRONMENT.
