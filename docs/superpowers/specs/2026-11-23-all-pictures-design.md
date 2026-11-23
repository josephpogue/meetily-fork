# All pictures after a call

## Intent

Picture cleanup (PR #30) shows only what Claude removed. Joseph cannot see the pictures that were kept, except a kept picture that leads a group of removed repeats.

Joseph wants to see every picture of a meeting after the call, with the option to see what was removed beside them. His reason, in his words: "I want to be able to see if what was kept is what I want as well (if some things were missed or can additionally be deleted)."

So the page must let him do two checks in one place:
- Claude removed something worth keeping: he restores it (this exists today in the removed panel).
- Claude kept something useless: he removes it himself (new).

## What Joseph sees

- **The link.** The "What was saved" line on the meeting page ends with "See all N pictures", where N is the kept count. It shows on every meeting that has pictures, also meetings Claude never cleaned up.
- **The All pictures panel.** The link opens a panel inline on the meeting page, in the same place and the same way as the removed panel (it takes the page body, Hide brings the transcript and summary back). Only one of the two panels is open at a time.
  - Title: "44 pictures kept" with the kept size.
  - Every kept picture in capture order, each thumbnail with its time.
  - A "Show removed (93)" switch, off by default. On, it puts each removed picture in its time spot, greyed, with its reason ("Repeat", "Still loading", "Removed by you" ...) and a Restore button. The switch is absent when nothing is removed.
  - Each kept thumbnail has a Remove button. Remove moves the picture into the removed set, the counts update, and a toast offers Undo.
- **Full size.** Clicking a picture opens the same full-size viewer the removed panel uses. Arrow keys step through what the panel shows (kept only, or kept and removed when the switch is on). The viewer has Remove for a kept picture and Restore for a removed one.
- **The removed panel.** Unchanged, plus a "Removed by you" group first, with the same per-picture, per-group and Restore all buttons. A repeat group whose lead picture Joseph removed shows that picture tagged "Removed by you" instead of Kept.
- **The header line.** Claude's line stays as it is. When Joseph removed pictures, it adds "You removed 4." The line shows whenever anything is removed and not restored, by Claude or by Joseph, so "See what was removed" is always reachable.

## How it works

- **One record (approach 1).** Joseph's removals go in the same removed-pictures record as Claude's (`picture-cleanup.json`), with a new reason, "removed by you". They use the same move into `removed/`, the same Restore, Undo and reconcile-with-disk rules.
- **A five-day clock per picture.** Each removed picture is deleted for good five days after it was removed. Claude's removals keep the date of Claude's run; a picture Joseph removes on day 4 gets five full days. The delete step changes from "delete the whole `removed/` folder on one date" to "delete each picture past its date". The header and panel show the soonest date; the viewer shows that picture's own date.
- **Claude's run and Joseph's removals are independent.** The record has to hold both "has Claude's cleanup run, and did it fail" and the list of removed pictures, which today share one state.
  - Joseph can remove a picture before Draft note. Claude's cleanup still runs later, and it sees only the kept pictures (it already reads only the top frames folder).
  - Claude's run adds its removals to the record; it never replaces Joseph's.
  - A failed Claude run still shows "Picture cleanup did not finish. Try again" beside any removals Joseph made.
  - A meeting whose removed pictures were already deleted for good can still take new removals.
- **Sizes.** "What was saved" and the sidebar totals already count only kept pictures; Remove and Restore reload them the way Restore does today.
- **New command.** One backend command lists every picture of the meeting, kept and removed, in capture order, with its state, reason and delete date. One command removes pictures by file. Both live in the fork's own modules.

## Approaches considered

1. **Add Joseph's removals to Claude's record (picked).** One restore path, one set of counts, the existing Restore, Undo and reconcile code reused. Cost: the delete step goes per picture, and the record splits Claude's run state from the removed list, so PR #30's locked tests for purge and state change with it.
2. **A separate record for Joseph's removals.** Claude's record and tests stay as they are. Rejected: it needs a second removed folder (the reconcile step moves back any file in `removed/` its record does not list), a second restore path and two sets of counts.
3. **Remove deletes for good at once.** Rejected: Joseph chose the same five-day Restore as Claude's removals.

Joseph picked 1 on 2026-11-23.

## Goals

- Every picture of a meeting is visible after the call, kept and removed, in one place.
- Joseph can fix Claude's cleanup both ways: restore what it should have kept, remove what it should have removed.

## Non-goals

- Claude does not learn from Joseph's fixes. A fix changes this meeting only (Joseph, 2026-11-23: "we don't need that bigger feature").
- No "Delete now"; every removal waits its five days.
- No Select or Select all for removing several kept pictures at once (my call: one click each with Undo; easy to add later).
- The in-call keep rule stays as it is.

## Anti-goals

- A Remove that loses a picture before its five days are up.
- Joseph's removals being undone or overwritten by Claude's run, a redraft, the reconcile step or the delete step.
- Any write outside the recording folder.

## Constraints

- New code lives in the fork's own modules; edits to upstream files stay small and go in `docs/FORK_CHANGES.md`.
- Supporting text is 12.5px or larger.
- The look follows the existing removed panel (thumbnails, tags, toast, viewer), so no new mockup (my call).

## Rules this touches

- **The cleanup design's "No delete button" and "No manual Delete now".** Joseph agreed on 2026-11-23 that a kept picture gets Remove, which moves it to the removed set with the five-day Restore. "Delete now" stays out. The PR updates the picture-cleanup line in `docs/GOAL.md` Requirements: Joseph can remove a kept picture himself, with the same five-day Restore.
- **"Nothing is written anywhere without a Save press."** Remove moves a file inside the recording folder, like Claude's cleanup, which is already the second exception.

## Left to the builder

- The record's new shape and how an existing PR #30 record is read into it.
- How the delete step finds pictures past their date (it still runs on app start and on loading the meeting list).
- Thumbnail sizes and loading for a meeting with 150+ pictures.
- The exact wording of the toast and the Remove button label.
