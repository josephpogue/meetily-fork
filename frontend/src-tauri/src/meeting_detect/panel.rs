// meeting_detect/panel.rs
//
// The floating meeting prompt: which cards it shows (from the detection rules) and where its
// window sits. See docs/superpowers/plans/2026-11-21-floating-meeting-prompt-brief.md.

use crate::audio::call_apps::CallAppId;

use super::rules::Rules;

/// Gap between the top of the screen's visible area (below the menu bar) and the window, in points.
pub const GAP_TOP: f64 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    /// "<App> meeting started": Record and Dismiss.
    Started,
    /// "<App> meeting ended": Stop recording and Keep recording.
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub app: CallAppId,
    pub kind: CardKind,
    pub title: Option<String>,
}

/// A screen area, top-left origin, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The cards the floating window shows now, Zoom before Teams. Empty means the window is hidden.
pub fn cards(_rules: &Rules) -> Vec<Card> {
    unimplemented!("not implemented")
}

/// Top-left point for a `width` x `height` window: centered horizontally in `visible`, its top
/// `GAP_TOP` below `visible.y`; x is `visible.x` when the window is wider than `visible`.
pub fn place(_visible: Rect, _width: f64, _height: f64) -> (f64, f64) {
    unimplemented!("not implemented")
}

#[cfg(test)]
#[path = "tests/panel.rs"]
mod tests;
