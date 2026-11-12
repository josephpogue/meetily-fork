-- Standing rules for the live assistant: text carried by every turn on every call. Empty by default.
ALTER TABLE assistant_settings ADD COLUMN standing_rules TEXT NOT NULL DEFAULT '';
