// assistant/vision/capture.rs
//
// The capture task: finds the Zoom (or else the Teams) meeting window, grabs
// it about once a second, keeps changed frames, saves them and records them
// in a FrameStore.

use std::path::PathBuf;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::store::{CallApp, FrameStore};

/// Zoom's bundle id on macOS.
const ZOOM_BUNDLE_ID: &str = "us.zoom.xos";
/// Microsoft Teams: the new app, then classic Teams.
const TEAMS_BUNDLE_IDS: [&str; 2] = ["com.microsoft.teams2", "com.microsoft.teams"];
/// A Zoom or Teams window smaller than this (points) is a popover, not the meeting.
const ZOOM_MIN_WIDTH: f64 = 400.0;
const ZOOM_MIN_HEIGHT: f64 = 300.0;
/// The sections Teams' main window starts its title with ("Chat | Acme |
/// Microsoft Teams"). A meeting window has none of them.
const TEAMS_SECTIONS: [&str; 17] = [
    "activity",
    "chat",
    "teams",
    "calendar",
    "calls",
    "onedrive",
    "files",
    "apps",
    "copilot",
    "communities",
    "people",
    "assignments",
    "help",
    "settings",
    "search",
    "viva engage",
    "approvals",
];
/// Kept frames are saved at this JPEG quality, full size.
const SAVE_QUALITY: f64 = 0.9;
/// One grab never takes longer than this before the loop gives up and retries.
const GRAB_TIMEOUT: Duration = Duration::from_secs(10);

/// Which window to capture. `zoom()` in production; `from_env_or_zoom()`
/// lets a test point the capture at another app's window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowTarget {
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub title_contains: Option<String>,
}

impl WindowTarget {
    pub fn zoom() -> Self {
        Self {
            bundle_id: Some(ZOOM_BUNDLE_ID.to_string()),
            app_name: None,
            title_contains: None,
        }
    }

    /// `MEETILY_VISION_APP` (a bundle id or an app name) and optional
    /// `MEETILY_VISION_TITLE` (a title substring) point the capture at that
    /// app's largest on-screen window instead of Zoom's. Unset means `zoom()`.
    /// Debug builds only: a release build always returns `zoom()`, so a stray
    /// variable can never point capture at another app and send it to Claude.
    pub fn from_env_or_zoom() -> Self {
        #[cfg(debug_assertions)]
        {
            Self::from_overrides(
                std::env::var("MEETILY_VISION_APP").ok().as_deref(),
                std::env::var("MEETILY_VISION_TITLE").ok().as_deref(),
            )
        }
        #[cfg(not(debug_assertions))]
        {
            Self::zoom()
        }
    }

    #[cfg_attr(not(any(debug_assertions, test)), allow(dead_code))]
    fn from_overrides(app: Option<&str>, title: Option<&str>) -> Self {
        let app = app.map(str::trim).filter(|s| !s.is_empty());
        let Some(app) = app else {
            return Self::zoom();
        };
        let title = title
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // A bundle id has dots ("com.apple.Preview"); an app name usually not.
        if app.contains('.') {
            Self {
                bundle_id: Some(app.to_string()),
                app_name: None,
                title_contains: title,
            }
        } else {
            Self {
                bundle_id: None,
                app_name: Some(app.to_string()),
                title_contains: title,
            }
        }
    }

    /// True for the production target (Zoom, else Teams), which has the
    /// meeting-window rules; any other target takes its largest matching window.
    fn is_zoom(&self) -> bool {
        self.bundle_id.as_deref() == Some(ZOOM_BUNDLE_ID)
            && self.app_name.is_none()
            && self.title_contains.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Kept Zoom frames are written here full-size. Created on first keep.
    pub frames_dir: PathBuf,
    /// Kept Teams frames are written here. Created on first Teams keep.
    pub teams_frames_dir: PathBuf,
    pub target: WindowTarget,
    /// Between grabs while the window is there (about 1s).
    pub interval: Duration,
    /// Between looks for the window while it is missing (a few seconds).
    pub retry_interval: Duration,
}

impl CaptureConfig {
    pub fn new(frames_dir: PathBuf, teams_frames_dir: PathBuf) -> Self {
        Self {
            frames_dir,
            teams_frames_dir,
            target: WindowTarget::from_env_or_zoom(),
            interval: Duration::from_secs(1),
            retry_interval: Duration::from_secs(5),
        }
    }
}

/// Stops the capture task when `stop()` is called or the handle drops.
pub struct CaptureHandle {
    token: CancellationToken,
}

impl CaptureHandle {
    pub fn stop(&self) {
        self.token.cancel();
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

/// Starts the capture task on its own tokio task and returns at once. It
/// never blocks a caller and never fails loudly: no window, no permission or
/// a failed grab only logs and waits.
pub fn start(cfg: CaptureConfig, store: FrameStore) -> CaptureHandle {
    let token = CancellationToken::new();
    let task = run(cfg, store, token.clone());
    match tokio::runtime::Handle::try_current() {
        Ok(rt) => {
            rt.spawn(task);
        }
        Err(_) => {
            tauri::async_runtime::spawn(task);
        }
    }
    CaptureHandle { token }
}

// ---------------------------------------------------------------------------
// Window choice (pure)
// ---------------------------------------------------------------------------

/// What the window choice needs to know about one on-screen-capable window.
#[derive(Debug, Clone)]
struct WindowInfo {
    bundle_id: String,
    app_name: String,
    title: String,
    layer: i64,
    on_screen: bool,
    width: f64,
    height: f64,
}

impl WindowInfo {
    fn area(&self) -> f64 {
        self.width * self.height
    }
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

fn is_zoom_meeting(w: &WindowInfo) -> bool {
    w.bundle_id == ZOOM_BUNDLE_ID
        && w.layer == 0
        && w.width >= ZOOM_MIN_WIDTH
        && w.height >= ZOOM_MIN_HEIGHT
        && (contains_ci(&w.title, "meeting") || contains_ci(&w.title, "webinar"))
        && !contains_ci(&w.title, "chat")
}

fn is_teams(w: &WindowInfo) -> bool {
    TEAMS_BUNDLE_IDS.contains(&w.bundle_id.as_str())
}

/// A Teams main window title: "Microsoft Teams" alone, or a navigation
/// section before " | " ("Chat | Acme | Microsoft Teams").
fn is_teams_main_title(title: &str) -> bool {
    let t = title.trim();
    if t.eq_ignore_ascii_case("microsoft teams") {
        return true;
    }
    match t.split_once(" | ") {
        Some((section, _)) => {
            let section = section.trim().to_lowercase();
            TEAMS_SECTIONS.contains(&section.as_str())
        }
        None => false,
    }
}

/// A Teams meeting window: Teams' own window, big enough, with a title that
/// is neither the main window's, a control bar nor a notification. A blank
/// title is refused: Teams keeps several big untitled helper windows.
fn is_teams_meeting(w: &WindowInfo) -> bool {
    is_teams(w)
        && w.layer == 0
        && w.width >= ZOOM_MIN_WIDTH
        && w.height >= ZOOM_MIN_HEIGHT
        && !w.title.trim().is_empty()
        && !is_teams_main_title(&w.title)
        && !contains_ci(&w.title, "sharing control bar")
        && !contains_ci(&w.title, "notification")
}

/// Which app a chosen window belongs to. A test override pointed at some
/// other app is filed with Zoom, like before.
fn call_app(w: &WindowInfo) -> CallApp {
    if is_teams(w) {
        CallApp::Teams
    } else {
        CallApp::Zoom
    }
}

/// Titles of Zoom's titled windows, for the "open but no meeting window"
/// log line.
fn zoom_titles(windows: &[WindowInfo]) -> Vec<String> {
    let mut t: Vec<String> = windows
        .iter()
        .filter(|w| w.bundle_id == ZOOM_BUNDLE_ID && !w.title.trim().is_empty())
        .map(|w| w.title.clone())
        .collect();
    t.sort();
    t.dedup();
    t
}

/// Titles of Teams' titled windows, for the same log line.
fn teams_titles(windows: &[WindowInfo]) -> Vec<String> {
    let mut t: Vec<String> = windows
        .iter()
        .filter(|w| is_teams(w) && w.layer == 0 && !w.title.trim().is_empty())
        .map(|w| w.title.clone())
        .collect();
    t.sort();
    t.dedup();
    t
}

/// Window titles of the call apps that are open, shown while no meeting
/// window is found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OpenTitles {
    zoom: Vec<String>,
    teams: Vec<String>,
}

impl OpenTitles {
    fn of(windows: &[WindowInfo]) -> Self {
        Self {
            zoom: zoom_titles(windows),
            teams: teams_titles(windows),
        }
    }

    fn is_empty(&self) -> bool {
        self.zoom.is_empty() && self.teams.is_empty()
    }
}

/// Index of the window to capture, or None when there is no (meeting) window.
fn choose_window(target: &WindowTarget, windows: &[WindowInfo]) -> Option<usize> {
    let largest = |idx: Vec<usize>| {
        idx.into_iter().max_by(|&a, &b| {
            windows[a]
                .area()
                .partial_cmp(&windows[b].area())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    };

    if target.is_zoom() {
        // Only the meeting window: a Zoom window titled meeting or webinar,
        // never a chat window. Not required to be on screen, so a full-screen
        // Zoom on another Space still counts (a grab of a minimized one fails
        // and lands in the quiet retry). Visible windows win over hidden ones.
        // A Zoom meeting window always wins; only without one does a Teams
        // meeting window count, chosen the same way.
        let pick = |is_meeting: fn(&WindowInfo) -> bool| {
            let meeting: Vec<usize> = (0..windows.len())
                .filter(|&i| is_meeting(&windows[i]))
                .collect();
            let visible: Vec<usize> = meeting
                .iter()
                .copied()
                .filter(|&i| windows[i].on_screen)
                .collect();
            largest(if visible.is_empty() { meeting } else { visible })
        };
        return pick(is_zoom_meeting).or_else(|| pick(is_teams_meeting));
    }

    if target.bundle_id.is_none() && target.app_name.is_none() {
        return None;
    }
    let matching: Vec<usize> = (0..windows.len())
        .filter(|&i| {
            let w = &windows[i];
            w.on_screen
                && w.layer == 0
                && w.area() > 0.0
                && target
                    .bundle_id
                    .as_deref()
                    .map_or(true, |b| w.bundle_id.eq_ignore_ascii_case(b))
                && target
                    .app_name
                    .as_deref()
                    .map_or(true, |n| contains_ci(&w.app_name, n))
                && target
                    .title_contains
                    .as_deref()
                    .map_or(true, |t| contains_ci(&w.title, t))
        })
        .collect();
    largest(matching)
}

/// Saved frames sort by time: `zoom-20261014-101502-123.jpg`,
/// `teams-20261014-101502-123.jpg`.
fn frame_file_name(app: CallApp, captured_at_ms: i64) -> String {
    use chrono::TimeZone;
    let t = chrono::Local
        .timestamp_millis_opt(captured_at_ms)
        .single()
        .unwrap_or_else(chrono::Local::now);
    let prefix = match app {
        CallApp::Zoom => "zoom",
        CallApp::Teams => "teams",
    };
    format!("{}-{}.jpg", prefix, t.format("%Y%m%d-%H%M%S-%3f"))
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

/// What one pass of the loop found; logged once per change.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    NoPermission,
    /// No window yet. Holds the window titles of Zoom and Teams when one of
    /// them is open but none of its windows is a meeting window.
    NoWindow(OpenTitles),
    Failed(String),
    Capturing(CallApp, String),
}

fn log_state(state: &State) {
    match state {
        State::NoPermission => log::info!(
            "vision: Screen Recording permission is missing, the assistant cannot see the window"
        ),
        State::NoWindow(titles) if titles.is_empty() => {
            log::info!("vision: no window to capture yet, waiting")
        }
        State::NoWindow(titles) => log::info!(
            "vision: Zoom or Teams is open but none of its windows is a meeting window, waiting. Zoom windows: {:?}. Teams windows: {:?}",
            titles.zoom,
            titles.teams
        ),
        State::Failed(e) => log::info!("vision: capture failed, will retry: {}", e),
        State::Capturing(app, title) => {
            log::info!("vision: capturing {} window \"{}\"", app.name(), title)
        }
    }
}

#[cfg(target_os = "macos")]
async fn run(cfg: CaptureConfig, store: FrameStore, token: CancellationToken) {
    let mut pipeline = Some(mac::Pipeline::new(&cfg));
    let mut last: Option<State> = None;
    loop {
        if token.is_cancelled() {
            break;
        }
        let started = std::time::Instant::now();
        let p = pipeline.take().unwrap_or_else(|| mac::Pipeline::new(&cfg));
        let rt = tokio::runtime::Handle::current();
        let (state, kept, wait) = match tokio::task::spawn_blocking(move || p.step(&rt)).await {
            Ok((p, outcome)) => {
                pipeline = Some(p);
                match outcome {
                    mac::Outcome::NoPermission => (State::NoPermission, None, cfg.retry_interval),
                    mac::Outcome::NoWindow(t) => (State::NoWindow(t), None, cfg.retry_interval),
                    mac::Outcome::Failed(e) => (State::Failed(e), None, cfg.retry_interval),
                    mac::Outcome::Seen { app, title, kept } => (
                        State::Capturing(app, title),
                        kept.map(|k| (app, k)),
                        cfg.interval.saturating_sub(started.elapsed()),
                    ),
                }
            }
            // The blocking pass panicked: the pipeline is rebuilt next loop.
            Err(e) => (
                State::Failed(format!("capture pass crashed: {}", e)),
                None,
                cfg.retry_interval,
            ),
        };
        // Every grabbed meeting window's title is kept for naming the meeting.
        if let State::Capturing(app, title) = &state {
            store.record_title(*app, title);
        }
        if last.as_ref() != Some(&state) {
            log_state(&state);
            last = Some(state);
        }
        // Pushed even after a stop: the frame is already on disk, and stopping
        // only means no new grabs.
        if let Some((app, (path, ms))) = kept {
            store.push_from(app, path, ms);
        }
        tokio::select! {
            _ = token.cancelled() => break,
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(not(target_os = "macos"))]
async fn run(_cfg: CaptureConfig, _store: FrameStore, _token: CancellationToken) {
    log::info!("vision: window capture is only available on macOS");
}

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use cidre::{arc, cg, cv, objc, sc};
    use tokio::runtime::Handle;
    use super::super::change::{ChangeFilter, Frame, PixelOrder};
    use super::super::encode::mac::{jpeg_bytes, Bitmap};
    use super::{call_app, choose_window, frame_file_name, CallApp, CaptureConfig, OpenTitles, WindowInfo, WindowTarget};
    use super::{GRAB_TIMEOUT, SAVE_QUALITY};

    #[derive(Clone)]
    pub enum Outcome {
        NoPermission,
        NoWindow(OpenTitles),
        Failed(String),
        /// A grab worked. `kept` is the saved file when the frame was kept.
        Seen {
            app: CallApp,
            title: String,
            kept: Option<(PathBuf, i64)>,
        },
    }

    /// Everything one pass needs, moved into and back out of `spawn_blocking`.
    pub struct Pipeline {
        filter: ChangeFilter,
        frames_dir: PathBuf,
        teams_frames_dir: PathBuf,
        target: WindowTarget,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }

    impl Pipeline {
        pub fn new(cfg: &CaptureConfig) -> Self {
            Self {
                filter: ChangeFilter::new(),
                frames_dir: cfg.frames_dir.clone(),
                teams_frames_dir: cfg.teams_frames_dir.clone(),
                target: cfg.target.clone(),
            }
        }

        /// One find / grab / filter / save pass. Runs on a blocking thread.
        pub fn step(mut self, rt: &Handle) -> (Self, Outcome) {
            // The blocking thread is reused and has no autorelease pool of its
            // own: drain what ScreenCaptureKit and ImageIO autorelease per pass.
            let outcome = objc::ar_pool(|| self.pass(rt));
            (self, outcome)
        }

        fn pass(&mut self, rt: &Handle) -> Outcome {
            // Preflight never shows the permission prompt; a missing grant just waits.
            if !unsafe { CGPreflightScreenCaptureAccess() } {
                return Outcome::NoPermission;
            }
            // Stamped before the grab, so it is when the picture was taken.
            let captured_at_ms = chrono::Local::now().timestamp_millis();
            let grabbed = rt.block_on(async {
                tokio::time::timeout(GRAB_TIMEOUT, grab(&self.target))
                    .await
                    .unwrap_or_else(|_| Err("the window grab timed out".to_string()))
            });
            let (app, title, image) = match grabbed {
                Ok(Grab::Got(app, title, image)) => (app, title, image),
                Ok(Grab::NoWindow(titles)) => return Outcome::NoWindow(titles),
                Err(e) => return Outcome::Failed(e),
            };

            let (w, h) = (image.width(), image.height());
            let mut bmp = match Bitmap::new(w, h) {
                Ok(b) => b,
                Err(e) => return Outcome::Failed(e),
            };
            bmp.draw(&image);
            let frame = Frame {
                data: bmp.rgba(),
                width: w,
                height: h,
                stride: w * 4,
                order: PixelOrder::Rgba,
            };
            let verdict = self.filter.observe(&frame);
            if !verdict.kept {
                return Outcome::Seen { app, title, kept: None };
            }
            match self.save(app, &image, captured_at_ms) {
                Ok(k) => {
                    // Only a saved frame counts as kept for later dedup.
                    self.filter.commit_kept();
                    Outcome::Seen {
                        app,
                        title,
                        kept: Some(k),
                    }
                }
                Err(e) => Outcome::Failed(e),
            }
        }

        fn save(&self, app: CallApp, image: &cg::Image, ms: i64) -> Result<(PathBuf, i64), String> {
            // Each app has its own folder, made on its first saved frame.
            let dir = match app {
                CallApp::Zoom => &self.frames_dir,
                CallApp::Teams => &self.teams_frames_dir,
            };
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
            let path = dir.join(frame_file_name(app, ms));
            let bytes = jpeg_bytes(image, SAVE_QUALITY)?;
            std::fs::write(&path, bytes)
                .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
            Ok((path, ms))
        }
    }

    enum Grab {
        Got(CallApp, String, arc::R<cg::Image>),
        /// No window to capture; holds the titles of Zoom and Teams windows.
        NoWindow(OpenTitles),
    }

    /// Finds the target window and grabs it at full pixel size.
    async fn grab(target: &WindowTarget) -> Result<Grab, String> {
        let content = sc::ShareableContent::current()
            .await
            .map_err(|e| format!("cannot list windows: {:?}", e))?;
        let windows = content.windows();
        let infos: Vec<WindowInfo> = windows.iter().map(info_of).collect();
        let Some(idx) = choose_window(target, &infos) else {
            let titles = if target.is_zoom() { OpenTitles::of(&infos) } else { OpenTitles::default() };
            return Ok(Grab::NoWindow(titles));
        };
        let window = &windows[idx];
        let filter = sc::ContentFilter::with_desktop_independent_window(window);
        let scale = sc::ShareableContent::info_for_filter(&filter).point_pixel_scale();
        let scale = if scale > 0.0 { scale as f64 } else { 1.0 };
        let frame = window.frame();
        let (w, h) = (
            (frame.size.width as f64 * scale).round() as usize,
            (frame.size.height as f64 * scale).round() as usize,
        );
        if w == 0 || h == 0 {
            return Ok(Grab::NoWindow(OpenTitles::default()));
        }
        let mut cfg = sc::StreamCfg::new();
        cfg.set_width(w);
        cfg.set_height(h);
        cfg.set_pixel_format(cv::PixelFormat::_32_BGRA);
        cfg.set_shows_cursor(false);
        let image = sc::ScreenshotManager::capture_image(&filter, &cfg)
            .await
            .map_err(|e| format!("cannot grab the window: {:?}", e))?;
        Ok(Grab::Got(call_app(&infos[idx]), infos[idx].title.clone(), image))
    }

    fn info_of(w: &sc::Window) -> WindowInfo {
        let (bundle_id, app_name) = match w.owning_app() {
            Some(app) => (app.bundle_id().to_string(), app.app_name().to_string()),
            None => (String::new(), String::new()),
        };
        let frame = w.frame();
        WindowInfo {
            bundle_id,
            app_name,
            title: w.title().map(|t| t.to_string()).unwrap_or_default(),
            layer: w.window_layer() as i64,
            on_screen: w.is_on_screen(),
            width: frame.size.width as f64,
            height: frame.size.height as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(bundle: &str, title: &str, w: f64, h: f64) -> WindowInfo {
        WindowInfo {
            bundle_id: bundle.to_string(),
            app_name: bundle.rsplit('.').next().unwrap_or("").to_string(),
            title: title.to_string(),
            layer: 0,
            on_screen: true,
            width: w,
            height: h,
        }
    }

    const Z: &str = "us.zoom.xos";

    #[test]
    fn zoom_takes_only_the_meeting_window_never_a_bigger_other_one() {
        let ws = vec![
            win(Z, "Zoom Workplace", 1600.0, 1000.0),
            win(Z, "Zoom Meeting", 900.0, 600.0),
            win("com.apple.Safari", "Meeting notes", 2000.0, 1200.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(1));
    }

    #[test]
    fn zoom_webinar_counts_and_the_largest_meeting_window_wins() {
        let ws = vec![
            win(Z, "Zoom Webinar", 800.0, 500.0),
            win(Z, "zoom meeting", 1200.0, 800.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(1));
    }

    #[test]
    fn zoom_without_a_meeting_title_is_no_meeting() {
        let ws = vec![
            win(Z, "Zoom Workplace", 1600.0, 1000.0),
            win(Z, "Zoom", 1600.0, 1000.0),
            win(Z, "", 1600.0, 1000.0),
            win(Z, "Screen share", 1700.0, 1100.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), None);
        assert_eq!(
            zoom_titles(&ws),
            vec!["Screen share", "Zoom", "Zoom Workplace"]
        );
    }

    #[test]
    fn zoom_chat_windows_are_excluded_even_with_meeting_in_the_title() {
        let ws = vec![
            win(Z, "Meeting Chat", 1000.0, 800.0),
            win(Z, "Zoom Meeting", 900.0, 600.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(1));
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws[..1]), None);
    }

    #[test]
    fn zoom_meeting_off_screen_still_counts_but_visible_wins() {
        let mut off = win(Z, "Zoom Meeting", 1600.0, 1000.0);
        off.on_screen = false;
        assert_eq!(choose_window(&WindowTarget::zoom(), &[off.clone()]), Some(0));
        let ws = vec![off, win(Z, "Zoom Meeting", 900.0, 600.0)];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(1));
    }

    #[test]
    fn zoom_ignores_small_and_non_zero_layer_windows() {
        let mut high = win(Z, "Zoom Meeting", 900.0, 600.0);
        high.layer = 25;
        let ws = vec![
            win(Z, "Zoom Meeting", 399.0, 600.0),
            win(Z, "Zoom Meeting", 900.0, 299.0),
            high,
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), None);
    }

    #[test]
    fn override_target_takes_the_largest_matching_window() {
        let t = WindowTarget::from_overrides(Some("com.apple.Preview"), None);
        assert_eq!(t.bundle_id.as_deref(), Some("com.apple.Preview"));
        let ws = vec![
            win("com.apple.Preview", "small.pdf", 300.0, 200.0),
            win("com.apple.Preview", "deck.pdf", 900.0, 700.0),
            win(Z, "Zoom Meeting", 2000.0, 1500.0),
        ];
        assert_eq!(choose_window(&t, &ws), Some(1));
        let by_title = WindowTarget::from_overrides(Some("com.apple.Preview"), Some("SMALL"));
        assert_eq!(choose_window(&by_title, &ws), Some(0));
    }

    #[test]
    fn override_by_app_name_and_unset_means_zoom() {
        let t = WindowTarget::from_overrides(Some("Preview"), None);
        assert_eq!(t.app_name.as_deref(), Some("Preview"));
        assert_eq!(t.bundle_id, None);
        let ws = vec![win("com.apple.Preview", "deck.pdf", 900.0, 700.0)];
        assert_eq!(choose_window(&t, &ws), Some(0));
        assert_eq!(WindowTarget::from_overrides(None, None), WindowTarget::zoom());
        assert_eq!(
            WindowTarget::from_overrides(Some("  "), Some("x")),
            WindowTarget::zoom()
        );
    }

    const T: &str = "com.microsoft.teams2";

    #[test]
    fn teams_meeting_window_is_chosen_when_there_is_no_zoom_meeting() {
        let ws = vec![
            win(T, "Chat | Broadcom | Microsoft Teams", 1600.0, 1000.0),
            win(Z, "Zoom Workplace", 1600.0, 1000.0),
            win(T, "Storage roadmap review | Microsoft Teams", 1200.0, 800.0),
            win("com.apple.Safari", "Meeting notes", 2000.0, 1200.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(2));
        assert_eq!(call_app(&ws[2]), CallApp::Teams);
        assert_eq!(call_app(&win(Z, "Zoom Meeting", 900.0, 600.0)), CallApp::Zoom);
    }

    #[test]
    fn classic_teams_bundle_counts_too() {
        let ws = vec![win("com.microsoft.teams", "Standup | Microsoft Teams", 900.0, 600.0)];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(0));
    }

    #[test]
    fn teams_main_window_is_never_chosen() {
        let ws = vec![
            win(T, "Chat | Broadcom | Microsoft Teams", 1600.0, 1000.0),
            win(T, "Microsoft Teams", 1600.0, 1000.0),
            win(T, "Activity | Kubernetes Level Up Program: Session #4 (External) | Microsoft Teams", 1400.0, 800.0),
            win(T, "Calendar | Microsoft Teams", 1400.0, 800.0),
            win(T, "Copilot | Microsoft Teams", 1400.0, 800.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), None);
        assert_eq!(
            teams_titles(&ws).len(),
            5,
            "the log line still lists them so a wrong rule is visible"
        );
    }

    #[test]
    fn teams_non_meeting_windows_are_never_chosen() {
        let ws = vec![
            win(T, "Storage roadmap review | Microsoft Teams", 300.0, 200.0),
            win(T, "Sharing control bar | Microsoft Teams", 900.0, 600.0),
            win(T, "Notification", 900.0, 600.0),
            win(T, "", 500.0, 500.0),
            win("com.microsoft.teams2.helper", "Storage review | Microsoft Teams", 900.0, 600.0),
            WindowInfo { layer: 25, ..win(T, "Storage review | Microsoft Teams", 900.0, 600.0) },
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), None);
    }

    #[test]
    fn teams_visible_beats_hidden_then_largest() {
        let hidden_big = WindowInfo {
            on_screen: false,
            ..win(T, "Big hidden | Microsoft Teams", 1800.0, 1100.0)
        };
        let ws = vec![
            hidden_big,
            win(T, "Small visible | Microsoft Teams", 800.0, 500.0),
            win(T, "Large visible | Microsoft Teams", 1200.0, 800.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(2));
    }

    #[test]
    fn a_zoom_meeting_wins_over_a_bigger_teams_meeting() {
        let ws = vec![
            win(T, "Storage roadmap review | Microsoft Teams", 1900.0, 1100.0),
            win(Z, "Zoom Meeting", 900.0, 600.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(1));
        // A hidden Zoom meeting still wins over a visible Teams one.
        let ws = vec![
            win(T, "Storage roadmap review | Microsoft Teams", 1900.0, 1100.0),
            WindowInfo { on_screen: false, ..win(Z, "Zoom Meeting", 900.0, 600.0) },
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(1));
    }

    #[test]
    fn an_override_target_ignores_the_teams_rules() {
        let ws = vec![win(T, "Chat | Microsoft Teams", 1600.0, 1000.0)];
        let t = WindowTarget::from_overrides(Some("com.microsoft.teams2"), None);
        assert_eq!(choose_window(&t, &ws), Some(0));
    }

    #[test]
    fn teams_frame_file_name_matches_zoom_for_the_same_instant() {
        use chrono::TimeZone;
        let ms = chrono::Local
            .with_ymd_and_hms(2026, 10, 14, 10, 15, 2)
            .unwrap()
            .timestamp_millis()
            + 123;
        assert_eq!(
            frame_file_name(CallApp::Teams, ms),
            "teams-20261014-101502-123.jpg"
        );
        assert_eq!(
            frame_file_name(CallApp::Zoom, ms),
            "zoom-20261014-101502-123.jpg"
        );
    }

    #[test]
    fn open_titles_list_both_apps() {
        let ws = vec![
            win(Z, "Zoom Workplace", 1600.0, 1000.0),
            win(T, "Chat | Broadcom | Microsoft Teams", 1600.0, 1000.0),
            win(T, "", 500.0, 500.0),
        ];
        let o = OpenTitles::of(&ws);
        assert_eq!(o.zoom, vec!["Zoom Workplace"]);
        assert_eq!(o.teams, vec!["Chat | Broadcom | Microsoft Teams"]);
        assert!(!o.is_empty());
        assert!(OpenTitles::of(&[]).is_empty());
    }

    #[test]
    fn frame_names_sort_by_time() {
        let a = frame_file_name(CallApp::Zoom, 1_700_000_000_123);
        let b = frame_file_name(CallApp::Zoom, 1_700_000_000_456);
        let c = frame_file_name(CallApp::Zoom, 1_700_000_061_000);
        assert!(a < b && b < c, "{a} {b} {c}");
        assert!(a.starts_with("zoom-") && a.ends_with(".jpg"));
    }

    #[tokio::test]
    async fn start_returns_at_once_and_stops_on_stop() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = CaptureConfig::new(dir.path().join("frames"), dir.path().join("teams-frames"));
        cfg.target = WindowTarget {
            bundle_id: Some("com.example.nothing".to_string()),
            app_name: None,
            title_contains: None,
        };
        cfg.retry_interval = Duration::from_millis(50);
        let store = FrameStore::new();
        let t0 = std::time::Instant::now();
        let handle = start(cfg, store.clone());
        assert!(t0.elapsed() < Duration::from_millis(500));
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.stop();
        assert!(store.all().is_empty());
        assert!(!dir.path().join("frames").exists(), "nothing kept, no folder");
    }

    /// Live end-to-end check against Preview showing /tmp/mf-build/deck.pdf.
    /// Run: VISION_CAPTURE_TEST=1 cargo test -p meetily --lib -- --ignored --nocapture live_preview
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn live_preview_keeps_exactly_the_two_slides() {
        use std::process::Command;
        if std::env::var("VISION_CAPTURE_TEST").as_deref() != Ok("1") {
            eprintln!("skipped: set VISION_CAPTURE_TEST=1");
            return;
        }
        // A bare test binary has no window server connection (CoreGraphics
        // asserts CGS_REQUIRE_INIT); the real app gets one from Tauri.
        #[link(name = "AppKit", kind = "framework")]
        unsafe extern "C" {
            fn NSApplicationLoad() -> bool;
        }
        assert!(unsafe { NSApplicationLoad() });

        let _ = env_logger::builder().filter_level(log::LevelFilter::Info).try_init();

        struct ClosePreview;
        impl Drop for ClosePreview {
            fn drop(&mut self) {
                let _ = Command::new("osascript")
                    .args(["-e", "tell application \"Preview\" to close (every window whose name contains \"deck\")"])
                    .status();
            }
        }
        let _close = ClosePreview;

        drop(ClosePreview); // start from a fresh window on page 1
        tokio::time::sleep(Duration::from_secs(1)).await;
        let deck = std::env::var("VISION_CAPTURE_DECK")
            .unwrap_or_else(|_| "/tmp/mf-build/deck.pdf".to_string());
        let ok = Command::new("open")
            .args(["-a", "Preview", deck.as_str()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "open -a Preview failed");
        // Wait for the deck window (a cold Preview launch takes a few seconds),
        // then show one page at a time so "Go > Down" is the next slide.
        let names = || {
            let o = Command::new("osascript")
                .args(["-e", "tell application \"System Events\" to get name of every window of process \"Preview\""])
                .output()
                .unwrap();
            String::from_utf8_lossy(&o.stdout).to_string()
        };
        let mut waited = 0;
        while !names().contains("deck") && waited < 20 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            waited += 1;
        }
        assert!(names().contains("deck"), "Preview never showed deck.pdf");
        let menu = |item: &str, menu: &str| {
            Command::new("osascript")
                .args(["-e", "tell application \"Preview\" to activate", "-e"])
                .arg(format!("tell application \"System Events\" to tell process \"Preview\" to click menu item \"{item}\" of menu \"{menu}\" of menu bar 1"))
                .output()
                .unwrap()
        };
        menu("Single Page", "View");
        menu("Up", "Go"); // Preview reopens on the last page it showed; go back to page 1
        // Preview resizes and animates its window as it opens; start only once
        // the window size has held still for two looks.
        let size = || {
            let o = Command::new("osascript")
                .args(["-e", "tell application \"System Events\" to tell process \"Preview\" to get size of window 1"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&o.stdout).to_string()
        };
        let mut last = size();
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let now = size();
            if now == last {
                break;
            }
            last = now;
        }

        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        let mut cfg = CaptureConfig::new(frames.clone(), dir.path().join("teams-frames"));
        cfg.target = WindowTarget {
            bundle_id: Some("com.apple.Preview".to_string()),
            app_name: None,
            title_contains: Some("deck".to_string()),
        };
        cfg.retry_interval = Duration::from_secs(1);
        let store = FrameStore::new();
        let handle = start(cfg, store.clone());

        tokio::time::sleep(Duration::from_secs(4)).await;
        for f in store.all() {
            eprintln!("before advance: kept {}", f.path.display());
        }
        assert_eq!(store.all().len(), 1, "first page only, still seconds add nothing");

        let adv = menu("Down", "Go");
        eprintln!("advance: status={:?} stderr={}", adv.status.code(), String::from_utf8_lossy(&adv.stderr));
        tokio::time::sleep(Duration::from_secs(6)).await;
        handle.stop();

        let kept = store.all();
        let on_disk = std::fs::read_dir(&frames).map(|d| d.count()).unwrap_or(0);
        for f in &kept {
            let sz = std::fs::metadata(&f.path).map(|m| m.len()).unwrap_or(0);
            eprintln!("kept seq={} at={} {} ({} bytes)", f.seq, f.captured_at_ms, f.path.display(), sz);
        }
        // Keep copies where the report can point at them.
        let out = std::path::Path::new("/tmp/mf-build/live-frames");
        let _ = std::fs::create_dir_all(out);
        for f in &kept {
            let _ = std::fs::copy(&f.path, out.join(f.path.file_name().unwrap()));
        }
        assert_eq!(kept.len(), 2, "slide 1 and slide 2");
        assert_eq!(on_disk, 2);
        let a = super::super::encode::for_send(&kept[0].path).unwrap();
        assert!(!a.base64.is_empty());
    }
}
