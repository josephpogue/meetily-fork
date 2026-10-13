# Fork changes

What this fork adds or changes on top of upstream Meetily ([Zackriya-Solutions/meetily](https://github.com/Zackriya-Solutions/meetily)).
Use it when merging a new upstream release: every upstream file listed here is a likely conflict.

**Upstream base:** v0.4.1 (merged in PR #3).

## Features

### Live in-meeting Claude assistant (PR #1)

A panel beside the live transcript that answers typed and spoken questions using the meeting so far.

- Volunteers an answer when someone in the room asks Joseph something.
- Explain and Catch-up buttons, plus global hotkeys: opt-A voice ask, opt-E explain, opt-C catch-up, opt-M mode.
- Drafts a meeting note at the end that is saved to the vault only when Save is pressed.
- Runs the local `claude` CLI on subscription auth, so there are no API keys.
- Assistant tab in Settings; turning the assistant off keeps upstream behaviour.
- Own code: `frontend/src-tauri/src/assistant/`, `frontend/src/components/AssistantPanel/`, `AssistantSettings.tsx`, `AssistantContext.tsx`, `assistantService.ts`.
- Upstream files touched: `audio/pipeline.rs`, `audio/vad.rs`, `audio/recording_state.rs`, `audio/transcription/worker.rs`, `audio/incremental_saver.rs`, `lib.rs`, `main.rs`, `app/layout.tsx`, `app/page.tsx`, `settings/page.tsx`, `VirtualizedTranscriptView.tsx`, `TranscriptPanel.tsx`, `TranscriptContext.tsx`, `useRecordingStart.ts`, `useRecordingStop.ts`, `types/index.ts`.

### You / Them labels on the live transcript (PR #1)

Each transcript line is tagged as the mic (You) or system audio (Them), so the assistant can tell the room from Joseph.

- Upstream files touched: `audio/pipeline.rs`, `audio/vad.rs`, `VirtualizedTranscriptView.tsx`.
- Candidate to send upstream as its own PR.

### Renamed to meetily-fork (PR #4)

The window, dock, tray and notifications say meetily-fork, so it can sit next to an installed upstream Meetily.

- Upstream files touched: `tauri.conf.json`, `tray.rs`, `notifications/commands.rs`, `notifications/types.rs`, `app/metadata.ts(x)`, `Logo.tsx`.

### Update checks turned off (PR #4)

The app no longer offers to install upstream Meetily releases over the fork; upstream updates arrive by merging instead.

- Upstream file touched: `services/updateService.ts`.

## Upstream bug fixes

Bugs found in upstream code and fixed here; each is a candidate to send upstream.

- **Recording arguments dropped (PR #1):** `recordingService.ts` sent snake_case keys that Tauri v2 ignores, so device and meeting names were lost.
- **Recording auto-started after relaunch (PR #1):** the `autoStartRecording` flag is now a short-lived timestamp in `SidebarProvider.tsx`.
- **RUST_LOG ignored (PR #1):** `main.rs` now sets a default only when the variable is unset.
- **Built-in mic test failed on some hardware (PR #5):** the test no longer depends on the host machine.

## Docs-only changes

- `CLAUDE.md` trimmed, with reference sections moved into `docs/` (PR #2).
- Building a worktree without full Xcode (PR #7).
