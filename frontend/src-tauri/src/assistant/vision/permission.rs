// assistant/vision/permission.rs
//
// Screen Recording permission, for the settings switch. Owned by the
// settings slice; filled in there.

/// Whether this app may capture other apps' windows.
pub fn screen_recording_allowed() -> bool {
    false
}
