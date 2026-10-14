// assistant/vision/mod.rs
//
// The assistant sees the Zoom meeting window: a capture task grabs that one
// window about once a second while a recording runs, keeps a frame only when
// it changed (same rule as the /watch skill), saves kept frames full-size in
// the meeting's folder, and the answer lanes send the new ones to claude as
// image blocks inside the turn's message.

pub mod capture;
pub mod change;
pub mod encode;
pub mod permission;
pub mod store;

pub use store::{FrameStore, ImageBlock, KeptFrame};
