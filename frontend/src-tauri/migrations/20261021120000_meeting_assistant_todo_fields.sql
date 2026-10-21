-- To-do items ride the same table: kind 'todo' keeps its text in `lead`.
-- Additive only, so an older build still opens the database.
ALTER TABLE meeting_assistant_items ADD COLUMN owner TEXT NOT NULL DEFAULT '';
ALTER TABLE meeting_assistant_items ADD COLUMN owner_name TEXT NOT NULL DEFAULT '';
ALTER TABLE meeting_assistant_items ADD COLUMN due TEXT NOT NULL DEFAULT '';
