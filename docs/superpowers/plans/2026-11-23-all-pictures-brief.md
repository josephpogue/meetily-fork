# All pictures: job brief

Design: `docs/superpowers/specs/2026-11-23-all-pictures-design.md` (read it first). It extends picture cleanup: `docs/superpowers/specs/2026-11-18-picture-cleanup-design.md` and its brief `docs/superpowers/plans/2026-11-18-picture-cleanup-brief.md`, whose rules hold unless this brief changes them.

Entry points:
- `frontend/src-tauri/src/assistant/cleanup.rs`: new `remove_by_you`, `all_pictures`, `MeetingPicture`, `AllPictures`, `Reason::ByYou`, `CleanupState::NotRun`, `RemovedPicture.delete_after_ms`, `CleanupSummary.by_you`; changed behaviour of `apply`, `record_failure`, `restore`, `remove_again`, `load`, `purge_if_expired`, `should_start`, `summary`.
- `frontend/src/components/MeetingSaved/allPictures.ts`: `shownPictures`, `seeAllLabel`.

## Goal

After a call Joseph can see every picture of a meeting, kept and removed, in one panel on the meeting page. He can restore what Claude should have kept and remove what Claude should have removed. His removals go in the same record as Claude's, with the same Restore and Undo, and each removed picture is deleted for good five days after it was removed.

## Done when

- "See all N pictures" on the What was saved line opens the All pictures panel on every meeting with pictures, also right after Stop and before Draft note.
- Remove on a kept picture moves it to the removed set, a toast offers Undo, and the header line, What was saved and the sidebar totals update.
- "Show removed" puts the removed pictures in their time spots with reason and Restore; the viewer steps through what the panel shows, with Remove or Restore.
- The removed panel shows a "Removed by you" group first.
- A picture removed on day 4 is still restorable on day 8, and deleted on day 9.
- Every locked test passes, and the existing suites still pass.

## The record after this change

`CleanupRecord` becomes `{ state, ran_at_ms, removed }`:
- `state` is Claude's run only: `NotRun` (only Joseph's removals so far; `ran_at_ms` 0), `Done`, or `Failed { error }`. `Purged` goes away.
- The record-level `delete_after_ms` goes away; each `RemovedPicture` carries its own `delete_after_ms`.
- The file format is the builder's, but a record written by PR #30 must still load:
  - `{"state":"done","ranAtMs":1000,"deleteAfterMs":432001000,"removed":[...entries without deleteAfterMs...]}` loads as Done, each entry with `delete_after_ms` 432001000.
  - `{"state":"purged",...}` loads as Done with an empty list.
  - `{"state":"failed","error":"boom","ranAtMs":7,"deleteAfterMs":0,"removed":[]}` loads as Failed.

## States and readers

| State of a meeting | Who reads it | What they must show or do |
|---|---|---|
| No record, pictures on disk | All pictures panel, header, `should_start` | every picture listed as kept; no cleanup line; Draft note starts Claude's run |
| NotRun, Joseph removed some | header, both panels, `should_start`, purge | line "You removed 4 pictures to free 2 MB."; Draft note still starts Claude's run; Joseph's pictures purge on their own clocks |
| Claude's run in flight, Joseph removes | `apply`, `remove_by_you` | both changes survive: the run's result is merged into the record as it is when the run applies, never over it |
| Done, Claude removed, Joseph removed none | header, panels | as today |
| Done, both removed | header, panels | "Claude removed 93 pictures to free 51 MB: 56 repeats, 37 not useful. You removed 4." |
| Done, Claude's all restored or deleted, Joseph's pending | header | "You removed 4 pictures to free 2 MB." |
| Failed, Joseph's pending | header, `should_start`, purge | "Picture cleanup did not finish." with Try again, and "See what was removed" still reachable; Draft note or Try again starts a run; Joseph's pictures purge on their clocks |
| Some pending past their date | purge, both panels | only those pictures are deleted and their entries dropped; the rest stay restorable |
| Everything restored or deleted | header | no cleanup line; All pictures lists every picture as kept |
| PR #30 record on disk | `load` and every reader | loads as above, nothing lost |

Readers of the record today: `cleanup_run.rs` (`view_of`, `picture_cleanup_get`, `purge_expired_all`, `retry`), `PictureCleanup.tsx`, `MeetingSaved/index.tsx`, `Sidebar/SidebarProvider.tsx` (totals).

## Rules

**`remove_by_you(folder, files, now_ms)`**
- Each name is a kept picture's `file` (top of `zoom-frames/` or `teams-frames/`). A name that is not a kept picture, or is missing from disk, is skipped.
- It moves into `removed/` in its frames folder. Its entry: reason ByYou, no caption, no `repeat_of`, its bytes, its `time` from the file name, `restored` false, `delete_after_ms` = now_ms + 432_000_000.
- An earlier entry for the same file (a picture Claude removed and Joseph restored) is replaced by the ByYou entry.
- No record yet: it writes one with state NotRun. A Done or Failed record keeps its state.
- An empty list removes nothing; with no record it writes none. Returns Ok with the record as it now is (a NotRun record with no entries when there is none).
- A move that fails part way: pictures already moved stay recorded; Ok. If the record cannot be written, every picture this call moved goes back, and Err.
- Claude's reply can never name ByYou: `parse_reply` keeps its six reasons.

**`apply` (Claude's run)** reads the record inside the folder lock and adds its removals to it: Joseph's entries stay as they are, the state becomes Done, `ran_at_ms` = now_ms, each new entry's clock = now_ms + 432_000_000. A picture Joseph removed while the run was out is missing from disk and is skipped. If the record cannot be written, only the pictures this run moved go back.

**`record_failure`** keeps every entry and sets the state to Failed.

**`should_start(record, in_flight)`**: in flight false; None true; NotRun true; Failed true; Done false (also a PR #30 purged record, which loads as Done).

**`restore(folder, files)`** works on any record that has entries, whatever its state; an empty list restores every pending picture; no record, or a record with no entries, is Err. **`remove_again`** (Undo of a restore) puts the picture back exactly as it was: same reason, caption, `repeat_of` and `delete_after_ms`. Undo of Joseph's Remove is `restore` of that file.

**`load`** reconciles every record that has entries, whatever its state, with the same disk-is-truth rules as before; a stray picture in `removed/` that no entry lists is moved back.

**`purge_if_expired(folder, now_ms)`** looks at each entry, in any state:
- pending and `delete_after_ms` <= now_ms: its file is deleted and its entry dropped;
- restored and `delete_after_ms` <= now_ms: its entry is dropped (its file is a kept picture and stays);
- a delete that fails keeps that entry, so the next sweep tries again.
- It never changes the state, and an empty `removed/` folder may be deleted. True when it deleted at least one file.

**`summary`**
- `removed`, `repeats`, `not_useful`: Claude's pending entries (reason not ByYou), as today. `by_you`: Joseph's pending entries.
- `bytes`: every pending entry, Claude's and Joseph's. `delete_after_ms`: the soonest pending entry's date; 0 when none.
- `line`: Failed gives "Picture cleanup did not finish." (failed true; counts still filled). Otherwise Claude's part as today, using only Claude's bytes, then " You removed N." when Joseph has pending; with only Joseph's pending: "You removed N pictures to free X." (`format_pictures`, `format_size`). None when nothing is pending.

**`all_pictures(folder)`** loads (with reconcile), then lists every kept picture and every pending entry, oldest first by the time in the file name across both folders:
- kept: `shown_file` = `file`, `removed` false, reason, caption and `repeat_of` None, `delete_after_ms` 0, bytes from disk. A restored picture is kept.
- removed: `shown_file` = its `removed_file`, `removed` true, its reason, caption, `repeat_of`, `delete_after_ms` and bytes.
- `kept`, `kept_bytes`, `removed` (count of pending). `title`: "44 pictures kept, 25 MB"; with none kept: "No pictures kept". A folder with no frames folders gives an empty list.

**Frontend helpers (`allPictures.ts`)**
- `shownPictures(all, false)`: only pictures with `removed` false, in the given order. `shownPictures(all, true)`: every picture, in the given order.
- `seeAllLabel(kept, removed)`: kept 44 "See all 44 pictures"; kept 1 "See 1 picture"; kept 0 with removed 3 "See all pictures"; kept 0 with removed 0 null. Counts above 999 use commas ("See all 1,204 pictures").

**Wiring (not unit tested; the walkthrough checks it)**
- New commands in `cleanup_run.rs`, registered in `lib.rs`: `picture_cleanup_all(meetingId) -> AllPictures` (purges first, as `picture_cleanup_get` does) and `picture_cleanup_remove(meetingId, files) -> CleanupView`.
- `view_of`: title counts every pending picture ("97 pictures removed, 53 MB freed on Mon, Nov 30"), dates use the soonest pending date, and the note fits a record with only Joseph's removals.
- `PictureCleanup.tsx`: "Removed by you" group first, with Restore per picture, Restore these N and Restore all. A repeat group whose kept picture Joseph removed tags that picture "Removed by you" instead of Kept. "See what was removed" shows whenever anything is pending, also on a Failed record.
- New All pictures panel beside it, opened from the What was saved line, in the same place as the removed panel; one of the two open at a time. Remove and Restore buttons on thumbnails and in the viewer; the toast's Undo after Remove calls restore.
- After every Remove, Restore or Undo the panels redraw from the command's returned view, never from their own edited copy, and the sidebar totals reload as Restore does today.

## Worked examples

Meeting folder: `teams-frames/teams-20261118-110300-478.jpg` (T1, 200 bytes), `zoom-frames/zoom-20261118-110310-000.jpg` (Z, 100), `teams-frames/teams-20261118-110332-532.jpg` (T3, 300), `teams-frames/teams-20261118-110433-637.jpg` (T4, 400). Day = 86_400_000 ms; five days = 432_000_000.

- No record. `all_pictures`: four kept in order T1, Z, T3, T4; kept 4, kept_bytes 1000, removed 0, title "4 pictures kept, under 1 MB".
- `remove_by_you(folder, [T4], 1000)`: T4 is at `teams-frames/removed/teams-20261118-110433-637.jpg`; record NotRun, ran_at_ms 0, one entry ByYou, bytes 400, `delete_after_ms` 432_001_000. `should_start` true. `summary`: by_you 1, removed 0, bytes 400, line "You removed 1 picture to free under 1 MB.", delete_after_ms 432_001_000.
- Then `all_pictures`: T1, Z, T3 kept, T4 removed (`shown_file` its removed path, reason by_you), in that order; kept 3, removed 1, title "3 pictures kept, under 1 MB".
- Then Claude's run: pictures from `list_pictures` are T1 P1, Z P2, T3 P3; `apply` at 5000 with P1 before_share. Record Done, ran_at_ms 5000, two entries: T4 ByYou clock 432_001_000, T1 BeforeShare clock 432_005_000. `summary` line "Claude removed 1 picture to free under 1 MB: 1 not useful. You removed 1.", bytes 600, delete_after_ms 432_001_000.
- `purge_if_expired(folder, 432_001_000)`: T4's file is deleted and its entry dropped; T1 stays in `removed/`; true. Line now "Claude removed 1 picture to free under 1 MB: 1 not useful."
- In-flight run: `list_pictures` gave T1..T4; Joseph `remove_by_you([T3])` at 2000 before the run applies; the run's reply removes P3 (T3) as a repeat of P4 and P1 as blank. `apply`: T3 is skipped (already gone from its place), T1 removed; the record keeps T3 as ByYou.
- Restore and remove again: after Claude removed T1 (Done, clock 432_005_000), `restore([T1])`, then `remove_by_you([T1], 300_000_000)`: one entry for T1, ByYou, clock 732_000_000; Claude's count 0.
- Undo of a restore: Claude removed T1 as popup with caption "Save changes? box", clock 432_005_000; `restore([T1])` then `remove_again([T1])`: the entry is popup, same caption, clock 432_005_000.
- Failed with Joseph's removal: `remove_by_you([T4], 1000)` then `record_failure("boom", 2000)`: state Failed, T4's entry kept; `summary` failed true, by_you 1, line "Picture cleanup did not finish."; `should_start` true; `purge_if_expired(folder, 432_001_000)` deletes T4.
- `remove_by_you([], 1000)` with no record: Ok, no record file written. `remove_by_you(["teams-frames/nope.jpg"], 1000)`: Ok, nothing moved, no record written.
- A record with only restored entries: `restore` of nothing pending is Ok and changes nothing; with no record at all, `restore` is Err.
- PR #30 record as in "The record after this change", with T1 in `removed/`: `load` gives Done with T1's clock 432_001_000; `all_pictures` lists T1 as removed with that date.
- Frontend: `shownPictures` on [T1 kept, T3 removed, T4 kept] with false gives [T1, T4]; with true gives all three in order.

## Existing tests that encode the old rule

Update these with the new tests: `frontend/src-tauri/src/assistant/tests/cleanup.rs` (the `entry` and `record` helpers, the record-level `delete_after_ms` asserts, every `CleanupState::Purged` case, "restore with a purged record is Err", "purge ignores a failed record"), `tests/cleanup_round2.rs` (purge, should_start and summary cases), and check `tests/cleanup_after_stop.rs` still holds.

## Constraints

- New code lives in the fork's own modules; edits to upstream files stay small and are listed in `docs/FORK_CHANGES.md`. `docs/GOAL.md` Requirements: the picture-cleanup line gains "Joseph can also remove a kept picture himself, with the same five-day Restore."
- `apply`, `remove_by_you`, `restore`, `remove_again`, `record_failure` and `purge_if_expired` take the same per-folder lock; record writes stay temporary file then rename.
- Supporting text 12.5px or larger. No em dashes in any text.

## Test file locations

- New: `frontend/src-tauri/src/assistant/tests/cleanup_all_pictures.rs` (already declared as a child module of `cleanup.rs`, so `use super::*;` works).
- Updated: `frontend/src-tauri/src/assistant/tests/cleanup.rs`, `tests/cleanup_round2.rs`.
- New: `frontend/src/components/MeetingSaved/allPictures.test.ts`, `bun:test`, like `Sidebar/cleanupEvents.test.ts`.
