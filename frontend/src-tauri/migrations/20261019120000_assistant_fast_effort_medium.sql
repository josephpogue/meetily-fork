-- Fast lane effort default moves from low to medium; rows still on the old default follow it.
UPDATE assistant_settings SET fast_effort = 'medium' WHERE fast_effort = 'low';
