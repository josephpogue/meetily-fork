// assistant/vision/permission.rs
//
// Screen Recording permission, for the settings switch and the amber line.
//
// The preflight call alone is not enough. macOS pins a Screen Recording grant
// to the build's code signature, and after a rebuild with a different
// signature the preflight can still say yes while every real capture is
// refused. So "allowed" means a real window listing works and shows windows of
// other apps, which a refused capture cannot do.

use super::capture::WindowInfo;

/// The app's own bundle id (tauri.conf.json `identifier`, checked by a test).
const OWN_BUNDLE_ID: &str = "com.meetily.ai";

/// Whether a window listing proves capture works: it listed without error and
/// includes at least one window that belongs to another app. A refused
/// capture either errors or shows only this app's own windows.
pub(crate) fn listing_proves_access(listing: &Result<Vec<WindowInfo>, String>) -> bool {
    match listing {
        Ok(windows) => windows
            .iter()
            .any(|w| !w.bundle_id.is_empty() && w.bundle_id != OWN_BUNDLE_ID),
        Err(_) => false,
    }
}

/// Whether this app can really capture other apps' windows right now. It
/// never shows a system prompt (the listing checks the preflight first).
pub async fn screen_recording_allowed() -> bool {
    listing_proves_access(&super::capture::list_windows().await)
}

/// Asks macOS for Screen Recording access when the preflight says it is not
/// granted, which shows the system prompt once. Does nothing when the preflight
/// already passes, because a stale grant never prompts; `scripts/install-mac.sh`
/// resets it in that case.
#[cfg(target_os = "macos")]
pub fn request_access_if_missing() {
    let access = core_graphics::access::ScreenCaptureAccess;
    if !access.preflight() {
        log::info!("Screen Recording is not granted, asking macOS");
        access.request();
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_access_if_missing() {}

/// Opens the Screen Recording pane of System Settings.
pub fn open_screen_recording_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open System Settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(bundle: &str) -> WindowInfo {
        WindowInfo {
            bundle_id: bundle.to_string(),
            app_name: String::new(),
            title: String::new(),
            layer: 0,
            on_screen: true,
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn a_listing_with_another_apps_window_proves_access() {
        let listing = Ok(vec![win("com.meetily.ai"), win("us.zoom.xos")]);
        assert!(listing_proves_access(&listing));
    }

    #[test]
    fn a_failed_listing_does_not() {
        assert!(!listing_proves_access(&Err("cannot list windows: declined".into())));
    }

    #[test]
    fn a_listing_of_only_our_own_windows_does_not() {
        // What a stale grant can look like: the call works, but only this app shows.
        assert!(!listing_proves_access(&Ok(vec![win("com.meetily.ai")])));
        assert!(!listing_proves_access(&Ok(vec![])));
        assert!(!listing_proves_access(&Ok(vec![win("")])));
    }

    #[test]
    fn own_bundle_id_matches_the_tauri_identifier() {
        let conf = include_str!("../../../tauri.conf.json");
        assert!(conf.contains(&format!("\"identifier\": \"{OWN_BUNDLE_ID}\"")));
    }
}
