# Ask Next: proactive questions from the meeting brief - Design

Adds a proactive half to the live assistant. Today the assistant reacts: it answers when the room asks Joseph something, or when he asks it. Ask Next works the other way. Joseph describes what he wants out of the call in the meeting brief; during the call the assistant compares that goal with what it has heard and lists the gaps as questions for Joseph to ask the other side. It can also put a short question to Joseph himself when the brief leaves something unclear.

The motivating case is a discovery or introductory call where Joseph does not yet know which questions an experienced person would ask. There are no call types or templates: the brief, in his own words, is the only input. With no brief, the assistant infers the purpose of the call from the transcript.

Builds on `docs/superpowers/specs/2026-09-01-live-assistant-design.md`. All binding constraints from that spec carry over unchanged: subscription-only `claude -p`, nothing leaves the machine except through the claude CLI, the assistant suggests and never acts, text only, and an assistant failure never touches recording or transcription.

## What Joseph sees

The assistant panel gains two sections above the existing card stream.

1. **Meeting brief (optional).** The existing box, same label, same placeholder. Change: it stays visible and editable for the whole call instead of disappearing when the session opens. An edit commits on blur. A collapse chevron folds it to one line so it does not crowd the panel.
2. **Ask next.** Up to 5 questions for Joseph to ask the other side, most important first. Each item is the question in one line plus one short muted line saying why it matters to the brief. Each item has a dismiss (x) control. A small Refresh button sits in the section header, with a spinner while a refresh runs. An "updated 12s ago" stamp sits beside it.
3. **For you** (inside the Ask next section, below the list, only when non-empty). Up to 2 questions the assistant has for Joseph about his own goal, for example "Is pricing in scope for this call?". Each has a one-line answer field (Enter submits) and a dismiss control. A submitted answer is appended to the brief text as a new line `- <question> <answer>`, visible and editable in the brief box like anything else he typed.

Items that the transcript answers drop off the list on the next refresh. Questions that are still open keep their exact wording and position where possible, so the list does not jump around while he reads it.

Empty states inside the Ask next section:

- Session not open: section hidden (the brief box shows alone, as today).
- Session open, no list yet: "Listening. Questions appear after the first exchange."
- Model says there are no gaps: "Nothing to ask right now."
- Last refresh failed: the previous list stays, the stamp turns amber with "refresh failed", and the panel's existing transient error line shows the reason.

## When the list refreshes

- **Automatically after someone finishes speaking:** every closed utterance from either channel (You or Them) requests a refresh.
- **Manually:** the Refresh button and a new hotkey, Option-Q.
- **After a brief edit, a dismiss, or a For-you answer:** each requests a refresh so the list reflects it right away.

Requests coalesce rather than queue. At most one Ask Next turn is in flight. A request that arrives while a turn runs sets a dirty flag; when the turn finishes, one follow-up turn runs if the flag is set. The model's own latency (a few seconds) is therefore the natural rate limit, and a burst of short utterances costs at most two turns.

The existing listening toggle (header dot) pauses automatic refreshes as well as answer triggers. A manual refresh still runs while paused. The assistant on/off switch turns Ask Next off with everything else. There is no separate Ask Next setting.

## Architecture

All new code lives inside the existing `assistant/` module and `AssistantPanel`. No upstream meetily files change beyond what the live assistant already touches.

### The ask lane (`assistant/ask_lane.rs`, new)

A third lineage forked from the same seeded trunk as the fast and deep lanes, opened in `AnswerLanes::open` alongside them (one more parallel fork, no added wait). It owns its own session id, transcript cursor, in-flight flag, and dirty flag.

- Model: the fast model from settings (`claude-sonnet-5` by default) at effort `medium`, pinned for the meeting. Medium rather than the fast lane's low because the quality of the questions is the whole point and there is no two-second reading deadline. Not a user setting.
- No tools, `--safe-mode`, same as the fast lane. It reasons from the brief and the transcript only.
- System prompt `assistant/prompts/ask.md` (new, embedded with `include_str!`).
- Each turn is `claude -p --resume <ask lane id>`. The prompt carries only what changed since the last ask turn:
  - the transcript delta since the ask cursor (same `TranscriptLog::delta_since` the other lanes use),
  - the full current brief, only if it changed since the last ask turn,
  - questions Joseph dismissed since the last turn ("Joseph dismissed these; do not suggest them again"),
  - the current list as last emitted, so the model can keep still-open items verbatim.
- Every turn returns the full replacement list, not a diff. This keeps the model's job simple and makes "answered questions drop off" the default behavior rather than something the model has to remember to signal.

The lane is a separate struct rather than more fields on `AnswerLanes`, because its turn shape (whole-list replace, coalescing, no streaming into a card) shares nothing with the answer/settle flow. It reuses `Runner`, `ClaudeTurn`, `TranscriptLog`, and `card::content_words`.

### Output contract and parsing (`assistant/ask_list.rs`, new)

The model writes exactly this shape:

```
ASK: <question for Joseph to ask> | WHY: <one short reason tied to the brief>
ASK: ...
YOU: <question for Joseph himself>
```

or the single word `NONE`. At most 5 ASK lines and 2 YOU lines; extra lines are dropped by the parser, not trusted to the prompt. A line with no `| WHY:` part is kept with an empty reason. Unparseable output keeps the previous list and counts as a failed refresh.

Stable ids: the parser assigns each new item the id of the previous item it matches (same kind, content-word overlap of at least 0.6 using `card::content_words`), else a fresh uuid. React keys on the id, so an item whose wording shifted slightly does not flash out and back in.

### Brief changes mid-call

`set_brief` today stores a pending brief that the next session seeds from. New behavior:

- Before a session opens: unchanged.
- While a session is open: store it as the current brief and bump a `brief_version`. The ask lane includes the new brief in its next turn (see above). The fast and deep lanes also pick it up: each lane remembers the brief version it last saw, and its next turn's prompt is prefixed with "The meeting brief was updated:" plus the new text. This keeps answer cards and Ask Next working from the same goal.
- A For-you answer goes through the same `set_brief` path after the panel appends the line, so there is one way the brief changes.

### Commands and events

New commands (all `Result<T, String>`, thin wrappers in `commands.rs` over plain async functions in `core.rs`, matching the existing pattern):

- `assistant_refresh_asks()`
- `assistant_dismiss_ask(id: String)` - removes the item from the emitted list at once, records it for the next turn, requests a refresh.
- `assistant_set_brief(text)` - existing command; now also valid while a session is open, as described above.

The For-you answer needs no new command: the panel appends the line to the brief text and calls `assistant_set_brief`, then dismisses the For-you item with `assistant_dismiss_ask`.

New event `assistant-asks`:

```
{ state: "idle" | "thinking" | "failed",
  asks:   [{ id, question, why }],
  forYou: [{ id, question }],
  updatedAt: <ms> | null,
  brief: <current brief text> }
```

`brief` is included so the panel's brief box reflects the server's copy after a session opens (the pending brief moves into the session) without a second round trip.

### Meeting note

The note draft already has an open-questions section. The final Ask next list (asks only, not For-you) is passed into the note turn alongside the Q&A log as "Questions the assistant suggested that were still open when the call ended", so the note can list them as follow-ups. No change to how or where the note is saved.

## Prompt (`assistant/prompts/ask.md`)

Key instructions, final wording written during implementation:

- You are the planning half of a silent meeting assistant. Joseph is in a live call. Your output is a short list he glances at between turns.
- The brief is Joseph's goal for the call in his own words. If there is no brief, infer the most likely purpose from the transcript and work toward that.
- List what Joseph still needs to learn from the other side to reach the goal. Phrase each as a question he can say out loud as written: short, open, one topic each, in plain conversational words.
- Rank by how much the answer moves him toward the goal. Prefer questions that fit where the conversation is now over ones that would jerk it to a new topic.
- Drop a question as soon as the transcript answers it, even partly. Do not repeat anything already asked by You.
- Keep the exact wording of any still-open question from the current list.
- Use YOU lines only when the brief is ambiguous in a way that changes which questions matter. Never more than 2.
- Never suggest anything Joseph dismissed.
- Output contract as above, no preamble, no em dashes.

## Error handling

- Ask lane failed to fork at session open: Ask Next shows "Ask next unavailable for this call" and the answer lanes are unaffected. `lanes_ready` still reflects only the fast and deep lanes.
- Failed or unparseable turn: keep the previous list, emit `state: "failed"`, set `last_error`. The next request tries again.
- Session close (recording stopped): kill an in-flight ask turn, clear the dirty flag, leave the last list on screen until the next session opens.

## Testing

- Unit (`ask_list.rs`): parse ASK/YOU/NONE, caps at 5 and 2, missing WHY, junk output, id carry-over on reworded items, fresh id on a new item, dismissed item never re-emitted.
- Unit (`ask_lane.rs`, fake `Runner`): coalescing (three requests during one turn produce exactly one follow-up turn), cursor advances only on success, brief included only when its version changed, dismissed list sent once then cleared, failure keeps the previous list.
- Unit (`core.rs`): `set_brief` while open bumps the version and fast/deep prompts get the update prefix once.
- End to end, as Joseph hits it (the guard that counts): dev build; type a brief such as "Intro call with a property manager. I want to learn their fees, how they screen tenants, and how they handle maintenance."; start a recording; play a spoken WAV of the other side covering fees only. Confirm: the Ask next list appears after the first utterance, the fees question drops off, tenant screening and maintenance remain; dismiss one and it does not return; press Refresh and the spinner runs; answer a For-you question and see the line appended to the brief; edit the brief mid-call and see the list change. Stop and draft the note; the still-open questions appear in it. Screenshot each state.

## Out of scope

- Call types, templates, or question playbooks.
- Suggested edits to the brief from the assistant (Joseph's call: not for now).
- Marking a question as "asked" by hand; the transcript decides.
- Saving briefs between meetings or reusing a past brief.
- Any change to the existing answer-card behavior other than picking up mid-call brief edits.
