CREATE TABLE IF NOT EXISTS meeting_assistant_items (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    call_time REAL NOT NULL,
    shown_at TEXT NOT NULL,
    question TEXT NOT NULL DEFAULT '',
    lead TEXT NOT NULL DEFAULT '',
    bullets_json TEXT NOT NULL DEFAULT '[]',
    why TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '',
    phase TEXT NOT NULL DEFAULT '',
    changed_lines_json TEXT NOT NULL DEFAULT '[]',
    answer TEXT NOT NULL DEFAULT '',
    answered_how TEXT NOT NULL DEFAULT '',
    answered_call_time REAL
);
CREATE INDEX IF NOT EXISTS idx_meeting_assistant_items_meeting ON meeting_assistant_items(meeting_id, position);
