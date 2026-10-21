# Goal

Joseph's own build of Meetily: an assistant that sits beside him during a call, mainly as a consultant on discovery and knowledge-transfer calls. It keeps track of what the call has covered and tells him what to say, both answers to questions he is asked and questions he should ask. He can also chat with it on the side at any point in the call.
- **Track the call:** Follow the live transcript and the point of the call, taken from an optional meeting brief he can edit during the call and from the conversation itself.
- **Know what he knows:** Have the same context as his Second Brain vault, and before the call starts, read what the vault holds on the client, project and people involved, so the assistant is ready from the first minute.
- **Tell him what to say:** Have a suggested answer ready when someone asks him a question, and keep a live list of the gaps it sees, each with a question he can ask to fill it, and save every answered question with its answer.
- **Chat on the side:** Answer anything he types or speaks to it mid-call, using the call so far and his vault.
- **Stay close to upstream:** Add only what his use needs, so every upstream Meetily release merges in cheaply.

Every difference from upstream Meetily, built or planned, is listed in `docs/FORK_CHANGES.md`.

## Stories

Each story is one person and one result. Check a change against them: Joseph must still get each result.
- **Answer on the spot:** Someone on the call asks Joseph a question, and a suggested answer is on screen before he has to reply.
- **Fill the gaps:** On any call, Joseph sees what is still unknown and a question to ask for each gap.
- **Gaps clear themselves:** Once a question Joseph asks or the assistant suggests is answered, either on the call or by Joseph and the assistant working it out, it moves off the open list and is kept with its answer, without Joseph touching it.
- **Point of the call:** Joseph types a brief before or during the call, or none at all, and the assistant still knows what the call is for.
- **Ask privately:** Joseph types or speaks a question mid-call and gets an answer that uses the call so far.
- **Catch up:** After losing the thread, Joseph presses one key and gets what was just said and what it means.
- **Keep track of promises:** During a call, Joseph sees what he promised and what the other side owes him, picked up from what is said without typing; a to-do the call shows finished moves to Done by itself, and after the call the list is kept with the meeting.
- **Keep a record:** After the call, a note with decisions, action items, answered questions with their answers, and open questions goes into the vault only when Joseph saves it.
- **Stay current:** A new Meetily release merges into the fork, and every feature in `docs/FORK_CHANGES.md` still works.

## Requirements

Decisions Joseph has made for this version. Every feature follows them.
- Every model call is the local `claude` CLI on his subscription. No API keys, no paid APIs, and Meetily's own summary providers stay untouched.
- Audio and transcripts stay on the Mac. The only outside path is Anthropic through the `claude` CLI, and that path also carries pictures of the Zoom meeting window when "See the Zoom window" is on.
- The assistant sees only the Zoom meeting window, and only while recording. It keeps a picture only when the window changes, by the /watch rule in `~/.claude/skills/watch/scripts/frames.py` (scene score above 0.20, then a 16x16 thumbnail more than 2.0 from the last kept picture). Kept pictures are saved full-size in the meeting folder.
- The assistant informs and converses, and never acts. Nothing is written anywhere without a Save press.
- Output is text only, with no spoken replies.
- The app runs on macOS on Apple Silicon.
- The app is a normal window. It is not hidden from screen share, and is never described as hidden.
- The meeting brief is optional and editable during the call. The assistant also works out the point of the call from the conversation.
- Gaps are the assistant's own judgment from the context it has. There is no preset question list or checklist.
- Once a question Joseph asks or the assistant suggests is answered, either on the call or by Joseph and the assistant working it out, the assistant moves it off the open list and keeps it with its answer. Nothing is deleted, and Joseph does not tick questions off.
- The assistant panel keeps what to say and what to ask apart: a Say lane and an Ask lane, one label, icon and colour per kind of content, and nothing in the panel cut short. The panel width is Joseph's to drag, and it is remembered.
- What the assistant showed during a call is saved with the meeting automatically when the meeting is saved, with no prompt at the end. Saving it is the one write that needs no Save press, because it stays inside the app's own meeting storage and the recording folder, never the vault.
- The saved assistant items show in an Assistant tab beside the transcript, each with its call time. A copy is kept as `assistant.json` in the meeting's recording folder, and Joseph can delete a meeting's saved items.
- The assistant keeps a live To do list. A You to-do is something Joseph says he will do, something the other side asks him to do unless he declines, or one he types after `todo:` in the Ask box (Opt-T starts one in the app). A Them to-do is something someone on the other side promises him, or agrees to when he asks, shown with the person's name when the call says it. The You and Them transcript labels help tell them apart.
- To-dos are picked up on the Ask next run (quick, then checked), not by a new Claude call per utterance. A to-do the call shows finished moves to Done by itself, the way answered questions move, and Joseph never ticks one off. Each to-do shows its call time and can be dismissed.
- The To do list is a one-line strip across the panel above Explain and Catch up, with the You and Them counts and the newest to-do in full. Clicking it opens a two-column drawer, You promised under Say and They owe you under Ask, over the lower part of the lanes until it is closed. The Say and Ask lanes keep their full size. To do is its own kind: rose, with a clipboard icon and no checkboxes.
- To-dos are saved with the meeting in its assistant history and `assistant.json`, and shown at the top of its Assistant tab. They are never sent anywhere (no Todoist, no export), and the vault meeting note does not change.
- Only the PNC Jim 10/20/2026 meeting is backfilled with assistant history. Every other earlier meeting stays as it is.
- New code lives in its own modules, and edits to upstream files stay small and are listed in `docs/FORK_CHANGES.md`.
- Upstream releases arrive only by merging into the fork. Meetily's in-app updater stays off, because it would install plain Meetily over the fork.
- Every assistant Claude session starts with a small fixed context and nothing else (`--safe-mode` on): the vault page index, the routing registry, Joseph's profile, and the meeting rule (live call, read only, answer briefly, never start helpers or agents, never write anything). When the brief names a project or client, that project's vault page comes with it.
- Gap questions and Ask follow the answer cards: a fast answer shows first and the deep model, with read-only access to the vault, replaces or corrects it. The fast lane runs at medium effort.
