# assistant-backfill

One-off tool for meetily-fork. The assistant's output for the meeting "PNC - Introductory Call with Jim 10/20/2026"
(`meeting-10c08825-4e2e-498d-b1ce-252b8de0f071`) was never saved; it survives only in the Claude Code transcripts of the
assistant's claude CLI sessions. `backfill.py` rebuilds that meeting's items from those transcripts into the
`meeting_assistant_items` table and `<meeting folder>/assistant.json`, in the shape of the assistant-history contract.
It is approved for that one meeting only. Python 3 standard library only.

## Run

```
cd scripts/assistant-backfill
python3 -I backfill.py                                  # dry run (default): prints the table and counts, writes nothing
python3 -I backfill.py --apply --db <sqlite> --folder <meeting folder>
python3 -I -m unittest                                  # tests
```

- Defaults: `--sessions` is the assistant session folder under `~/.claude/projects/`, `--meeting-id` the Jim meeting,
  `--folder` the Jim recording folder (its `metadata.json` `created_at` is the recording start; the dry run only reads it).
- `--apply` needs `--db`. It refuses, with a plain message, when the `meeting_assistant_items` table is missing (run the new
  meetily-fork build once so it adds the table), when the meeting id is not in `meetings`, or when the meeting already has
  items (pass `--replace` to overwrite). It inserts every row in one transaction with fresh uuid4 ids, and writes
  `assistant.json` atomically (temp file and rename) before the commit; a failed write rolls the rows back.
- The sessions are only ever read. Quit the app before the real apply so the database is not open for writes.

## How it reads the sessions

- Answer lanes: the two sessions whose prompts carry `TRIGGER:`. The deep lane is the one that used read-only file tools
  (`tool_use` blocks, a stronger model, slower replies); the fast lane has none (lanes.rs: fast = no tools, deep = ReadOnly).
- Ask next lanes: the two sessions whose prompts carry `CURRENT LIST:`.
- A reply is the last non-empty assistant text after a prompt (what the CLI's final `result` holds). `SKIP`, `NONE`, `READY`,
  empty replies and replies the app's parsers cannot use are never saved.
- Cards follow `card.rs`; a fast and a deep reply for one trigger are one item settled as in `lanes.rs` (checked, or corrected
  with changed lines). Ask items follow `ask_list.rs` and are deduped by normalized text.
- `call_time = max(0, (shown_at - recording start) / 1000)`, three decimals, positions in call_time order.
