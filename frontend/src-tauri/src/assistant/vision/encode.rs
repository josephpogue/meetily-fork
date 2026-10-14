// assistant/vision/encode.rs
//
// Turns a kept frame on disk into an image block for a claude message.

use std::path::Path;

use super::store::ImageBlock;

/// Sent images are downscaled to at most this many pixels wide.
pub const SEND_MAX_WIDTH: u32 = 1280;

/// Reads the full-size frame at `path`, downscales it to at most
/// `SEND_MAX_WIDTH` wide (aspect kept, never upscaled) and encodes it for
/// sending. Contract only; the capture slice implements it.
pub fn for_send(path: &Path) -> Result<ImageBlock, String> {
    Err(format!("vision encode not built yet: {}", path.display()))
}
