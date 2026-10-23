// meeting_detect/probe.rs
//
// Gathers one tick's signals: which call apps hold the mic (Core Audio, by
// the shared ownership rule in audio/call_apps), which windows exist
// (ScreenCaptureKit) and which app is frontmost (NSWorkspace). The pure part
// turns a window list into per-app signals.

use crate::assistant::meeting_name::meeting_name_from_titles;
use crate::assistant::vision::capture::{
    is_teams, is_teams_meeting, is_zoom_meeting, WindowInfo, ZOOM_MIN_HEIGHT, ZOOM_MIN_WIDTH,
};
use crate::assistant::vision::CallApp;
use crate::audio::call_apps::{audio_processes, owned_by, running_apps, AudioProcess, CallAppId};

use super::rules::AppSignals;

/// Mic and owned processes per app: the Core Audio half of a tick.
pub struct AudioView {
    pub owned: [Vec<AudioProcess>; 2],
}

impl AudioView {
    pub fn mic(&self, app: CallAppId) -> bool {
        self.owned[slot(app)].iter().any(|p| p.running_input)
    }
}

fn slot(app: CallAppId) -> usize {
    match app {
        CallAppId::Zoom => 0,
        CallAppId::Teams => 1,
    }
}

/// One Core Audio snapshot. Processes the ownership rule cannot place are ignored.
pub fn audio_view() -> anyhow::Result<AudioView> {
    let procs = audio_processes()?;
    let apps = running_apps();
    Ok(AudioView {
        owned: [
            owned_by(CallAppId::Zoom, &procs, &apps),
            owned_by(CallAppId::Teams, &procs, &apps),
        ],
    })
}

/// The frontmost app's bundle id.
#[cfg(target_os = "macos")]
pub fn frontmost_bundle_id() -> Option<String> {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::CStr;

    // SAFETY: plain Objective-C message sends on classes AppKit always provides; every returned pointer is
    // null-checked before use and the autorelease pool is drained before returning.
    unsafe {
        let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
        let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let front: *mut Object = msg_send![ws, frontmostApplication];
        let mut out = None;
        if !front.is_null() {
            let id: *mut Object = msg_send![front, bundleIdentifier];
            if !id.is_null() {
                let p: *const std::os::raw::c_char = msg_send![id, UTF8String];
                if !p.is_null() {
                    out = Some(CStr::from_ptr(p).to_string_lossy().into_owned());
                }
            }
        }
        let _: () = msg_send![pool, drain];
        out
    }
}

#[cfg(not(target_os = "macos"))]
pub fn frontmost_bundle_id() -> Option<String> {
    None
}

fn app_matches(app: CallAppId, bundle_id: &str) -> bool {
    app.bundle_ids().contains(&bundle_id)
}

/// The meeting name a window title gives, None when generic.
fn clean(app: CallAppId, title: &str) -> Option<String> {
    let kind = match app {
        CallAppId::Zoom => CallApp::Zoom,
        CallAppId::Teams => CallApp::Teams,
    };
    meeting_name_from_titles(&[(kind, title.to_string())])
}

/// Window signals for one app. `windows` is None when listing failed or was
/// skipped: no window evidence, so no prompt.
pub fn signals(
    app: CallAppId,
    mic: bool,
    frontmost_bundle: Option<&str>,
    windows: Option<&[WindowInfo]>,
) -> AppSignals {
    let mut s = AppSignals {
        mic,
        frontmost: frontmost_bundle.is_some_and(|b| app_matches(app, b)),
        ..Default::default()
    };
    let Some(windows) = windows else { return s };
    match app {
        CallAppId::Zoom => {
            if let Some(w) = windows.iter().find(|w| is_zoom_meeting(w)) {
                s.meeting_window = true;
                s.title = clean(app, &w.title);
            }
        }
        CallAppId::Teams => {
            if let Some(w) = windows.iter().find(|w| is_teams_meeting(w)) {
                s.meeting_window = true;
                s.title = clean(app, &w.title);
            } else if let Some(w) = windows.iter().find(|w| is_plain_teams_window(w)) {
                s.plain_window = true;
                s.title = clean(app, &w.title);
            }
        }
    }
    s
}

/// A titled, normal-size, layer-0 Teams window.
fn is_plain_teams_window(w: &WindowInfo) -> bool {
    is_teams(w)
        && w.layer == 0
        && w.width >= ZOOM_MIN_WIDTH
        && w.height >= ZOOM_MIN_HEIGHT
        && !w.title.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(bundle: &str, title: &str) -> WindowInfo {
        WindowInfo {
            bundle_id: bundle.to_string(),
            app_name: String::new(),
            title: title.to_string(),
            layer: 0,
            on_screen: true,
            width: 900.0,
            height: 700.0,
        }
    }

    #[test]
    fn zoom_home_window_is_not_a_meeting_window() {
        let w = [win("us.zoom.xos", "Zoom Workplace")];
        assert!(!signals(CallAppId::Zoom, true, None, Some(&w)).meeting_window);
    }

    #[test]
    fn zoom_meeting_window_gives_a_cleaned_title() {
        let w = [win("us.zoom.xos", "Zoom Meeting - Standup")];
        let s = signals(CallAppId::Zoom, true, None, Some(&w));
        assert!(s.meeting_window);
        assert_eq!(s.title.as_deref(), Some("Standup"));
    }

    #[test]
    fn teams_main_window_is_plain_and_meeting_window_wins() {
        let main = [win("com.microsoft.teams2", "Chat | Acme | Microsoft Teams")];
        let s = signals(CallAppId::Teams, true, Some("com.microsoft.teams2"), Some(&main));
        assert!(!s.meeting_window && s.plain_window && s.frontmost);
        let call = [win("com.microsoft.teams2", "Design review | Microsoft Teams")];
        let s = signals(CallAppId::Teams, true, None, Some(&call));
        assert!(s.meeting_window && !s.plain_window);
        assert_eq!(s.title.as_deref(), Some("Design review"));
    }

    #[test]
    fn no_listing_means_no_window_evidence() {
        let s = signals(CallAppId::Teams, true, None, None);
        assert!(s.mic && !s.meeting_window && !s.plain_window);
    }

    #[test]
    fn other_apps_windows_do_not_count() {
        let w = [win("us.zoom.xos", "Zoom Meeting")];
        let s = signals(CallAppId::Teams, true, None, Some(&w));
        assert!(!s.meeting_window && !s.plain_window);
    }

    /// Live, read-only: prints each app's signals on this Mac. Zoom and Teams
    /// may simply be closed. Run with --ignored --nocapture.
    #[tokio::test]
    #[ignore]
    async fn live_probe() {
        let view = audio_view().expect("core audio snapshot");
        let front = frontmost_bundle_id();
        let windows = crate::assistant::vision::capture::list_windows().await;
        println!("frontmost app: {:?}", front);
        match &windows {
            Ok(w) => println!("windows listed: {}", w.len()),
            Err(e) => println!("window listing failed: {}", e),
        }
        for app in CallAppId::ALL {
            let s = signals(app, view.mic(app), front.as_deref(), windows.as_ref().ok().map(|w| w.as_slice()));
            println!(
                "{}: mic={} meeting_window={} plain_window={} frontmost={} title={:?}",
                app.as_str(), s.mic, s.meeting_window, s.plain_window, s.frontmost, s.title
            );
            for p in &view.owned[slot(app)] {
                println!(
                    "  owned: pid={} bundle={:?} in={} out={} exe={:?}",
                    p.pid, p.bundle_id, p.running_input, p.running_output, p.exe_path
                );
            }
        }
    }
}
