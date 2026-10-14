-- "See the Zoom window" switch for the live assistant. On by default.
ALTER TABLE assistant_settings ADD COLUMN see_zoom_window INTEGER NOT NULL DEFAULT 1;
