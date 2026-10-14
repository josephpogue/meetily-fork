# Changes from upstream Meetily

Every way this fork differs from upstream Meetily ([Zackriya-Solutions/meetily](https://github.com/Zackriya-Solutions/meetily)), built or planned.
The goal, stories and requirements are in `CLAUDE.md`. Update this file in the same PR as any change from upstream.

**Upstream base:** v0.4.1 (merged in PR #3). Upstream's newest release on 2026-10-13 is also v0.4.1.

## Features

| Feature | What it does | Status |
|---|---|---|
| Assistant panel | A panel beside the live transcript that holds the assistant's answer cards, with an on/off switch and a collapse toggle. | Built (PR #1) |
| Answer help | When someone on the call asks Joseph a question, the assistant suggests an answer from the transcript and the meeting brief. A quick answer appears first, then a fuller check settles it and marks any correction. | Built (PR #1) |
| Answer modes | Manual never volunteers, Gated (default) answers when Joseph is named or after a pause, Continuous answers any question from the room. Opt-M cycles them. | Built (PR #1) |
| Ask | Joseph types a question, or speaks one with opt-A, and gets an answer that uses the call so far. | Built (PR #1) |
| Explain and Catch-up | Opt-E explains what was just said; opt-C summarises the call so far. | Built (PR #1) |
| Meeting note | After recording stops, the assistant drafts a note and a full transcript. They go to the vault only when Save is pressed. Meetily's own meeting save still happens. | Built (PR #1) |
| Assistant settings | A new Assistant tab in Settings: models, answer mode, Joseph's names, vault folder, and a Test claude button. | Built (PR #1) |
| You / Them labels | Each live transcript line is marked as Joseph's mic (You) or the call audio (Them). | Built (PR #1): live view only, not in the saved meeting |
| Meeting brief | An optional typed brief that tells the assistant the point of the call. It stays visible for the whole call and changes only when Joseph presses Save (or Cmd+Enter); an Unsaved marker shows edits not yet sent. | Built (PR #1); Save button and mid-call edits in PR #6 |
| Brief editable during the call | A saved change to the brief mid-call reaches the answer lanes and Ask next on their next turn. | Built (PR #6) |
| Point of the call from the conversation | The assistant works out what the call is for from what is said, alongside the brief, and works from that when there is no brief. | Built (PR #6) |
| Gap questions | Ask next: a live list of up to five questions for the gaps in what has been described (missing, vague or contradictory details), each with the gap it fills, plus up to two For-you questions when the brief is unclear. Not tuned to any kind of call. The assistant judges the gaps itself and removes a question once the call answers it. Refreshes after each utterance, on the Refresh button and with Opt-Q; questions can be dismissed; open ones go into the meeting note. | Built (PR #6) |
| Stay on screen until the note is done | With the assistant on, the app waits for Save or Discard on the note before it opens the meeting details. | Built (PR #1) |
| meetily-fork name | The window, dock, tray and notifications say meetily-fork, so it can sit next to an installed upstream Meetily. | Built (PR #4) |
| In-app updater off | Meetily's update prompt no longer appears, because it would install plain Meetily over the fork. Upstream updates arrive by merging. | Built (PR #4) |
| See the Zoom window | While recording, the assistant grabs the Zoom meeting window about once a second, keeps a picture only when it changes (the /watch scene rule), saves kept pictures full-size in the meeting folder's `zoom-frames/`, and sends new ones to Claude inside each turn: the newest one with a suggested answer or Explain, all new ones with Ask, Catch-up and the fuller check, and the call's slides with the note. A Settings switch, on by default, turns it off; a missing Screen Recording permission shows a line with a button to System Settings. A turn that fails with pictures is retried once without them. | Built (PR #9) |
| Test Mac add-on | Builds made by testenv/build-test-app.sh include an embedded WebDriver server so the user-mock agent can use the app in a throwaway test Mac. Builds Joseph installs never include it: the test build lands in `target/testenv/`, never in the release bundle path, and makes no DMG. | Built (PR #11) |
| Named icon buttons | The round record, stop, pause, resume and play buttons, the sidebar's record button, and the Ask box's send arrow and mic have accessible names (Start recording, Stop recording, Pause recording, Resume recording, Play recording, Recording in progress, Send question, Speak a question, Stop and submit), so the user-mock agent and screen readers can find them. The look is unchanged. | Built (PR #11) |

## Upstream bug fixes

Bugs in upstream code fixed here. Each is a candidate to send upstream.

- **Recording arguments dropped (PR #1):** `recordingService.ts` sent snake_case keys that Tauri v2 ignores, so device and meeting names were lost.
- **Recording auto-started after relaunch (PR #1):** the `autoStartRecording` flag is now a short-lived timestamp in `SidebarProvider.tsx`.
- **RUST_LOG ignored (PR #1):** `main.rs` now sets a default only when the variable is unset.
- **Built-in mic test failed on some hardware (PR #5):** `device_detection.rs` no longer depends on the host machine.
- **Release app needed Homebrew onnxruntime (PR #10):** on a build Mac with Homebrew's onnxruntime, `ort-sys` linked that dylib by absolute path, so the app died at launch on a Mac without Homebrew. `LIBONNXRUNTIME_NO_PKG_CONFIG` in `.cargo/config.toml` makes it link pyke's prebuilt static build instead, so nothing extra ships in the bundle. Upstream has the same latent bug.

## Upstream files touched

The files most likely to conflict when merging a new upstream release.

- **Assistant (PR #1):** `Cargo.toml`, `audio/pipeline.rs`, `audio/vad.rs`, `audio/recording_state.rs`, `audio/transcription/worker.rs`, `audio/incremental_saver.rs`, `audio/mod.rs`, `lib.rs`, `main.rs`, `app/layout.tsx`, `app/page.tsx`, `settings/page.tsx`, `VirtualizedTranscriptView.tsx`, `TranscriptPanel.tsx`, `TranscriptContext.tsx`, `SidebarProvider.tsx`, `useRecordingStart.ts`, `useRecordingStop.ts`, `recordingService.ts`, `types/index.ts`.
- **Assistant's own code (PR #1):** `src-tauri/src/assistant/`, `components/AssistantPanel/`, `AssistantSettings.tsx`, `AssistantContext.tsx`, `assistantService.ts`, and the `assistant_settings` database migration.
- **Name (PR #4):** `tauri.conf.json`, `tray.rs`, `notifications/commands.rs`, `notifications/types.rs`, `app/metadata.ts(x)`, `Logo.tsx`.
- **Updater off (PR #4):** `services/updateService.ts`.
- **Mic test (PR #5):** `audio/device_detection.rs`.
- **Ask next (PR #6):** `lib.rs` (Opt-Q hotkey and two commands), `types/index.ts`. Its own code is under `src-tauri/src/assistant/` and `components/AssistantPanel/`, plus `AssistantContext.tsx`, `assistantService.ts` and `lib/ask-next.ts`.
- **See the Zoom window (PR #9):** `lib.rs` (two commands), `types/index.ts`, `database/manager.rs` (migrations tolerate a newer database, so an older fork build still opens it), `Cargo.toml` and `Cargo.lock` (`base64`). Its own code is `src-tauri/src/assistant/vision/`, the assistant lanes and prompts, `AssistantSettings.tsx`, `assistantService.ts`, and the `see_zoom_window` migration. `scripts/xcodebuild-shim/` builds cidre without full Xcode.
- **Static onnxruntime (PR #10):** `frontend/src-tauri/.cargo/config.toml`, and a macOS note in `CLAUDE.md`.
- **Test add-on (PR #11):** `Cargo.toml` (feature `testenv`) and `Cargo.lock`, `lib.rs` (one cfg block).
- **Named icon buttons (PR #11):** `components/RecordingControls.tsx` and `components/Sidebar/index.tsx` (an `aria-label` on each icon-only record button), and `components/AssistantPanel/AskBox.tsx` ("Send question" on the send arrow, "Speak a question" on the mic).
- **Docs (PR #2, #7):** `CLAUDE.md` trimmed, with reference sections moved into `docs/architecture.md` and `docs/common-tasks.md`.
