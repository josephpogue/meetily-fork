# Picture cleanup: job brief

Design: `docs/superpowers/specs/2026-11-18-picture-cleanup-design.md` (read it first).
Entry points: `frontend/src-tauri/src/assistant/cleanup.rs`. Size change: `frontend/src-tauri/src/meeting_saved.rs`.

## Goal

After a call, when Joseph presses Draft note, a second Claude run beside the note names the pictures that add nothing. The app moves them into `removed/` inside their frames folder, shows what it removed on the meeting page, lets Joseph restore any of them, and deletes `removed/` for good five days later. "What was saved" counts only kept pictures.

## Done when

- Pressing Draft note on a call with pictures starts the note and the cleanup together; the note does not wait for the cleanup.
- The meeting page shows the cleanup line and the removed panel as in the approved mockup (`~/.claude/artifacts/587ff039-b5ae-419c-a417-6ffd8246b171/meetily-cleanup/removed-pictures.html`), and restore works from every place the mockup shows it.
- A failed cleanup shows "Picture cleanup did not finish. Try again" and Try again runs the cleanup alone.
- Five days after a cleanup, its `removed/` folders are gone.
- Every locked test passes, and the existing suites still pass.

## States and readers

| State of a meeting | Files on disk | Who reads it | What they must show or do |
|---|---|---|---|
| Never cleaned (no record) | no `picture-cleanup.json`, no `removed/` | meeting page header, `should_start`, "What was saved" | no cleanup line; Draft note starts a run; size and count as today |
| Run in flight | as before the run | `should_start`, header | a second Draft note press (redraft) starts no second run; header shows no cleanup line yet |
| Done, nothing removed | record with `removed: []` | header, summary | no cleanup line (nothing to show) |
| Done, pictures removed | record `done`; files in `<frames>/removed/` | header, panel, "What was saved", totals line | the line "Claude removed N pictures to free X: R repeats, U not useful."; count and size leave out `removed/` |
| Done, some restored | record entries `restored: true`, files back | header, panel | counts, bytes and line leave out restored pictures; restored files count again in "What was saved" |
| Done, all restored | every entry restored | header | no cleanup line |
| Failed | record `failed` with error, nothing moved | header, `should_start` | "Picture cleanup did not finish."; Draft note or Try again starts a run |
| Purged | record `purged`; no `removed/` | header, `purge_if_expired`, `should_start` | no cleanup line; no new run; restore is not offered |
| Recording has no deep-lane session (assistant off, or an older call) | anything | `should_start` caller | no run starts; nothing changes |

Readers of the frames folders today: `meeting_saved.rs` (`count_jpgs` reads only the top of each frames folder; `dir_bytes` counts every subfolder), `assistant/core.rs` and `assistant/vision/capture.rs` (write frames during a recording only).

## Ways a run can end, and what must still happen

| How it ends | What must happen after |
|---|---|
| Reply parsed, removals applied | record `done` written, header refreshes |
| Reply parsed, nothing to remove | record `done` with an empty list; nothing moved |
| Reply cannot be parsed | record `failed`; nothing moved |
| The `claude` run fails or times out | record `failed`; nothing moved |
| The run could not start (no deep session, no meeting folder) | no record written; nothing moved; the note still drafts |
| App quits or a new recording starts mid-run | nothing moved, no record; the next Draft note may run it |
| A file move fails part way | pictures already moved stay recorded so they can be restored; the record lists only pictures really in `removed/`; no picture is lost |
| A picture named in the reply no longer exists | skipped; the rest are applied |

## Rules for parsing a reply

- The reply holds one JSON object with a `remove` array; text before or after it is ignored.
- Reasons are `loading`, `greyed`, `blank`, `popup`, `before_share`, `repeat`.
- An id not in the picture list is skipped.
- An id listed twice is removed once (first entry wins).
- A `repeat` must name `repeat_of` as a picture that is kept (in the list and not itself removed); otherwise that removal is skipped, so a repeat never points at nothing.
- An unknown reason skips that entry.
- `{"remove": []}` is a valid reply meaning "remove nothing".
- No JSON object, or no `remove` array: Err.

## Worked examples

Meeting folder with `teams-frames/teams-20261118-110300-478.jpg` (200 bytes), `teams-frames/teams-20261118-110332-532.jpg` (300 bytes), `teams-frames/teams-20261118-110433-637.jpg` (400 bytes), `zoom-frames/zoom-20261118-110310-000.jpg` (100 bytes).

- `list_pictures` gives P1 teams 11:03:00, P2 zoom 11:03:10, P3 teams 11:03:32, P4 teams 11:04:33 (capture order across both folders, by the time in the file name).
- `picture_list` line for P1: `P1 = Teams window at 11:03:00`; for P2: `P2 = Zoom window at 11:03:10`.
- Reply `Here you go: {"remove":[{"id":"P1","reason":"before_share"},{"id":"P3","reason":"repeat","repeat_of":"P4","caption":"Same page, scrolled up"}]} done` parses to two removals.
- `apply` at now_ms = 1_000 moves P1 to `teams-frames/removed/teams-20261118-110300-478.jpg` and P3 to `teams-frames/removed/teams-20261118-110332-532.jpg`; the record has state done, ran_at_ms 1000, delete_after_ms 1000 + 5 days in ms (432_000_000), two entries with bytes 200 and 300, P3's `repeat_of` = `teams-frames/teams-20261118-110433-637.jpg`.
- After that, `list_pictures` lists only the zoom picture and P4 (renumbered P1, P2).
- `summary` after that: removed 2, repeats 1, not_useful 1, bytes 500, line `Claude removed 2 pictures to free under 1 MB: 1 repeat, 1 not useful.` (sizes use `meeting_saved::format_size`; counts use singular for 1).
- With 93 removed, 56 repeats, 37 not useful and 53,477,376 bytes: `Claude removed 93 pictures to free 51 MB: 56 repeats, 37 not useful.`
- Only repeats removed: `Claude removed 3 pictures to free under 1 MB: 3 repeats.` Only not useful: `...: 2 not useful.`
- `restore(folder, ["teams-frames/teams-20261118-110300-478.jpg"])` moves that file back, marks it restored; summary now removed 1, bytes 300.
- `restore(folder, [])` restores every removed picture.
- `purge_if_expired(folder, 1000 + 432_000_000 - 1)` is false and deletes nothing; at `1000 + 432_000_000` it deletes `teams-frames/removed/` and marks the record purged; summary line is None.
- Failed record: summary line `Picture cleanup did not finish.`, failed true.
- `should_start(None, false)` true; `(None, true)` false; failed record true; done record false; purged record false.
- "What was saved" with 200 bytes in `teams-frames/a.jpg` and 300 bytes in `teams-frames/removed/b.jpg` and 50 bytes `audio.mp4`: pictures 1, bytes 250. A folder named `removed` anywhere else in the meeting folder still counts.

## Constraints

- Every model call is the local `claude` CLI; the cleanup run forks the deep-lane session the way `AnswerLanes::draft_note` does (`assistant/lanes.rs`), with read-only tools.
- `draft_note_gated` in `assistant/core.rs` holds the core lock while the note run is in flight; the cleanup run must not wait on that lock, or the two would not run side by side.
- New code lives in its own modules; edits to upstream files stay small and are listed in `docs/FORK_CHANGES.md`. `docs/GOAL.md` gets the two Requirements lines named in the design.
- Supporting text 12.5px or larger. No em dashes in any text.

## Answers to the tester's questions

- **A move that fails part way:** `apply` returns Ok; the record lists only the pictures really in `removed/`, and the failure is logged. Err only when the record itself cannot be written.
- **Purge and a failed record:** `purge_if_expired` does nothing to a Failed record and returns false; the record stays. Only a Done record whose window has ended is purged.
- **Restore with no record, or a Purged record:** Err, and nothing changes.
- **Repeats are checked in reply order.** A `repeat` is accepted only if its `repeat_of` is still kept at that point. A later entry that would remove a picture an accepted repeat points at is skipped, so a kept picture that a repeat names always stays. Two repeats naming each other: the first is accepted, the second skipped.
- **Every named picture gone:** `apply` returns Ok with a Done record and an empty `removed` list.

## Test file locations

The guard accepts files under a `tests/` folder: `frontend/src-tauri/src/assistant/tests/cleanup.rs` (child module of cleanup.rs) and `frontend/src-tauri/src/tests/meeting_saved_cleanup.rs` (child module of meeting_saved.rs).

## Round 2: review findings (2026-11-18)

The high review found four ways the record and the disk can disagree. New rules:

- **The disk is the truth.** `load` reconciles the record with the disk before returning it: an entry whose picture is at `removed_file` is not restored; an entry whose picture is back at `file` is restored; an entry whose picture is in neither place is dropped, also when its whole frames folder is gone. A picture in a `removed/` folder that the record does not list is moved back to its frames folder (so nothing is ever stranded unrestorable), and is not listed. When reconciling changed anything, `load` writes the corrected record.
  - Example: after `apply` on the worked example, move `teams-frames/removed/teams-20261118-110300-478.jpg` back to `teams-frames/` by hand. `load` marks that entry restored, and `summary` shows 1 removed.
  - Example: a picture `teams-frames/removed/x.jpg` with no record entry. `load` moves it to `teams-frames/x.jpg`; it is not in the record; `list_pictures` lists it.
  - Example: with no record file at all and a stray `teams-frames/removed/x.jpg`, `load` still moves it back and returns None.
  - Example: Undo moved a restored picture back into `removed/` but the record still says restored. `load` marks it not restored.
- **One writer at a time.** `apply`, `restore`, `record_failure` and `purge_if_expired` on the same meeting folder never interleave: two `restore` calls on different pictures of one folder, run at the same time from two threads, both take effect, and the record afterwards marks both restored. A record write never leaves a half-written file (write to a unique temporary file in the same folder, then rename).
- **Try again sees every picture.** A retry forks the saved session and attaches every picture that session had not been sent, the same set the first run would have attached. The saved session file records which pictures that session already holds.
  - Entry point: `SavedSession.sent` (meeting-relative paths of the pictures the session already holds; a file without the field loads with an empty list) and `unsent(pictures, session)`, which returns the pictures not in `sent`, in their order.
  - Example: pictures P1 `teams-frames/a.jpg`, P2 `teams-frames/b.jpg`, P3 `zoom-frames/c.jpg`; `sent` = [`teams-frames/a.jpg`]. `unsent` gives P2 and P3.
  - Example: a session file saved without `sent` loads, and `unsent` gives every picture.
- **Round 3 (review 2):** an entry is dropped even when its frames folder no longer exists. The round 1 test that reads back each kind of record now puts the Done record's pictures on disk where the record says they are, so the rule and that test agree.

## Round 4: the cleanup never starts after Stop (walkthrough, 2026-11-19)

**Bug.** Stop closes the call's lanes (`AnswerLanes::close`), and Draft note only works after Stop. `start_beside_note` returns at once on closed lanes, so the cleanup never runs in the real app. The run's result is also thrown away on closed lanes. The tests never closed the lanes first, so they missed it.

**Rule.** Stop is the normal state for a cleanup, not a reason to skip it. Only a new recording cancels it.

- New entry points on `AnswerLanes`: `supersede(&self)` marks these lanes replaced by a new recording, and `is_superseded(&self) -> bool` reads it. A shared handle (`share()`) sees the same mark. `close()` does not set it.
- `start_beside_note` starts a run on lanes that are closed (after Stop), when they have a deep session and the folder has pictures and `should_start` allows it.
- It starts nothing on superseded lanes.
- A run whose lanes are superseded before it finishes moves nothing and writes no record, and the next Draft note may run it again. A run on lanes that are only closed applies its result as usual.
- Opening a new recording marks the previous call's lanes superseded before it replaces them (wiring in `assistant/core.rs`, not tested here).
- Every early return in `start_beside_note` logs one info line naming the reason, so a skipped cleanup shows in the app log.

| State of the lanes at Draft note | Run starts | Result applied |
|---|---|---|
| Open (no Stop yet; not reachable from Draft note, which needs Stop) | yes | yes |
| Closed by Stop | yes | yes |
| Closed by Stop, then superseded by a new recording mid-run | yes | no: nothing moved, no record, no run in flight afterwards |
| Superseded before Draft note | no | no record |
| No deep session | no | no record |

Ways a run ends: done (record written, pictures moved), failed (failed record, nothing moved), superseded (no record, nothing moved). In every case the in-flight mark is cleared afterwards, so `run_in_flight` is false.

**Worked example.** Folder with `zoom-frames/zoom-20261118-110300-000.jpg` and `zoom-frames/zoom-20261118-110310-000.jpg`. Lanes from `AnswerLanes::from_saved_session(fake_runner, "deep-1", cfg, vec![])`; `close()` called (Stop). The fake runner answers `{"remove":[{"id":"P1","reason":"before_share"}]}`. `start_beside_note(None, folder, lanes, "transcript")`; once `run_in_flight(folder)` is false: P1 is in `zoom-frames/removed/`, the record is Done with one entry.
- Same, but `supersede()` is called on a shared handle while the fake runner is still answering (the fake waits on a signal): afterwards both pictures are still in `zoom-frames/`, there is no record, and `run_in_flight` is false.
- Same, with `supersede()` before `start_beside_note`: nothing moves, no record, no session file.

**Done when** the locked tests pass and the walkthrough shows the "Claude removed" line after Stop then Draft note.
