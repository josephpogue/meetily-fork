// assistant/vision/capture.rs
//
// The capture task: finds the Zoom meeting window, grabs it about once a
// second, keeps changed frames, saves them and records them in a FrameStore.

use std::path::PathBuf;
use std::time::Duration;

use super::store::FrameStore;

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
            bundle_id: Some("us.zoom.xos".to_string()),
            app_name: None,
            title_contains: None,
        }
    }

    pub fn from_env_or_zoom() -> Self {
        Self::zoom()
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
    _private: (),
}

impl CaptureHandle {
    pub fn stop(&self) {}
}

/// Starts the capture task on its own tokio task and returns at once. It
/// never blocks a caller and never fails loudly: no window, no permission or
/// a failed grab only logs and waits. Contract only; the capture slice
/// implements it.
pub fn start(cfg: CaptureConfig, store: FrameStore) -> CaptureHandle {
    let _ = (cfg, store);
    CaptureHandle { _private: () }
}
