# Floating meeting prompt: design

## Intent

Joseph opens the app himself but forgets to start and stop the recording. Today the "meeting started" and "meeting ended" prompts reach him only as a card inside the app window, so he misses them while a browser or the call is in front. The macOS notification that should also appear does not reach him on his Mac. FluidVoice solves the same problem with its own small window that floats over every app; he wants that.

## Goals

- When a Zoom or Teams call starts and nothing is recording, a small window appears over whatever app is in front, with "Zoom meeting started", the call title, and Record and Dismiss.
- When that call ends during a recording, the same window shows "Zoom meeting ended" with Stop recording and Keep recording.
- It shows whether the app window is in front, behind other apps, or closed to the tray.
- It shows on every desktop (Space) and over a full-screen Zoom or Teams meeting.

## Non-goals

- Starting the app at login (Joseph opens it himself).
- Starting or stopping a recording by itself: nothing stops by itself stays the rule.
- New call apps, or any change to when a call counts as started or ended.
- Fixing why the macOS notification does not show on his Mac; the notification stays as it is.

## Anti-goals

- The window takes keyboard focus away from the call when it appears (typing in the call would go to it).
- Two prompts for the same call on screen at once (the floating window and the in-app card).
- The window stays on screen after its call is gone or its question is answered.

## Approaches

1. **Floating window (picked by Joseph):** a borderless always-on-top window of our own, like FluidVoice's prompt. Shows regardless of notification settings and Focus.
2. **Fix the macOS notification only:** less code, but it still sits small in the corner and Focus modes can hide it. Rejected for that reason.

## Decided

- One floating window holds every current card, stacked Zoom first, at the top center of the screen the pointer is on, just below the menu bar.
- It stays until a button is pressed or the call state changes (call ends before Record, recording stopped another way, rejoin).
- The buttons do what the in-app card buttons do today: Dismiss silences that app for 30 minutes; Keep and Stop recording as today; Record brings the main window forward and starts recording that app, the same as the notification's Record.
- The card inside the main window is no longer drawn; the floating window is the only on-screen card. The main window still runs the recording start.
- The detection state in Rust (`meeting_detect/rules.rs`) stays the one source of truth: Rust decides which cards show, and shows or hides the window.
- The macOS notification is unchanged.

## Left to the builder

- How the window is made to float over full-screen apps and every Space without taking focus (for example a non-activating panel level).
- How the window's page skips the app chrome (sidebar, onboarding) that the root layout draws.
- The window's exact size and the gap below the menu bar.
- The look, kept to the existing card style.
