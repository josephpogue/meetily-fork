# Fork changes

What this fork adds on top of upstream Meetily ([Zackriya-Solutions/meetily](https://github.com/Zackriya-Solutions/meetily)), and the decisions behind it.
The goal is in `CLAUDE.md`.

**Upstream base:** v0.4.1 (merged in PR #3).

## Features

| Feature | What it does | Status |
|---|---|---|
| Answer help | When someone on the call asks Joseph a question, the assistant suggests an answer from the transcript and the meeting brief. | Built (PR #1) |
| Ask, Explain, Catch-up | Typed or spoken questions to the assistant, an explanation of what was just said, and a summary of the call so far. Hotkeys: opt-A voice ask, opt-E explain, opt-C catch-up, opt-M mode. | Built (PR #1) |
| Meeting note | Drafts a note at the end of the call; it is saved to the vault only when Save is pressed. | Built (PR #1) |
| You / Them labels | Each live transcript line is marked as Joseph's mic (You) or the call audio (Them). | Built (PR #1); live view only |
| Meeting brief | An optional typed brief that tells the assistant the point of the call. | Built (PR #1): typed before the call only |
| Brief editable during the call | Changes to the brief mid-call reach the assistant straight away. | Planned |
| Point of the call from the conversation | The assistant works out what the call is for from what is said, alongside the brief. | Planned |
| Gap questions | A live list of what is still unknown, given the point of the call, with a question Joseph can ask to fill each gap. The assistant judges the gaps itself and removes a question once the call answers it. | Planned |
| meetily-fork name | The window, dock, tray and notifications say meetily-fork, so it can sit next to an installed upstream Meetily. | Built (PR #4) |

## Requirements

Decisions Joseph has made for this version. Every feature follows them.

- **Subscription only:** every model call is the local `claude` CLI on his subscription; no API keys, no paid APIs, and Meetily's own summary providers stay untouched.
- **Local:** audio and transcripts stay on the Mac; the only outside path is Anthropic through the `claude` CLI.
- **Suggests, never acts:** the assistant informs and converses, and nothing is written anywhere without a Save press.
- **Text only:** no spoken output.
- **Platform:** macOS on Apple Silicon.
- **Normal window:** the app is not hidden from screen share and is never described as hidden.
- **Point of the call:** the meeting brief is optional and editable during the call, and the assistant also works the purpose out from the conversation.
- **Gaps are the assistant's judgment:** there is no preset question list or checklist; gaps come from the context it has.
- **Answered questions clear themselves:** the assistant removes a gap question once the call answers it, with no tick-off by Joseph.
- **Stay mergeable with upstream:** new code lives in its own modules, edits to upstream files stay small and are listed below, and upstream releases arrive by merging into the fork.
- **Meetily's in-app updater is off (PR #4):** its update prompt would install plain Meetily over the fork and remove these features, so updates come only through the merge above.

## Upstream files touched

The files most likely to conflict when merging a new upstream release.

- **Assistant (PR #1):** `audio/pipeline.rs`, `audio/vad.rs`, `audio/recording_state.rs`, `audio/transcription/worker.rs`, `audio/incremental_saver.rs`, `lib.rs`, `main.rs`, `app/layout.tsx`, `app/page.tsx`, `settings/page.tsx`, `VirtualizedTranscriptView.tsx`, `TranscriptPanel.tsx`, `TranscriptContext.tsx`, `useRecordingStart.ts`, `useRecordingStop.ts`, `types/index.ts`. Its own code is in `frontend/src-tauri/src/assistant/`, `frontend/src/components/AssistantPanel/`, `AssistantSettings.tsx`, `AssistantContext.tsx` and `assistantService.ts`.
- **Name (PR #4):** `tauri.conf.json`, `tray.rs`, `notifications/commands.rs`, `notifications/types.rs`, `app/metadata.ts(x)`, `Logo.tsx`.
- **Updater off (PR #4):** `services/updateService.ts`.

## Upstream bug fixes

Bugs found in upstream code and fixed here; each is a candidate to send upstream.

- **Recording arguments dropped (PR #1):** `recordingService.ts` sent snake_case keys that Tauri v2 ignores, so device and meeting names were lost.
- **Recording auto-started after relaunch (PR #1):** the `autoStartRecording` flag is now a short-lived timestamp in `SidebarProvider.tsx`.
- **RUST_LOG ignored (PR #1):** `main.rs` now sets a default only when the variable is unset.
- **Built-in mic test failed on some hardware (PR #5):** the test no longer depends on the host machine.
