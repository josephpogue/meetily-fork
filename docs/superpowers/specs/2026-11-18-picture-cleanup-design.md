# Picture cleanup after a call

## Intent

A call keeps many pictures that add nothing: loading spinners, "Saving" overlays, popups, blank pages, the gallery view before anyone shares, and near-repeats of a picture already kept. A 63-minute Teams call on 2026-11-18 kept 137 pictures (78 MB); 93 of them (51 MB) were useless or repeats.

Joseph wants Claude to remove those after the call, so the recording folder holds only pictures worth looking at. His reasons: disk space, and "not over bloating with useless images". He must be able to see at a glance what was removed and bring any picture back.

## What Joseph sees

Mockup (clickable): `~/.claude/artifacts/587ff039-b5ae-419c-a417-6ffd8246b171/meetily-cleanup/removed-pictures.html`, with screenshots in `shots/` beside it. Joseph approved it on 2026-11-18.

- **The header line.** Under "What was saved" on the meeting page, one line: "Claude removed **93 pictures** to free 51 MB: 56 repeats, 37 not useful. See what was removed. You can bring them back until Mon, Nov 23." Three small thumbnails sit in front of it. "What was saved" counts only kept pictures.
- **The removed panel.** "See what was removed" opens a panel inline on the meeting page (not a pop-up), which scrolls on its own. Its title reads "93 pictures removed, 51 MB freed", with a "Restore all 93" button.
  - "Not useful" comes first, grouped by reason: Still loading, Greyed out while saving or testing, Blank or no data, Popup over the page, Before anyone shared. A caption gives a more specific reason where Claude has one.
  - "Repeats" come next, each group led by the kept picture (tagged Kept) with its removed repeats beside it.
  - Every removed thumbnail has a restore button. Each reason group has "Restore these N".
  - A restored picture gets a green outline and a Restored tag, the counts update, and a toast offers Undo.
- **Full size.** Clicking a picture opens it full size; a repeat opens beside the kept picture it repeats. Arrow keys step through; Restore works from there.
- **No delete button.** Removal is already done; Joseph's only action is Restore. Removed pictures are deleted for good when the restore window ends.

## How it works

- **When it runs.** Pressing Draft note starts two Claude runs side by side: the note (unchanged) and the cleanup. The cleanup runs once per recording: a redraft after a delete, or a note retry, does not start it again unless the earlier cleanup failed.
- **Context.** The cleanup run forks the call's deep-lane session, the same way `draft_note` does (`assistant/lanes.rs:946`), so it holds every picture the call kept, the brief, the questions and answers, and read-only vault access. It also gets the full transcript and any pictures the deep lane had not seen yet.
- **What Claude returns.** A list of the pictures to remove, each with a reason group, an optional specific caption, and, for a repeat, which kept picture it repeats. When pictures repeat each other, Claude keeps the clearest one. Claude writes nothing; the app does the moving.
- **Removing.** The app moves each removed picture into a `removed/` folder inside its frames folder (`teams-frames/` or `zoom-frames/`) and records the reasons beside it. `count_jpgs` in `meeting_saved.rs` reads only the top folder, so "What was saved" already counts only kept pictures.
- **Restoring.** Restore moves a picture back and updates the record.
- **Sizes during the restore window (my call).** "What was saved" and the meeting-list totals count only kept pictures, so the size leaves out `removed/` (today `dir_bytes` in `meeting_saved.rs` counts subfolders, and its `size_is_recursive_total` test changes with it; list the edit in `docs/FORK_CHANGES.md`). The header line keeps "to free 51 MB"; the panel title reads "93 pictures removed, 51 MB freed on Mon, Nov 23", so it never claims space that is still on disk.
- **When the cleanup fails (my call).** The meeting page shows one line in place of the header line, "Picture cleanup did not finish. Try again", and Try again starts the cleanup run alone. The failure is stored with the meeting so the page shows it after a restart.
- **Deleting for good.** Five days after the cleanup, the app deletes the `removed/` folder and its record. After that the header line no longer offers restore.

## Approaches considered

1. **A separate cleanup run that forks the call's session (picked).** Same context as the note, one job per run, and a failed note does not block the cleanup.
2. **Fold the cleanup into the note run.** No extra run, but the note prompt does two jobs and one failure stops both.
3. **Code removes repeats, Claude judges the rest.** Rejected: that call's repeats already passed the code's thumbnail check, so code cannot find them.

Joseph picked 1 because each run has the same context and the cleanup stays independent.

## Goals

- Remove useless pictures and repeats from every call where Draft note is pressed.
- Make what was removed visible at a glance and restorable in one click.
- Free the disk space when the restore window ends.

## Non-goals

- The keep rule during the call stays as it is (GOAL.md Requirements).
- No cleanup for recordings without a deep-lane session, such as calls recorded with the assistant off or before this feature.
- No manual "Delete now" button.

## Anti-goals

- Removing a picture that shows something discussed on the call. A wrong removal costs Joseph trust; restore is the safety net, not the plan.
- Any write outside the recording folder.

## Constraints

- Every model call is the local `claude` CLI on Joseph's subscription.
- New code lives in its own modules; edits to upstream files stay small and go in `docs/FORK_CHANGES.md`.
- The PR adds a Requirements line to `docs/GOAL.md` for this decision, and a row to `docs/FORK_CHANGES.md`.
- Supporting text is 12.5px or larger (the mockup uses 13px in the header).

## Rules this touches

- **"Nothing is written anywhere without a Save press."** The cleanup moves pictures when Draft note is pressed, before Save. Decided: a second exception beside the assistant-items save, because the move stays inside the recording folder and never reaches the vault.
- **"The assistant never acts; it informs and converses."** Claude only returns a list; the app moves the files, and every move can be undone for five days. Decided: picture cleanup is app housekeeping, not the assistant acting.

Joseph agreed to both on 2026-11-18. The PR adds them to `docs/GOAL.md` Requirements: the picture move on Draft note is the second write that needs no Save press, and picture cleanup is app housekeeping, not the assistant acting.

## Left to the builder

- The cleanup prompt's wording and the exact reply format.
- The file format of the removed-pictures record.
- How the five-day deletion is triggered (for example, on app start and on opening the meeting list).
- How thumbnails for the panel are made.
