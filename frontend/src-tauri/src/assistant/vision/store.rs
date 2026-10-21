// assistant/vision/store.rs
//
// The frames kept from the Zoom or Teams window during one recording, shared between
// the capture task (which pushes) and the answer lanes (which read what is
// new since their own last turn). Each kept frame gets a sequence number
// starting at 1, so a lane's cursor is just the last seq it sent (0 = none).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// One picture ready to go inside a `claude` user message as an image
/// content block (`{"type":"image","source":{"type":"base64",...}}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBlock {
    /// "image/jpeg" or "image/png".
    pub media_type: String,
    /// Base64 of the encoded image bytes, no data: prefix.
    pub base64: String,
}

/// Which call app a frame came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallApp {
    Zoom,
    Teams,
}

impl CallApp {
    /// The name used in log lines and image labels.
    pub fn name(self) -> &'static str {
        match self {
            CallApp::Zoom => "Zoom",
            CallApp::Teams => "Teams",
        }
    }
}

/// A frame the change logic kept, saved full-size on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptFrame {
    pub seq: u64,
    pub path: PathBuf,
    pub captured_at_ms: i64,
    pub app: CallApp,
}

/// At most this many distinct (app, title) pairs are remembered.
const MAX_TITLES: usize = 20;

#[derive(Default)]
struct Inner {
    frames: Vec<KeptFrame>,
    titles: Vec<(CallApp, String)>,
}

#[derive(Clone, Default)]
pub struct FrameStore {
    inner: Arc<Mutex<Inner>>,
}

impl FrameStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a kept Zoom frame and returns its seq.
    pub fn push(&self, path: PathBuf, captured_at_ms: i64) -> u64 {
        self.push_from(CallApp::Zoom, path, captured_at_ms)
    }

    /// Records a kept frame from `app` and returns its seq.
    pub fn push_from(&self, app: CallApp, path: PathBuf, captured_at_ms: i64) -> u64 {
        let mut v = self.inner.lock().unwrap();
        let seq = v.frames.last().map(|f| f.seq).unwrap_or(0) + 1;
        v.frames.push(KeptFrame {
            seq,
            path,
            captured_at_ms,
            app,
        });
        seq
    }

    /// Remembers the title of a meeting window the capture grabbed. Repeats
    /// are ignored, and so is anything past the first 20 distinct pairs.
    pub fn record_title(&self, app: CallApp, title: &str) {
        let mut v = self.inner.lock().unwrap();
        if v.titles.len() < MAX_TITLES && !v.titles.iter().any(|(a, t)| *a == app && t == title) {
            v.titles.push((app, title.to_string()));
        }
    }

    /// Distinct (app, title) pairs of the meeting windows grabbed so far, in
    /// first-seen order.
    pub fn window_titles(&self) -> Vec<(CallApp, String)> {
        self.inner.lock().unwrap().titles.clone()
    }

    /// Returns the window titles and empties them, so one recording's titles
    /// name at most one meeting.
    pub fn take_window_titles(&self) -> Vec<(CallApp, String)> {
        std::mem::take(&mut self.inner.lock().unwrap().titles)
    }

    /// Seq of the newest kept frame, 0 when none.
    pub fn latest_seq(&self) -> u64 {
        self.inner.lock().unwrap().frames.last().map(|f| f.seq).unwrap_or(0)
    }

    /// Every frame with seq > `seq`, oldest first.
    pub fn since(&self, seq: u64) -> Vec<KeptFrame> {
        self.inner
            .lock()
            .unwrap()
            .frames
            .iter()
            .filter(|f| f.seq > seq)
            .cloned()
            .collect()
    }

    /// The newest frame with seq > `seq`, if any.
    pub fn newest_since(&self, seq: u64) -> Option<KeptFrame> {
        self.inner
            .lock()
            .unwrap()
            .frames
            .last()
            .filter(|f| f.seq > seq)
            .cloned()
    }

    pub fn all(&self) -> Vec<KeptFrame> {
        self.inner.lock().unwrap().frames.clone()
    }

    pub fn clear(&self) {
        let mut v = self.inner.lock().unwrap();
        v.frames.clear();
        v.titles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seqs_start_at_one_and_since_is_exclusive() {
        let s = FrameStore::new();
        assert_eq!(s.latest_seq(), 0);
        assert_eq!(s.newest_since(0), None);
        assert_eq!(s.push("/a".into(), 1), 1);
        assert_eq!(s.push("/b".into(), 2), 2);
        assert_eq!(s.push("/c".into(), 3), 3);
        let since1: Vec<u64> = s.since(1).iter().map(|f| f.seq).collect();
        assert_eq!(since1, vec![2, 3]);
        assert_eq!(s.newest_since(1).map(|f| f.seq), Some(3));
        assert_eq!(s.newest_since(3), None);
        assert!(s.since(3).is_empty());
    }

    #[test]
    fn frames_remember_their_app() {
        let s = FrameStore::new();
        s.push_from(CallApp::Teams, "/t".into(), 1);
        s.push("/z".into(), 2);
        let apps: Vec<CallApp> = s.all().iter().map(|f| f.app).collect();
        assert_eq!(apps, vec![CallApp::Teams, CallApp::Zoom]);
    }

    #[test]
    fn window_titles_are_distinct_pairs_in_first_seen_order() {
        let s = FrameStore::new();
        s.record_title(CallApp::Teams, "Review | Microsoft Teams");
        s.record_title(CallApp::Zoom, "Zoom Meeting");
        s.record_title(CallApp::Teams, "Review | Microsoft Teams");
        s.record_title(CallApp::Zoom, "Review | Microsoft Teams");
        assert_eq!(
            s.window_titles(),
            vec![
                (CallApp::Teams, "Review | Microsoft Teams".to_string()),
                (CallApp::Zoom, "Zoom Meeting".to_string()),
                (CallApp::Zoom, "Review | Microsoft Teams".to_string()),
            ]
        );
    }

    #[test]
    fn taking_the_window_titles_empties_them() {
        let s = FrameStore::new();
        s.record_title(CallApp::Zoom, "A");
        assert_eq!(s.take_window_titles(), vec![(CallApp::Zoom, "A".to_string())]);
        assert!(s.take_window_titles().is_empty());
        assert!(s.window_titles().is_empty());
    }

    #[test]
    fn window_titles_stop_at_twenty_and_clear_empties_them() {
        let s = FrameStore::new();
        for i in 0..25 {
            s.record_title(CallApp::Teams, &format!("Meeting {}", i));
        }
        let t = s.window_titles();
        assert_eq!(t.len(), 20);
        assert_eq!(t[0].1, "Meeting 0");
        assert_eq!(t[19].1, "Meeting 19");
        s.clear();
        assert!(s.window_titles().is_empty());
    }
}
