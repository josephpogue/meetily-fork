# Meeting title from the note writer

Agreed with Joseph on 2026-11-25.

## Goal

When the assistant drafts the note after Stop, it also names the meeting in Joseph's naming framework, so meetings stop keeping names like "Meeting 25_11_26_11_03_31" or "PNC Teams meeting".
Nine of his fifteen recordings from 2026-11-12 to 2026-11-25 kept a timestamp name.

## The framework

`{CODE} - {Topic} {M/D/YY}`, for example `PNC - VCF Design Workshop 10/26/26`.

- **CODE** is the project's short code from `~/.claude/routing/project-codes.json` (`$PROJECT_CODES_FILE` overrides the path), the same file `todo-add` reads. Shape: `{"codes": {"<slug>": "<CODE>"}}`.
- The code belongs to the project picked in the note bar's Project list. A slug with no code of its own takes the code of the longest slug in the file that is a prefix of it plus "-" (`collaborator-design` gets COL; `broadcom-pnc-bank-wave-plan` gets PNC, not BRCM).
- No project, no code for it, or a missing or unreadable codes file: no code and no " - ", so the title is `{Topic} {M/D/YY}`.
- **Topic** is written by the note writer in the same Claude reply that drafts the note (a new `=== TITLE ===` section). It is a short topic only, with no code and no date, in the style of Joseph's past titles ("VCF Design Workshop", "Sprint Planning Meeting", "Maikol Intro / Catch Up Call").
- **Date** is the meeting's local start date, month/day/two-digit year with no leading zeros: `11/5/26`.

## What Joseph sees

After Stop, the note preview gets a Title box above Project, already filled in.

```
Title    [ PNC - VCF Design Workshop 11/25/26 ]
Project  [ broadcom-pnc-bank (suggested)   v ]
[ Save ]  [ Discard ]
```

- He can edit the box before Save or Discard.
- Picking another project swaps the code in the box, until he edits the box; after an edit the box keeps his text.
- Save and Discard both rename the meeting to the box's text. Discard still keeps the note out of the vault.
- After Save, the vault note's heading and its line under `## Meetings` on the project page use the same title.
- The sidebar and the meeting page header show the new name without a restart.

## Which names get replaced

- Replaced: the automatic timestamp names (`meeting_name::is_automatic_name`) and the name the app took from the call window at Stop (PR #20).
- Never replaced: a name Joseph typed. The app records the call-window name with the meeting at Stop, so it can tell the two apart later.
- When the current name is one he typed, the Title box shows that name and project changes do not touch it. It changes only if he edits the box.

## States and readers

| State | Title box | Meeting name after Save or Discard |
|---|---|---|
| Timestamp name, writer returned a topic | `{CODE} - {Topic} {date}` for the picked project | the box text |
| Call-window name, writer returned a topic | same as above | the box text |
| Name Joseph typed (before or during the call) | his name | unchanged unless he edited the box |
| Writer returned no TITLE section, or an empty one | the current name | unchanged unless he edited the box |
| Draft failed or never ran (assistant off) | no note bar | unchanged |
| Re-draft after a delete (note still Ready) | new suggestion, unless he edited the box, which keeps his text | the box text |
| He renamed the meeting in the sidebar while the note bar was open, box unedited | as it was | his sidebar name stays |
| Window closed or app quit before Save or Discard | gone | unchanged |
| Item deleted after Save (filed note is rewritten) | n/a | the rewritten vault note keeps the saved title |

Readers of the meeting name: the sidebar list, the meeting page header, the vault note heading, the project page `## Meetings` line, and the note writer's own picker input (`THIS MEETING` and `MEETINGS JOSEPH FILED BEFORE`).

## Worked examples

| Project pick | Codes file | Topic | Date | Title |
|---|---|---|---|---|
| broadcom-pnc-bank | PNC | VCF Design Workshop | 2026-10-26 | `PNC - VCF Design Workshop 10/26/26` |
| broadcom-csl-behring | CSL | Sprint Planning Meeting | 2026-11-05 | `CSL - Sprint Planning Meeting 11/5/26` |
| collaborator-design | only `collaborator: COL` | Stage Review | 2026-01-09 | `COL - Stage Review 1/9/26` |
| broadcom-pnc-bank-wave-plan | `broadcom: BRCM`, `broadcom-pnc-bank: PNC` | Wave Plan | 2026-12-31 | `PNC - Wave Plan 12/31/26` |
| job-search (no code) | no entry | Recruiter Screen | 2026-11-25 | `Recruiter Screen 11/25/26` |
| No project | any | PSO Automation Community Call | 2026-11-25 | `PSO Automation Community Call 11/25/26` |
| broadcom-pnc-bank | file missing | Capacity Planning | 2026-11-25 | `Capacity Planning 11/25/26` |

## Approaches

- **Chosen:** the note writer returns the topic in its existing reply, and the app builds the title from the codes file and the date. No extra Claude call, and the code and date are never left to the model.
- **Rejected:** a separate Claude call for the title, because it adds a wait and a second reply to parse.
- **Rejected:** the model writes the whole title, because the code and date would drift from the codes file and the calendar.

## Docs in the same PR

- `docs/GOAL.md` Requirements: the line "A new meeting is named from its call window title" now says the call-window name holds until the note writer's title replaces it.
- `docs/FORK_CHANGES.md`: a new row for this change.
- `prompts/note.md`: the output contract gains the TITLE section.

## Left to the builder

- Where the call-window name is stored, as long as it survives an app restart.
- How the Title box suggestions reach the frontend.

## Done when

The locked tests pass, and on a real recording the Title box shows the framework title, Save and Discard rename the meeting in the sidebar, and a saved vault note carries the title as its heading.
