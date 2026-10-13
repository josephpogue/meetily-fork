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

## Upstream bug fixes

Bugs in upstream code fixed here. Each is a candidate to send upstream.

- **Recording arguments dropped (PR #1):** `recordingService.ts` sent snake_case keys that Tauri v2 ignores, so device and meeting names were lost.
- **Recording auto-started after relaunch (PR #1):** the `autoStartRecording` flag is now a short-lived timestamp in `SidebarProvider.tsx`.
- **RUST_LOG ignored (PR #1):** `main.rs` now sets a default only when the variable is unset.
- **Built-in mic test failed on some hardware (PR #5):** `device_detection.rs` no longer depends on the host machine.

## Upstream files touched

The files most likely to conflict when merging a new upstream release.

- **Assistant (PR #1):** `Cargo.toml`, `audio/pipeline.rs`, `audio/vad.rs`, `audio/recording_state.rs`, `audio/transcription/worker.rs`, `audio/incremental_saver.rs`, `audio/mod.rs`, `lib.rs`, `main.rs`, `app/layout.tsx`, `app/page.tsx`, `settings/page.tsx`, `VirtualizedTranscriptView.tsx`, `TranscriptPanel.tsx`, `TranscriptContext.tsx`, `SidebarProvider.tsx`, `useRecordingStart.ts`, `useRecordingStop.ts`, `recordingService.ts`, `types/index.ts`.
- **Assistant's own code (PR #1):** `src-tauri/src/assistant/`, `components/AssistantPanel/`, `AssistantSettings.tsx`, `AssistantContext.tsx`, `assistantService.ts`, and the `assistant_settings` database migration.
- **Name (PR #4):** `tauri.conf.json`, `tray.rs`, `notifications/commands.rs`, `notifications/types.rs`, `app/metadata.ts(x)`, `Logo.tsx`.
- **Updater off (PR #4):** `services/updateService.ts`.
- **Mic test (PR #5):** `audio/device_detection.rs`.
- **Ask next (PR #6):** `lib.rs` (Opt-Q hotkey and two commands), `types/index.ts`. Its own code is under `src-tauri/src/assistant/` and `components/AssistantPanel/`, plus `AssistantContext.tsx`, `assistantService.ts` and `lib/ask-next.ts`.
- **Docs (PR #2, #7):** `CLAUDE.md` trimmed, with reference sections moved into `docs/architecture.md` and `docs/common-tasks.md`.
