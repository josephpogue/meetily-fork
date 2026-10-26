---
last_reviewed: 2026-10-23
---

# Meetily fork: goal

## Goal

Joseph's own build of Meetily: an assistant that sits beside him during a call, mainly as a consultant on discovery and knowledge-transfer calls. It keeps track of what the call has covered and tells him what to say, both answers to questions he is asked and questions he should ask. He can also chat with it on the side at any point in the call. To do that it follows the live transcript and the point of the call (from an optional meeting brief he can edit during the call and from the conversation itself); it has the same context as his Second Brain vault and, before the call starts, reads what the vault holds on the client, project and people involved, so it is ready from the first minute; it keeps a suggested answer ready when someone asks him a question, keeps a live list of the gaps it sees, each with a question he can ask to fill it, and saves every answered question with its answer; and it answers anything he types or speaks to it mid-call, using the call so far and his vault. It stays close to upstream: it adds only what his use needs, so every upstream Meetily release merges in cheaply. Every difference from upstream Meetily, built or planned, is listed in `docs/FORK_CHANGES.md`.

## User stories

- **Answer on the spot:** As Joseph, I want a suggested answer on screen when someone on the call asks me a question, so that I have it before I have to reply.
  - Acceptance: When someone on the call asks Joseph a question, the app shall show a suggested answer before he has to reply.
- **Fill the gaps:** As Joseph, I want to see what is still unknown on any call, with a question to ask for each gap, so that I can fill the gaps.
  - Acceptance: When a call is running, the app shall show what is still unknown.
  - Acceptance: When the app shows a gap, the app shall show a question to ask for it.
- **Gaps clear themselves:** As Joseph, I want an answered question to leave the open list on its own, so that I never touch it.
  - Acceptance: When a question Joseph asks or the assistant suggests is answered, on the call or by Joseph and the assistant working it out, the app shall move it off the open list and keep it with its answer, without Joseph touching it.
- **Point of the call:** As Joseph, I want to type a brief before or during the call, or none at all, so that the assistant still knows what the call is for.
  - Acceptance: When Joseph types a brief before or during the call, the app shall use it to know what the call is for.
  - Acceptance: When Joseph gives no brief, the app shall still know what the call is for.
- **Ask privately:** As Joseph, I want to type or speak a question mid-call and get an answer that uses the call so far, so that I can ask the assistant without the call hearing it.
  - Acceptance: When Joseph types or speaks a question mid-call, the app shall answer using the call so far.
- **Catch up:** As Joseph, I want one key that tells me what was just said and what it means after I lose the thread, so that I can rejoin the call.
  - Acceptance: When Joseph presses the catch-up key, the app shall show what was just said and what it means.
- **Keep track of promises:** As Joseph, I want to see what I promised and what the other side owes me, picked up from what is said without typing, so that nothing is forgotten after the call.
  - Acceptance: When a promise is made during a call, the app shall list it as a to-do for Joseph or for the other side without Joseph typing.
  - Acceptance: When the call shows a to-do finished, the app shall move it to Done by itself.
  - Acceptance: When the call ends, the app shall keep the list with the meeting.
- **Keep a record:** As Joseph, I want a note of decisions, action items, answered questions with their answers, and open questions in the vault after the call, so that the call is on record.
  - Acceptance: When Joseph saves after the call, the app shall put a note with decisions, action items, answered questions with their answers, and open questions into the vault.
  - Acceptance: When Joseph has not saved, the app shall put nothing into the vault.
- **Stay current:** As Joseph, I want a new Meetily release to merge into the fork, so that every feature in `docs/FORK_CHANGES.md` still works.
  - Acceptance: When a new Meetily release is merged into the fork, every feature in `docs/FORK_CHANGES.md` shall still work.

## Requirements

Decisions Joseph has made for this version. Every feature follows them.
- Meetily's own summary providers stay untouched by the assistant's model calls.
- The assistant sees only the Zoom or Microsoft Teams meeting window (Zoom first when both are open), and only while recording. It keeps a picture only when the window changes, by the /watch rule in `~/.claude/skills/watch/scripts/frames.py` (scene score above 0.20, then a 16x16 thumbnail more than 2.0 from the last kept picture). Kept pictures are saved full-size in the meeting folder.
- Nothing is written anywhere without a Save press.
- The meeting brief is optional and editable during the call. A new recording starts clean: nothing from the previous call (brief, cards, questions, to-dos) carries over, except a brief typed after Stop and before the new recording. The assistant also works out the point of the call from the conversation.
- Gaps are the assistant's own judgment from the context it has.
- Once a question Joseph asks or the assistant suggests is answered, either on the call or by Joseph and the assistant working it out, the assistant moves it off the open list and keeps it with its answer. Joseph does not tick questions off; removing one is his choice, by deleting it.
- The assistant panel keeps what to say and what to ask apart: a Say lane and an Ask lane, one label, icon and colour per kind of content, and nothing in the panel cut short. The panel width is Joseph's to drag, and it is remembered.
- What the assistant showed during a call is saved with the meeting automatically when the meeting is saved, with no prompt at the end. Saving it is the one write that needs no Save press, because it stays inside the app's own meeting storage and the recording folder, never the vault.
- The saved assistant items show in an Assistant tab beside the transcript, each with its call time. A copy is kept as `assistant.json` in the meeting's recording folder, and Joseph can delete a meeting's saved items.
- Joseph can delete any answer card, Ask or For-you question, answered question and to-do: one at a time with its x, or several with Select, Select all and Delete, during the call, after Stop and on a past meeting's Assistant tab. Selection checkboxes show only while selecting, so they never read as tick-off boxes. A deleted item is gone for good: from the panel, the saved history, `assistant.json`, the note draft and a filed vault note, which is rewritten and committed again. A question dismissed with its x is not saved either.
- Saving the note files it in the vault under a project Joseph confirms: the note bar suggests one from the meeting title, brief and transcript, from the `Wiki/Projects` pages, with No project as a choice. The project page gets a link to the note under `## Meetings`, so the next session in that project knows the meeting. The vault commit holds only the note, the transcript and that project page.
- The meeting brief is saved with the meeting, in its assistant record and in the vault note.
- The assistant keeps a live To do list. A You to-do is something Joseph says he will do, something the other side asks him to do unless he declines, or one he types after `todo:` in the Ask box (Opt-T starts one in the app). A Them to-do is something someone on the other side promises him, or agrees to when he asks, shown with the person's name when the call says it. The You and Them transcript labels help tell them apart.
- To-dos are picked up on the Ask next run (quick, then checked), not by a new Claude call per utterance. A to-do the call shows finished moves to Done by itself, the way answered questions move, and Joseph never ticks one off. Each to-do shows its call time and can be dismissed.
- The To do list is a one-line strip across the panel above Explain and Catch up, with the You and Them counts and the newest to-do in full. Clicking it opens a two-column drawer, You promised under Say and They owe you under Ask, over the lower part of the lanes until it is closed. The Say and Ask lanes keep their full size. To do is its own kind: rose, with a clipboard icon and no checkboxes.
- To-dos are saved with the meeting in its assistant history and `assistant.json`, and shown at the top of its Assistant tab. The vault meeting note does not change.
- New code lives in its own modules, and edits to upstream files stay small and are listed in `docs/FORK_CHANGES.md`.
- Upstream releases arrive only by merging into the fork.
- Every assistant Claude session starts with a small fixed context and nothing else (`--safe-mode` on): the vault page index, the routing registry, Joseph's profile, and the meeting rule (live call, read only, answer briefly, never start helpers or agents, never write anything). When the brief names a project or client, that project's vault page comes with it.
- Gap questions and Ask follow the answer cards: a fast answer shows first and the deep model, with read-only access to the vault, replaces or corrects it. The fast lane runs at medium effort.
- A new meeting is named from its call window title, kept whole (a ": <name> 1:1" tail stays, since it says who the call is with); only app chrome such as " - Zoom" is dropped.
- Each meeting shows what it saved (times, length, pictures, disk space) and the meeting list shows totals. The keep rule for pictures stays as it is.
- When a Zoom or Teams call starts, the app shows a macOS notification naming the app ("Zoom meeting started", "Teams meeting started") with a way to record that meeting. Each app is tracked on its own, so two calls in two apps give two notifications.
- One recording at a time. While recording, the app does not prompt for other meetings, as Fluid Voice does.
- A recording started from a detected meeting records only that app's sound, plus the microphone. A manual start records every app's sound by default, and a Call audio picker can narrow it to Zoom or Teams.

## Constraints

- Every model call is the local `claude` CLI on his subscription. No API keys, no paid APIs.
- Audio and transcripts stay on the Mac. The only outside path is Anthropic through the `claude` CLI, and that path also carries pictures of the Zoom or Teams meeting window when "See the Zoom or Teams window" is on.
- The app runs on macOS on Apple Silicon.
- Meeting detection follows Fluid Voice's approach, written fresh for the fork: Fluid Voice is GPL-3 and the fork is MIT, so none of its code is used.

## Non-goals

- The assistant never acts; it informs and converses.
- No spoken replies; output is text only.
- The app is not hidden from screen share and is never described as hidden.
- No preset question list or checklist for gaps.
- To-dos are never sent anywhere (no Todoist, no export).
- Only the PNC Jim 10/20/2026 meeting is backfilled with assistant history. Every other earlier meeting stays as it is.
- Meetily's in-app updater stays off, because it would install plain Meetily over the fork.
- Recording two meetings side by side is a later step.
