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
- New code lives in its own modules, and edits to upstream files stay small and are listed in `docs/FORK_CHANGES.md`.
- Upstream releases arrive only by merging into the fork. Meetily's in-app updater stays off, because it would install plain Meetily over the fork.
- Every assistant Claude session starts with a small fixed context and nothing else (`--safe-mode` on): the vault page index, the routing registry, Joseph's profile, and the meeting rule (live call, read only, answer briefly, never start helpers or agents, never write anything). When the brief names a project or client, that project's vault page comes with it.
- Gap questions and Ask follow the answer cards: a fast answer shows first and the deep model, with read-only access to the vault, replaces or corrects it. The fast lane runs at medium effort.
