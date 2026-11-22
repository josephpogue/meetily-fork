# Floating meeting prompt: job brief

Design: `docs/superpowers/specs/2026-11-21-floating-meeting-prompt-design.md` (read it first).
Entry points: `frontend/src-tauri/src/meeting_detect/panel.rs` (Rust) and `frontend/src/components/MeetingPrompt/FloatingMeetingPrompt.tsx` with the three new `meetingDetectService` methods (frontend).

## Goal

The "Zoom meeting started" / "Teams meeting started" card (Record, Dismiss) and the "Zoom meeting ended" / "Teams meeting ended" card (Stop recording, Keep recording) show in a small floating window of their own over whatever app is in front, on every desktop and over a full-screen call, without taking keyboard focus. The card inside the main window is no longer drawn. Detection rules, the macOS notification and what each button does stay as they are.

## Done when

- With a browser in front and the main window behind it or closed to the tray, a call start shows the floating window at the top center of the screen the pointer is on, below the menu bar.
- Typing in the front app keeps going to that app after the window appears.
- Each button does what the in-app card button did, and the window hides when it has no cards left.
- The main window never shows a meeting card of its own.
- Every locked test passes, and the existing suites (`bun test` in `frontend/`, `cargo test` for `meeting_detect`) still pass.

## Entry points

Rust, `meeting_detect/panel.rs`:

- `cards(rules: &Rules) -> Vec<Card>`: the cards the floating window shows now. `Card { app: CallAppId, kind: CardKind, title: Option<String> }`, `CardKind::Started | CardKind::Ended`. Started cards come from `rules.prompted()`, Ended cards from `rules.ended()` (title `None`), Zoom before Teams. An empty list means the window is hidden.
- `place(visible: Rect, width: f64, height: f64) -> (f64, f64)`: the window's top-left point. `visible` is the screen's visible area (below the menu bar, above the Dock) in the same coordinates as the result, top-left origin. The window is centered horizontally in `visible`, with its top `GAP_TOP` (a pub const, in points) below `visible.y`. When the window is wider than `visible`, x is `visible.x`.

Frontend:

- `meetingDetectService.cards(): Promise<PanelCard[]>` invokes `meeting_prompt_cards` (no args).
- `meetingDetectService.onCards(cb)` listens to the `meeting-prompt-cards` event, whose payload is the full current `PanelCard[]` (not a diff).
- `meetingDetectService.record(app)` invokes `meeting_detect_record` with `{ app }`.
- `PanelCard` in `src/types`: `{ app: CallApp; label: string; kind: 'started' | 'ended'; title: string | null }`.
- `FloatingMeetingPrompt` (no props): on mount it renders the list from `cards()`, then replaces it with each `meeting-prompt-cards` payload. It renders nothing for an empty list.

## States and readers

| State (Rules) | Floating window | Main window | Notification |
|---|---|---|---|
| Idle, no call | hidden, no cards | no card | none |
| Zoom call prompted, not recording | one Started card "Zoom meeting started", title line if known, Record and Dismiss | no card | as today |
| Zoom and Teams both prompted | two Started cards, Zoom first | no card | two, as today |
| Prompted call ends before any click (Cleared) | that card goes; window hides if none left | no card | withdrawn as today |
| Recording started any way (Record, main button, tray) | every Started card goes | no card | as today |
| Dismissed | that card goes; no card for 30 min for that app | no card | withdrawn as today |
| Recording, watched call ended (Ended) | one Ended card "Zoom meeting ended", Stop recording and Keep recording | no card | as today |
| Ended, Keep pressed | that card goes | no card | withdrawn |
| Ended, Stop pressed | card goes, recording stops by the tray's Stop path | no card | withdrawn |
| Ended, recording stopped another way, or call rejoined (EndedCleared) | card goes | no card | withdrawn |
| Onboarding showing in the main window | cards show as above; Record only brings the main window forward and starts nothing (same as the notification's Record today) | no card | as today |
| Floating window page loads after a prompt already exists | shows that prompt (from `cards()`) | no card | as today |

Readers of the meeting-detect state today: `meeting_detect/mod.rs` (`apply`, `on_banner`, the five commands), `MeetingPrompt.tsx` (cards plus the `meeting-detect-record` start flow), the notification code in `notify.rs`. After this job `MeetingPrompt` keeps its start flow (listening to `meeting-detect-record` and starting the recording on the home page) but draws no cards.

## Ways a button press can end, and what must still happen

| Press | What must happen |
|---|---|
| Record while that app is prompted and nothing records | main window comes forward, the recording of that app starts (same path as the notification's Record), card goes |
| Record on a stale card (call gone, or recording already on) | main window comes forward, nothing starts, no error shown |
| Dismiss | `meeting_detect_dismiss`; card goes |
| Keep recording | `meeting_detect_keep`; card goes |
| Stop recording | `meeting_detect_stop`; card goes |
| A command fails | the card stays until the next `meeting-prompt-cards` payload; the error is logged, nothing else breaks |

## Worked examples

- `cards` with Zoom prompted (title "Weekly sync") and Teams prompted (no title), not recording: `[Started Zoom "Weekly sync", Started Teams None]`.
- `cards` after `recording_started(Some(Zoom))` on that state: `[]`.
- `cards` while recording with Zoom ended: `[Ended Zoom None]`.
- `cards` after `keep(Zoom)`: `[]`.
- `place(Rect { x: 0, y: 25, w: 1512, h: 920 }, 360, 120)`: `(576, 25 + GAP_TOP)`.
- `place(Rect { x: 1512, y: 0, w: 1920, h: 1055 }, 360, 120)` (second screen to the right): `(2292, GAP_TOP)`.
- `place(Rect { x: 0, y: 25, w: 300, h: 600 }, 360, 120)`: `(0, 25 + GAP_TOP)`.
- `FloatingMeetingPrompt`, `cards()` returns one Started Zoom card: shows "Zoom meeting started" with Record and Dismiss; pressing Record invokes `meeting_detect_record` with `{ app: 'zoom' }`.
- Then a `meeting-prompt-cards` payload `[]` arrives: renders nothing.
- Ended Teams card: shows "Teams meeting ended" with Stop recording and Keep recording; Stop invokes `meeting_detect_stop` `{ app: 'teams' }`, Keep invokes `meeting_detect_keep` `{ app: 'teams' }`.

## Existing tests that encode the old rule

`frontend/tests/components/meeting-prompt.test.tsx` checks that `MeetingPrompt` draws cards in the main window. Update it: `MeetingPrompt` draws no cards, and its `meeting-detect-record` start flow still works as those tests check today.

## Out of scope

- Detection rules in `rules.rs`, the notification in `notify.rs`, auto start or stop, start at login, new call apps.

## Answers to the tester's questions

- **Rust test file:** `frontend/src-tauri/src/meeting_detect/tests/panel.rs`. `panel.rs` already declares it (`#[cfg(test)] #[path = "tests/panel.rs"] mod tests;`), so it is a child module of `panel`: use `super::*` for `cards`, `place`, `Rect`, `Card`, `CardKind`, `GAP_TOP`, and `crate::meeting_detect::rules::{Rules, Tick, AppSignals}` plus `crate::audio::call_apps::CallAppId`.
- **Rules, as a black box:** `Rules::new()`; `tick(&Tick) -> Vec<Event>`; `recording_started(Option<CallAppId>) -> Vec<Event>`; `keep(CallAppId) -> Option<Event>`; `dismiss(CallAppId, now: u64) -> Option<Event>`; `prompted()`; `ended()`.
  - `Tick { now: u64, recording: bool, recording_app: Option<CallAppId>, zoom: AppSignals, teams: AppSignals }`, all fields public, `Default`.
  - `AppSignals { mic: bool, meeting_window: bool, plain_window: bool, frontmost: bool, title: Option<String> }`, all public, `Default`.
  - Started prompt: one tick `now: 2, recording: false`, with that app's `mic: true, meeting_window: true` (and `title: Some(..)` for a title) prompts that app at once.
  - Cleared: after a prompt, feed non-recording ticks with all-default signals every 2 s until `prompted()` is empty (loop with a bound of 30; do not depend on the exact count).
  - Ended: tick `now: 0, recording: true, recording_app: None` with Zoom `mic: true, meeting_window: true`, then recording ticks with all-default signals every 2 s until `ended()` is not empty (same bounded loop).
  - EndedCleared by a stopped recording: after Ended, one non-recording tick with default signals.
- **Labels:** the visible button text is exactly "Record", "Dismiss", "Stop recording", "Keep recording". Each card is a `section` with `aria-label` "<Label> meeting started" or "<Label> meeting ended", as the old in-app card had.
- **When a card goes:** on a successful press, the card goes when the command resolves, without waiting for the next payload. On a failed press it stays until the next payload.
- **Record on a stale card, and the onboarding row:** both are checked by hand on the running app, not by unit tests.
