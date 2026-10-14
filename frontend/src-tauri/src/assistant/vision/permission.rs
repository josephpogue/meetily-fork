// assistant/vision/permission.rs
//
// Screen Recording permission, for the settings switch. The check is the
// preflight call, which never pops a system prompt.

/// Whether this app may capture other apps' windows.
#[cfg(target_os = "macos")]
pub fn screen_recording_allowed() -> bool {
    core_graphics::access::ScreenCaptureAccess.preflight()
}

#[cfg(not(target_os = "macos"))]
pub fn screen_recording_allowed() -> bool {
    false
}

/// Opens the Screen Recording pane of System Settings.
pub fn open_screen_recording_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open System Settings: {e}"))
}
