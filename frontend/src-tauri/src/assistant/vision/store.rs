// assistant/vision/store.rs
//
// The frames kept from the Zoom window during one recording, shared between
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

/// A frame the change logic kept, saved full-size on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptFrame {
    pub seq: u64,
    pub path: PathBuf,
    pub captured_at_ms: i64,
}

#[derive(Clone, Default)]
pub struct FrameStore {
    inner: Arc<Mutex<Vec<KeptFrame>>>,
}

impl FrameStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a kept frame and returns its seq.
    pub fn push(&self, path: PathBuf, captured_at_ms: i64) -> u64 {
        let mut v = self.inner.lock().unwrap();
        let seq = v.last().map(|f| f.seq).unwrap_or(0) + 1;
        v.push(KeptFrame {
            seq,
            path,
            captured_at_ms,
        });
        seq
    }

    /// Seq of the newest kept frame, 0 when none.
    pub fn latest_seq(&self) -> u64 {
        self.inner.lock().unwrap().last().map(|f| f.seq).unwrap_or(0)
    }

    /// Every frame with seq > `seq`, oldest first.
    pub fn since(&self, seq: u64) -> Vec<KeptFrame> {
        self.inner
            .lock()
            .unwrap()
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
            .last()
            .filter(|f| f.seq > seq)
            .cloned()
    }

    pub fn all(&self) -> Vec<KeptFrame> {
        self.inner.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
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
}
