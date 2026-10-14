// assistant/vision/change.rs
//
// Which captured frames to keep. Joseph chose the /watch skill's rule,
// reimplemented here on raw pixel buffers:
//   - scene gate: ffmpeg's `select='gt(scene,0.20)'` score against the
//     previous CAPTURED frame (ffmpeg libavfilter vf_select.c get_scene_score);
//   - dedup: 16x16 grayscale thumbnail, mean absolute difference from the last
//     KEPT frame's thumbnail, dropped when <= 2.0
//     (~/.claude/skills/watch/scripts/frames.py: SCENE_THRESHOLD, DEDUP_THUMB,
//     DEDUP_THRESHOLD, _frame_delta, _dedupe_by_deltas).
// Documented difference: ffmpeg scales the dedup thumbnail with bicubic; this
// box-averages into 16x16 cells.
//
// Everything here is pure: tests drive it with plain byte arrays.

pub const SCENE_THRESHOLD: f64 = 0.20;
pub const DEDUP_THUMB: usize = 16;
pub const DEDUP_THRESHOLD: f64 = 2.0;

/// Byte order of a 4-byte pixel. Alpha (byte 3) is ignored either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelOrder {
    Rgba,
    Bgra,
}

/// A borrowed pixel buffer: `height` rows of `stride` bytes, the first
/// `width * 4` bytes of each row being pixels.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub data: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub order: PixelOrder,
}

/// What the filter decided about one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    /// The scene score against the previous captured frame (1.0 for the first
    /// frame, which is always kept).
    pub score: f64,
    /// Mean absolute thumbnail difference from the last kept frame, when the
    /// frame was a candidate.
    pub thumb_delta: Option<f64>,
    pub kept: bool,
}

/// The last captured frame, kept as compact RGB-ordered pixels so the next
/// comparison does not depend on the caller's buffer.
struct Captured {
    data: Vec<u8>,
    width: usize,
    height: usize,
    stride: usize,
    order: PixelOrder,
}

impl Captured {
    fn of(f: &Frame) -> Self {
        Self {
            data: f.data.to_vec(),
            width: f.width,
            height: f.height,
            stride: f.stride,
            order: f.order,
        }
    }

    fn as_frame(&self) -> Frame<'_> {
        Frame {
            data: &self.data,
            width: self.width,
            height: self.height,
            stride: self.stride,
            order: self.order,
        }
    }
}

#[derive(Default)]
pub struct ChangeFilter {
    prev: Option<Captured>,
    /// ffmpeg's prev_mafd: the mafd of the last comparison.
    prev_mafd: f64,
    /// Thumbnail of the last KEPT frame.
    last_kept_thumb: Option<Vec<u8>>,
}

impl ChangeFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds the next captured frame (kept or not, it becomes the "previous
    /// captured" frame) and says whether to keep it.
    pub fn observe(&mut self, frame: &Frame) -> Verdict {
        let verdict = match &self.prev {
            None => {
                self.last_kept_thumb = Some(thumbnail(frame));
                Verdict {
                    score: 1.0,
                    thumb_delta: None,
                    kept: true,
                }
            }
            Some(prev) => {
                let prev_frame = prev.as_frame();
                let score = if prev.width != frame.width || prev.height != frame.height {
                    self.prev_mafd = 0.0;
                    1.0
                } else {
                    let m = mafd(&prev_frame, frame);
                    let diff = (m - self.prev_mafd).abs();
                    self.prev_mafd = m;
                    (m.min(diff) / 100.0).clamp(0.0, 1.0)
                };
                if score > SCENE_THRESHOLD {
                    let thumb = thumbnail(frame);
                    let delta = self
                        .last_kept_thumb
                        .as_deref()
                        .map(|last| thumb_delta(&thumb, last))
                        .unwrap_or(f64::INFINITY);
                    let kept = delta > DEDUP_THRESHOLD;
                    if kept {
                        self.last_kept_thumb = Some(thumb);
                    }
                    Verdict {
                        score,
                        thumb_delta: Some(delta),
                        kept,
                    }
                } else {
                    Verdict {
                        score,
                        thumb_delta: None,
                        kept: false,
                    }
                }
            }
        };
        self.prev = Some(Captured::of(frame));
        verdict
    }
}

fn rgb_at(f: &Frame, x: usize, y: usize) -> [u32; 3] {
    let i = y * f.stride + x * 4;
    let p = &f.data[i..i + 3];
    match f.order {
        PixelOrder::Rgba => [p[0] as u32, p[1] as u32, p[2] as u32],
        PixelOrder::Bgra => [p[2] as u32, p[1] as u32, p[0] as u32],
    }
}

/// ffmpeg `get_scene_score`'s mafd: sum of absolute byte differences over the
/// R, G and B bytes of every pixel / (width * height * 3). Alpha is ignored.
/// Both frames must be the same size.
pub fn mafd(a: &Frame, b: &Frame) -> f64 {
    debug_assert_eq!((a.width, a.height), (b.width, b.height));
    if a.width == 0 || a.height == 0 {
        return 0.0;
    }
    let row_bytes = a.width * 4;
    let mut sum: u64 = 0;
    for y in 0..a.height {
        let ra = &a.data[y * a.stride..y * a.stride + row_bytes];
        let rb = &b.data[y * b.stride..y * b.stride + row_bytes];
        // Differences are symmetric in channel order, so only alpha is skipped.
        let mut row: u32 = 0;
        for (pa, pb) in ra.chunks_exact(4).zip(rb.chunks_exact(4)) {
            row += pa[0].abs_diff(pb[0]) as u32
                + pa[1].abs_diff(pb[1]) as u32
                + pa[2].abs_diff(pb[2]) as u32;
        }
        sum += row as u64;
    }
    sum as f64 / (a.width * a.height * 3) as f64
}

/// 16x16 grayscale thumbnail (256 bytes): box-average each cell, luma =
/// 0.299R + 0.587G + 0.114B, rounded half up, in integer math so a flat
/// 127.5 average rounds to 128 on every platform.
pub fn thumbnail(f: &Frame) -> Vec<u8> {
    let n = DEDUP_THUMB;
    let mut out = vec![0u8; n * n];
    if f.width == 0 || f.height == 0 {
        return out;
    }
    for cy in 0..n {
        let y0 = (cy * f.height / n).min(f.height - 1);
        let y1 = ((cy + 1) * f.height / n).clamp(y0 + 1, f.height);
        for cx in 0..n {
            let x0 = (cx * f.width / n).min(f.width - 1);
            let x1 = ((cx + 1) * f.width / n).clamp(x0 + 1, f.width);
            let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let [pr, pg, pb] = rgb_at(f, x, y);
                    r += pr as u64;
                    g += pg as u64;
                    b += pb as u64;
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u64;
            let num = 299 * r + 587 * g + 114 * b;
            let den = 1000 * count;
            out[cy * n + cx] = ((2 * num + den) / (2 * den)).min(255) as u8;
        }
    }
    out
}

/// Mean absolute per-pixel difference (0-255) between two thumbnails; a length
/// mismatch counts as maximally different (frames.py `_frame_delta`).
pub fn thumb_delta(a: &[u8], b: &[u8]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return f64::INFINITY;
    }
    let sum: u64 = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f64 / a.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, v: u8) -> Vec<u8> {
        let mut d = vec![v; w * h * 4];
        for px in d.chunks_mut(4) {
            px[3] = 255;
        }
        d
    }

    fn frame(d: &[u8], w: usize, h: usize) -> Frame<'_> {
        Frame {
            data: d,
            width: w,
            height: h,
            stride: w * 4,
            order: PixelOrder::Rgba,
        }
    }

    fn checker(w: usize, h: usize, invert: bool) -> Vec<u8> {
        let mut d = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let on = ((x + y) % 2 == 0) != invert;
                let v = if on { 255 } else { 0 };
                let i = (y * w + x) * 4;
                d[i] = v;
                d[i + 1] = v;
                d[i + 2] = v;
            }
        }
        d
    }

    #[test]
    fn identical_frames_score_zero_and_are_not_kept() {
        let d = solid(64, 64, 90);
        let mut f = ChangeFilter::new();
        assert!(f.observe(&frame(&d, 64, 64)).kept, "first frame is kept");
        let v = f.observe(&frame(&d, 64, 64));
        assert_eq!(v.score, 0.0);
        assert!(!v.kept);
        assert_eq!(v.thumb_delta, None, "not a candidate, no dedup run");
    }

    #[test]
    fn black_to_white_scores_one_and_is_kept() {
        let (b, w) = (solid(64, 64, 0), solid(64, 64, 255));
        assert_eq!(mafd(&frame(&b, 64, 64), &frame(&w, 64, 64)), 255.0);
        let mut f = ChangeFilter::new();
        f.observe(&frame(&b, 64, 64));
        let v = f.observe(&frame(&w, 64, 64));
        assert_eq!(v.score, 1.0);
        assert_eq!(v.thumb_delta, Some(255.0));
        assert!(v.kept);
    }

    #[test]
    fn small_motion_is_below_the_scene_threshold() {
        let a = solid(64, 64, 128);
        let mut b = a.clone();
        for y in 0..8 {
            for x in 0..8 {
                let i = (y * 64 + x) * 4;
                b[i] = 255;
                b[i + 1] = 255;
                b[i + 2] = 255;
            }
        }
        assert_eq!(mafd(&frame(&a, 64, 64), &frame(&b, 64, 64)), 1.984375);
        let mut f = ChangeFilter::new();
        f.observe(&frame(&a, 64, 64));
        let v = f.observe(&frame(&b, 64, 64));
        assert_eq!(v.score, 0.01984375);
        assert!(!v.kept);
    }

    #[test]
    fn steady_change_scores_on_the_jump_not_the_drift() {
        let (a, b, c) = (solid(64, 64, 0), solid(64, 64, 50), solid(64, 64, 100));
        let mut f = ChangeFilter::new();
        f.observe(&frame(&a, 64, 64));
        let v2 = f.observe(&frame(&b, 64, 64));
        assert_eq!(v2.score, 0.5);
        assert!(v2.kept);
        // mafd 50, prev_mafd 50: diff 0, so the score is 0.
        let v3 = f.observe(&frame(&c, 64, 64));
        assert_eq!(v3.score, 0.0);
        assert!(!v3.kept);
    }

    #[test]
    fn candidate_with_same_thumbnail_is_dropped() {
        let (a, b) = (checker(64, 64, false), checker(64, 64, true));
        let ta = thumbnail(&frame(&a, 64, 64));
        let tb = thumbnail(&frame(&b, 64, 64));
        assert_eq!(ta.len(), 256);
        assert!(ta.iter().chain(tb.iter()).all(|&v| v == 128));
        let mut f = ChangeFilter::new();
        assert!(f.observe(&frame(&a, 64, 64)).kept);
        let v = f.observe(&frame(&b, 64, 64));
        assert_eq!(v.score, 1.0);
        assert_eq!(v.thumb_delta, Some(0.0));
        assert!(!v.kept);
    }

    #[test]
    fn dedup_boundary_is_inclusive_at_two() {
        let a = vec![100u8; 256];
        assert_eq!(thumb_delta(&a, &vec![102u8; 256]), 2.0);
        assert_eq!(thumb_delta(&a, &vec![103u8; 256]), 3.0);
        assert_eq!(thumb_delta(&a, &vec![98u8; 256]), 2.0);
        assert_eq!(thumb_delta(&a, &vec![100u8; 255]), f64::INFINITY);
    }

    #[test]
    fn dedup_decision_uses_the_boundary() {
        // 0 -> 2 scores 0.02 (not a candidate), so drive the dedup through a
        // filter whose scene gate passes: a size change always passes.
        let a = solid(64, 64, 100);
        let same = solid(32, 32, 102);
        let more = solid(32, 32, 103);
        let mut f = ChangeFilter::new();
        f.observe(&frame(&a, 64, 64));
        let v = f.observe(&frame(&same, 32, 32));
        assert_eq!(v.score, 1.0);
        assert_eq!(v.thumb_delta, Some(2.0));
        assert!(!v.kept);
        let mut g = ChangeFilter::new();
        g.observe(&frame(&a, 64, 64));
        let v = g.observe(&frame(&more, 32, 32));
        assert_eq!(v.thumb_delta, Some(3.0));
        assert!(v.kept);
    }

    #[test]
    fn size_change_scores_one_and_dedup_decides() {
        let (a, b) = (solid(64, 64, 128), solid(32, 32, 128));
        let mut f = ChangeFilter::new();
        f.observe(&frame(&a, 64, 64));
        let v = f.observe(&frame(&b, 32, 32));
        assert_eq!(v.score, 1.0);
        assert_eq!(v.thumb_delta, Some(0.0));
        assert!(!v.kept);
    }

    #[test]
    fn stride_padding_and_bgra_order_are_handled() {
        // 4x2 image with 8 bytes of padding per row; BGRA bytes of pure red.
        let mut d = vec![7u8; 24 * 2];
        for y in 0..2 {
            for x in 0..4 {
                let i = y * 24 + x * 4;
                d[i..i + 4].copy_from_slice(&[0, 0, 255, 255]);
            }
        }
        let fr = Frame {
            data: &d,
            width: 4,
            height: 2,
            stride: 24,
            order: PixelOrder::Bgra,
        };
        let t = thumbnail(&fr);
        assert_eq!(t.len(), 256);
        // luma of pure red = 0.299 * 255 = 76.2 -> 76
        assert!(t.iter().all(|&v| v == 76));
        let white = solid(4, 2, 255);
        assert_eq!(mafd(&fr, &frame(&white, 4, 2)), (255.0 + 255.0) / 3.0);
    }
}
