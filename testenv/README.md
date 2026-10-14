# Test Mac guide for user-mock (Meetily)

How user-mock uses Meetily in a throwaway test Mac. Walk the steps in this order. Every step gets a screenshot, and you `Read` the shot before you call the step passed.

## 1. Build

The build is not yours. You are handed the path of the built app. It comes from `bash testenv/build-test-app.sh`, whose last line is the app path. User-mock receives that path and does not build.

## 2. Start

- Start a fresh test Mac: `id=$(testenv up meetily-fork --app <app path> --recipe <repo>/testenv)`
- Open the app in it: `testenv exec $id -- bash ~/launch-app.sh`

## 3. Drive

- Use the app with `testenv exec $id -- node ~/testenv-ui/ui.mjs wait|click|type|read ...`
  - `wait "<text>"` waits until that text is on screen.
  - `click "<text>"` presses the button or link with that text.
  - `type "<field>" "<text>"` types into the field with that name or placeholder.
  - `read` prints all the text on screen.
- If a `ui.mjs` verb fails (the text never appears, or the app does not answer), that is a failed step, not an environment failure. Screenshot it and report what is on screen.
- After every step, take a screenshot: `testenv shot $id <step-name>`

## 4. Audio

- Your voice: `testenv play $id mic /Users/admin/clips/you.wav`. The expected words are "blue giraffe" and "seven pancakes", and they must be labelled You.
- The other side of the call: `testenv play $id call /Users/admin/clips/them.wav`. The expected words are "orange submarine" and "Tuesday harbor", and they must be labelled Them.
- Play a clip only while a recording is running.

## 5. Zoom

- Open the stand-in Zoom window: `testenv exec $id -- open -a "Zoom Stand-in"`
- During a recording, run `testenv exec $id -- bash -c 'ls ~/Movies/meetily-recordings/*/zoom-frames/ 2>/dev/null | head'`. It must list picture files.
- The app saves the frames in `zoom-frames` inside the meeting folder (default `~/Movies/meetily-recordings/<meeting>/`). See `zoom_frames_dir` in `frontend/src-tauri/src/assistant/core.rs`. You do not read that file; this is only where the pictures land.

## 6. The three standing checks

Every Meetily run makes these first three checks:
1. **Opens on a clean Mac:** the app launches and reaches the first screen, with no crash report and no permission prompt.
2. **You and Them:** during a recording, `you.wav` played into the fake microphone shows up as a You line, and `them.wav` played as call audio shows up as a Them line, each with its words.
3. **Zoom window seen:** with the stand-in open during a recording, pictures appear in the meeting folder's `zoom-frames/`.

Features that call Claude (answers, gap questions, the note) are not tested in the test Mac, because that needs Joseph's Claude login inside it and the app under test could read it. Skip the user stories that need Claude.

## 7. Finish

Always, even after a failure: `testenv collect $id`, then `testenv down $id`.

## 8. Environment failures

Exit code 2 from any testenv command is an environment failure. Stop, collect and tear down if you can, and report RESULT: ENVIRONMENT.
