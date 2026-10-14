# Test environment design: fresh test Macs and the user-mock agent

Date: 2026-10-14. Approved in conversation the same day; this file is the written spec for review.
Research behind it: `~/brain/wiki/_sources/research/2026-10-14-software-factory-test-environments-homelab-macos.md`.

## Goal

Before a Meetily PR opens, an agent that never saw the code installs the new build on a fresh, clean Mac, uses it the way Joseph would, and reports what broke.
The fresh Mac is a throwaway virtual machine on Joseph's Mac, created in seconds from a prepared image and deleted after the run.
Dev-lead checks the code; the new user-mock agent checks the app.
The tool that creates the test Macs is shared, so other Mac apps and later the Rental Ledger lab environment use the same commands.

## How a run looks

1. A developer finishes a slice and builds the app with the test add-on (below).
2. Dev-lead reviews the code, runs the tests and the fresh code review, exactly as today.
3. Dev-lead starts user-mock with Joseph's request, each slice's "done when" check, the path to the built app, and the repo name.
4. User-mock runs `testenv up meetily-fork --app <path>`, which clones a fresh test Mac from the prepared image, starts it with no access to the real microphone, and copies the app in.
5. User-mock walks the new feature's checks and then the app's user stories, taking a screenshot at each step.
6. User-mock runs `testenv down <id>` and returns its report.
7. A failed step goes back to the developer who built that part; after the fix, user-mock runs again on the new build.
8. The PR opens only when the code checks and the user-mock run both pass.

## Part 1: the `testenv` tool

A new private repo, `josephpogue/testenv`, at `~/brain/departments/personal/projects/testenv`, with a row in the routing registry.
It is one shell command over Tart 2.41.0 (already installed at `~/.local/opt/tart-2.41.0/`), linked into `~/.local/bin/testenv`.

| Command | What it does |
|---|---|
| `testenv bake <app>` | Clones the Cirrus `macos-sequoia-base` image, runs the app repo's `testenv/golden.sh` inside it, and saves the result as `<app>-golden`, tagged with a hash of the recipe |
| `testenv up <app> --app <path>` | Clones `<app>-golden` into `run-<app>-<id>`, starts it headless with `--no-audio`, waits until it answers, copies the app into `/Applications`, prints the id |
| `testenv exec <id> -- <cmd>` | Runs a command in the test Mac |
| `testenv shot <id> <file>` | Saves a screenshot of the test Mac's screen |
| `testenv play <id> mic\|call <clip>` | Plays a clip into the fake microphone (BlackHole) or as call audio through the speakers |
| `testenv collect <id> <dir>` | Copies out the app's logs and any crash reports |
| `testenv down <id>` | Stops and deletes the test Mac |
| `testenv ls` | Lists test Macs and their age |
| `testenv reap` | Deletes `run-*` test Macs older than two hours |

Rules the tool enforces:
- **Two at most:** Apple's license allows two macOS virtual machines at once. `up` waits up to ten minutes for a free slot, then fails.
- **Stale image:** `up` refuses to start when the app repo's `golden.sh` no longer matches the hash on `<app>-golden`, and says to run `bake`.
- **Two kinds of failure:** exit code 2 means the test Mac itself failed (no slot, did not start, copy failed). Any other non-zero code is passed through from the command it ran. User-mock reports exit code 2 as an environment failure, never as an app bug.
- **Cleanup job:** a launchd job runs `testenv reap` every 15 minutes and reports through `runlog`.

Run output (screenshots, logs, the report) goes to `~/.cache/testenv/runs/<id>/`, which survives `down`.

## Part 2: Meetily's test setup

A `testenv/` folder in the Meetily repo:

- **`golden.sh`:** the recipe for the prepared image.
  - Installs BlackHole 2ch, sox and switchaudio-osx, and makes BlackHole the default input, so the app hears a fake microphone.
  - Copies the Parakeet transcription model (640 MB) from Joseph's Mac into the guest's app data, so a run never downloads it.
  - Builds and installs the Zoom stand-in (below).
  - Writes the permission rows before the app has ever launched: microphone and system-audio capture for `com.meetily.ai` in the user permission database, screen recording in the system one. The base image already has SIP off, which this needs.
  - Leaves the rest of the app data empty, so every run starts as a first launch.
- **Zoom stand-in:** a small Swift app with Zoom's bundle id `us.zoom.xos` that shows a window titled "Zoom Meeting" and changes its slide every few seconds. Meetily's capture picks windows by that bundle id and title (`assistant/vision/capture.rs`), so this exercises the real capture path without a Zoom account or network.
- **Test clips:** `you.wav` and `them.wav`, two short sentences made with `say`, each with distinct words to search for in the transcript.
- **Test add-on:** `tauri-plugin-wdio-webdriver` behind a cargo feature named `testenv`. The test build is a normal release build with that feature on (`pnpm tauri build -- --features testenv`), so packaging bugs like the onnxruntime crash still show up. The release build Joseph installs never has the feature.
- **`ui.mjs`:** a small WebdriverIO script user-mock calls to use the app: `ui.mjs click "<text>"`, `ui.mjs type "<field>" "<text>"`, `ui.mjs read`, `ui.mjs wait "<text>"`. It talks to the add-on inside the test Mac.
- **`README.md`:** how user-mock drives this app, for user-mock to read. It holds no code.

The first three checks every Meetily run makes:
1. **Opens on a clean Mac:** the app launches and reaches the first screen, with no crash report and no permission prompt.
2. **You and Them:** during a recording, `you.wav` played into the fake microphone shows up as a You line, and `them.wav` played as call audio shows up as a Them line, each with its words.
3. **Zoom window seen:** with the stand-in open during a recording, pictures appear in the meeting folder's `zoom-frames/`.

Features that call Claude (answers, gap questions, the note) are not tested in the test Mac, because that needs Joseph's Claude login inside it and the app under test could read it.

## Part 3: the user-mock agent

`~/.claude/agents/user-mock.md`, Sonnet, tools Bash and Read.

- **Gets:** Joseph's request, the "done when" checks, the built app's path, the repo name.
- **Reads:** the repo's `CLAUDE.md` Goal and Stories sections and `testenv/README.md`. It never reads source code or diffs, so it judges the app the way Joseph would.
- **Walks:** the new feature's checks first, then every user story it can reach without Claude, then the three standing checks.
- **Returns:** a table with one row per step: pass, fail or blocked, what it saw, and a screenshot path. Environment failures are listed separately. Anything that looked broken outside the steps gets its own line.
- **Never:** fixes code, opens a PR, or runs a third test Mac.
- **Web apps:** until a lab environment exists, user-mock starts the app with the repo's run recipe against its test database and drives it with Playwright, with the same report.
- It also works as a main thread, so Joseph can run `claude --agent user-mock` and ask it to walk an app.

## Part 4: dev-lead changes

In `~/.claude/agents/dev-lead.md`, section 7 ("Final check on the combined build"):
- Step 2 changes from dev-lead starting the app and taking screenshots to dev-lead starting user-mock and waiting for its report.
- A failed step goes to the slice's developer with the step, what user-mock saw, and the screenshot; user-mock runs again on the fixed build.
- An environment failure goes to Joseph, not to a developer.
- The PR body lists user-mock's steps and results.

In `~/.claude/CLAUDE.md`, the "Code goes to dev-lead" section gains one line: dev-lead hands the walk-through of the running app to `user-mock`, and Joseph can ask for a user-mock walk-through directly.

## Part 5: proving it works

1. Build the old Meetily (main before PR 10) with the test add-on and run user-mock on it. It must report "Opens on a clean Mac" as failed, with the onnxruntime crash report.
2. Build current main with the test add-on and run user-mock. All three checks must pass.
3. Leave a test Mac running, wait past two hours, and confirm the cleanup job deletes it and its Mission Control run is sealed.

## Out of scope for this spec

- The Rental Ledger lab environment, which gets its own spec next and adds a lab backend to the same `testenv` commands.
- Telling individual voices apart, which is a later Meetily feature.

## Also in this PR

The macOS note in `CLAUDE.md` about building without Xcode is now wrong: full Xcode 26.3 was installed on 2026-10-14 and `cidre` builds with it. The note is rewritten to say so.
