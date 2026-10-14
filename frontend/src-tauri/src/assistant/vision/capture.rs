// assistant/vision/capture.rs
//
// The capture task: finds the Zoom meeting window, grabs it about once a
// second, keeps changed frames, saves them and records them in a FrameStore.

use std::path::PathBuf;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::store::FrameStore;

/// Zoom's bundle id on macOS.
const ZOOM_BUNDLE_ID: &str = "us.zoom.xos";
/// A Zoom window smaller than this (points) is a popover, not the meeting.
const ZOOM_MIN_WIDTH: f64 = 400.0;
const ZOOM_MIN_HEIGHT: f64 = 300.0;
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
    pub fn from_env_or_zoom() -> Self {
        Self::from_overrides(
            std::env::var("MEETILY_VISION_APP").ok().as_deref(),
            std::env::var("MEETILY_VISION_TITLE").ok().as_deref(),
        )
    }

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

    /// True for the production Zoom target, which has the meeting-window
    /// rules; any other target takes its largest matching window.
    fn is_zoom(&self) -> bool {
        self.bundle_id.as_deref() == Some(ZOOM_BUNDLE_ID)
            && self.app_name.is_none()
            && self.title_contains.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Kept frames are written here full-size. Created on first keep.
    pub frames_dir: PathBuf,
    pub target: WindowTarget,
    /// Between grabs while the window is there (about 1s).
    pub interval: Duration,
    /// Between looks for the window while it is missing (a few seconds).
    pub retry_interval: Duration,
}

impl CaptureConfig {
    pub fn new(frames_dir: PathBuf) -> Self {
        Self {
            frames_dir,
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
        let zoom: Vec<usize> = (0..windows.len())
            .filter(|&i| {
                let w = &windows[i];
                w.bundle_id == ZOOM_BUNDLE_ID
                    && w.on_screen
                    && w.layer == 0
                    && w.width >= ZOOM_MIN_WIDTH
                    && w.height >= ZOOM_MIN_HEIGHT
            })
            .collect();
        let meeting: Vec<usize> = zoom
            .iter()
            .copied()
            .filter(|&i| {
                contains_ci(&windows[i].title, "meeting") || contains_ci(&windows[i].title, "webinar")
            })
            .collect();
        if let Some(i) = largest(meeting) {
            return Some(i);
        }
        let other: Vec<usize> = zoom
            .into_iter()
            .filter(|&i| {
                let t = windows[i].title.trim().to_lowercase();
                !t.is_empty() && t != "zoom workplace" && t != "zoom"
            })
            .collect();
        return largest(other);
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

/// Saved frames sort by time: `zoom-20261014-101502-123.jpg`.
fn frame_file_name(captured_at_ms: i64) -> String {
    use chrono::TimeZone;
    let t = chrono::Local
        .timestamp_millis_opt(captured_at_ms)
        .single()
        .unwrap_or_else(chrono::Local::now);
    format!("zoom-{}.jpg", t.format("%Y%m%d-%H%M%S-%3f"))
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

/// What one pass of the loop found; logged once per change.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    NoPermission,
    NoWindow,
    Failed(String),
    Capturing(String),
}

fn log_state(state: &State) {
    match state {
        State::NoPermission => log::info!(
            "vision: Screen Recording permission is missing, the assistant cannot see the window"
        ),
        State::NoWindow => log::info!("vision: no window to capture yet, waiting"),
        State::Failed(e) => log::info!("vision: capture failed, will retry: {}", e),
        State::Capturing(title) => log::info!("vision: capturing window \"{}\"", title),
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
        let t = token.clone();
        let (state, kept, wait) = match tokio::task::spawn_blocking(move || p.step(&rt, &t)).await {
            Ok((p, outcome)) => {
                pipeline = Some(p);
                match outcome {
                    mac::Outcome::NoPermission => (State::NoPermission, None, cfg.retry_interval),
                    mac::Outcome::NoWindow => (State::NoWindow, None, cfg.retry_interval),
                    mac::Outcome::Failed(e) => (State::Failed(e), None, cfg.retry_interval),
                    mac::Outcome::Seen { title, kept } => (
                        State::Capturing(title),
                        kept,
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
        if last.as_ref() != Some(&state) {
            log_state(&state);
            last = Some(state);
        }
        if let Some((path, ms)) = kept {
            if !token.is_cancelled() {
                store.push(path, ms);
            }
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

    use cidre::{arc, cg, cv, sc};
    use tokio::runtime::Handle;
    use tokio_util::sync::CancellationToken;

    use super::super::change::{ChangeFilter, Frame, PixelOrder};
    use super::super::encode::mac::{jpeg_bytes, Bitmap};
    use super::{choose_window, frame_file_name, CaptureConfig, WindowInfo, WindowTarget};
    use super::{GRAB_TIMEOUT, SAVE_QUALITY};

    pub enum Outcome {
        NoPermission,
        NoWindow,
        Failed(String),
        /// A grab worked. `kept` is the saved file when the frame was kept.
        Seen {
            title: String,
            kept: Option<(PathBuf, i64)>,
        },
    }

    /// Everything one pass needs, moved into and back out of `spawn_blocking`.
    pub struct Pipeline {
        filter: ChangeFilter,
        frames_dir: PathBuf,
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
                target: cfg.target.clone(),
            }
        }

        /// One find / grab / filter / save pass. Runs on a blocking thread.
        pub fn step(mut self, rt: &Handle, token: &CancellationToken) -> (Self, Outcome) {
            let outcome = self.pass(rt, token);
            (self, outcome)
        }

        fn pass(&mut self, rt: &Handle, token: &CancellationToken) -> Outcome {
            // Preflight never shows the permission prompt; a missing grant just waits.
            if !unsafe { CGPreflightScreenCaptureAccess() } {
                return Outcome::NoPermission;
            }
            let grabbed = rt.block_on(async {
                tokio::time::timeout(GRAB_TIMEOUT, grab(&self.target))
                    .await
                    .unwrap_or_else(|_| Err("the window grab timed out".to_string()))
            });
            let (title, image) = match grabbed {
                Ok(Some(g)) => g,
                Ok(None) => return Outcome::NoWindow,
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
            if !verdict.kept || token.is_cancelled() {
                return Outcome::Seen { title, kept: None };
            }
            match self.save(&image) {
                Ok(k) => Outcome::Seen {
                    title,
                    kept: Some(k),
                },
                Err(e) => Outcome::Failed(e),
            }
        }

        fn save(&self, image: &cg::Image) -> Result<(PathBuf, i64), String> {
            let ms = chrono::Local::now().timestamp_millis();
            std::fs::create_dir_all(&self.frames_dir)
                .map_err(|e| format!("cannot create {}: {}", self.frames_dir.display(), e))?;
            let path = self.frames_dir.join(frame_file_name(ms));
            let bytes = jpeg_bytes(image, SAVE_QUALITY)?;
            std::fs::write(&path, bytes)
                .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
            Ok((path, ms))
        }
    }

    /// Finds the target window and grabs it at full pixel size. `Ok(None)`
    /// when there is no such window.
    async fn grab(target: &WindowTarget) -> Result<Option<(String, arc::R<cg::Image>)>, String> {
        let content = sc::ShareableContent::current()
            .await
            .map_err(|e| format!("cannot list windows: {:?}", e))?;
        let windows = content.windows();
        let infos: Vec<WindowInfo> = windows.iter().map(info_of).collect();
        let Some(idx) = choose_window(target, &infos) else {
            return Ok(None);
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
            return Ok(None);
        }
        let mut cfg = sc::StreamCfg::new();
        cfg.set_width(w);
        cfg.set_height(h);
        cfg.set_pixel_format(cv::PixelFormat::_32_BGRA);
        cfg.set_shows_cursor(false);
        let image = sc::ScreenshotManager::capture_image(&filter, &cfg)
            .await
            .map_err(|e| format!("cannot grab the window: {:?}", e))?;
        Ok(Some((infos[idx].title.clone(), image)))
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
    fn zoom_prefers_the_meeting_window_over_a_bigger_one() {
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
    fn zoom_home_and_untitled_windows_are_no_meeting() {
        let ws = vec![
            win(Z, "Zoom Workplace", 1600.0, 1000.0),
            win(Z, "Zoom", 1600.0, 1000.0),
            win(Z, "", 1600.0, 1000.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), None);
    }

    #[test]
    fn zoom_falls_back_to_the_largest_titled_window() {
        let ws = vec![
            win(Z, "Zoom Workplace", 1600.0, 1000.0),
            win(Z, "Screen share", 700.0, 400.0),
            win(Z, "Chat", 500.0, 450.0),
        ];
        assert_eq!(choose_window(&WindowTarget::zoom(), &ws), Some(1));
    }

    #[test]
    fn zoom_ignores_small_offscreen_and_non_zero_layer_windows() {
        let mut off = win(Z, "Zoom Meeting", 900.0, 600.0);
        off.on_screen = false;
        let mut high = win(Z, "Zoom Meeting", 900.0, 600.0);
        high.layer = 25;
        let ws = vec![
            win(Z, "Zoom Meeting", 399.0, 600.0),
            win(Z, "Zoom Meeting", 900.0, 299.0),
            off,
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

    #[test]
    fn frame_names_sort_by_time() {
        let a = frame_file_name(1_700_000_000_123);
        let b = frame_file_name(1_700_000_000_456);
        let c = frame_file_name(1_700_000_061_000);
        assert!(a < b && b < c, "{a} {b} {c}");
        assert!(a.starts_with("zoom-") && a.ends_with(".jpg"));
    }

    #[tokio::test]
    async fn start_returns_at_once_and_stops_on_stop() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = CaptureConfig::new(dir.path().join("frames"));
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
        let mut cfg = CaptureConfig::new(frames.clone());
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
