CREATE TABLE IF NOT EXISTS meeting_assistant_meta (
    meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    brief TEXT NOT NULL DEFAULT '',
    vault_project TEXT,
    vault_note_path TEXT,
    vault_transcript_path TEXT,
    filed_at TEXT
);
