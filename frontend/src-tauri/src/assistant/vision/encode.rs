// assistant/vision/encode.rs
//
// Turns a kept frame on disk into an image block for a claude message, and
// holds the small CoreGraphics / ImageIO helpers the capture task shares
// (pixel access for the change filter, JPEG writing for kept frames).

use std::path::Path;

use super::store::ImageBlock;

/// Sent images are downscaled to at most this many pixels wide.
pub const SEND_MAX_WIDTH: u32 = 1280;

/// JPEG quality of the copy sent to claude.
const SEND_QUALITY: f64 = 0.8;

/// Size a frame of `w` x `h` is sent at: at most `SEND_MAX_WIDTH` wide,
/// aspect kept, never upscaled.
pub fn send_size(w: usize, h: usize) -> (usize, usize) {
    let max = SEND_MAX_WIDTH as usize;
    if w <= max || w == 0 {
        return (w, h);
    }
    let th = ((h as f64) * (max as f64) / (w as f64)).round() as usize;
    (max, th.max(1))
}

/// Reads the full-size frame at `path`, downscales it to at most
/// `SEND_MAX_WIDTH` wide (aspect kept, never upscaled) and encodes it for
/// sending (JPEG quality 0.8, base64).
pub fn for_send(path: &Path) -> Result<ImageBlock, String> {
    #[cfg(target_os = "macos")]
    {
        use base64::Engine;
        let img = mac::decode_file(path)?;
        let (w, h) = (img.width(), img.height());
        let (tw, th) = send_size(w, h);
        let jpeg = if (tw, th) == (w, h) {
            mac::jpeg_bytes(&img, SEND_QUALITY)?
        } else {
            let mut bmp = mac::Bitmap::new(tw, th)?;
            bmp.draw(&img);
            let small = bmp.to_image().ok_or("could not build the downscaled image")?;
            mac::jpeg_bytes(&small, SEND_QUALITY)?
        };
        Ok(ImageBlock {
            media_type: "image/jpeg".to_string(),
            base64: base64::engine::general_purpose::STANDARD.encode(jpeg),
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(format!(
            "frame encoding is only available on macOS: {}",
            path.display()
        ))
    }
}

#[cfg(target_os = "macos")]
pub(super) mod mac {
    use std::ffi::c_void;
    use std::path::Path;

    use cidre::{arc, cf, cg};

    /// An RGBX bitmap (bytes R, G, B, unused) the CoreGraphics calls draw
    /// into; `rgba()` gives the pixels, `to_image()` a CGImage of them.
    pub struct Bitmap {
        ctx: *mut c_void,
        buf: Vec<u8>,
        pub width: usize,
        pub height: usize,
    }

    impl Bitmap {
        pub fn new(width: usize, height: usize) -> Result<Self, String> {
            if width == 0 || height == 0 {
                return Err("empty image".to_string());
            }
            let space = cg::ColorSpace::with_name(cg::color_space::names::srgb())
                .ok_or("no sRGB color space")?;
            let mut buf = vec![0u8; width * height * 4];
            // kCGImageAlphaNoneSkipLast (5), default byte order: bytes R,G,B,X.
            let ctx = unsafe {
                CGBitmapContextCreate(buf.as_mut_ptr().cast(), width, height, 8, width * 4, &space, 5)
            };
            if ctx.is_null() {
                return Err("could not create a bitmap context".to_string());
            }
            Ok(Self {
                ctx,
                buf,
                width,
                height,
            })
        }

        /// Draws `img` scaled to fill the bitmap.
        pub fn draw(&mut self, img: &cg::Image) {
            unsafe {
                CGContextSetInterpolationQuality(self.ctx, 3); // high
                CGContextDrawImage(
                    self.ctx,
                    cg::Rect::with_size(self.width as cg::Float, self.height as cg::Float),
                    img,
                );
            }
        }

        /// The pixel bytes, `width * 4` per row, no padding.
        pub fn rgba(&self) -> &[u8] {
            &self.buf
        }

        pub fn to_image(&self) -> Option<arc::R<cg::Image>> {
            unsafe { CGBitmapContextCreateImage(self.ctx) }
        }

        /// Test helper: sets one pixel.
        #[cfg(test)]
        pub fn set_rgb(&mut self, x: usize, y: usize, rgb: [u8; 3]) {
            let i = (y * self.width + x) * 4;
            self.buf[i..i + 3].copy_from_slice(&rgb);
        }
    }

    impl Drop for Bitmap {
        fn drop(&mut self) {
            unsafe { CGContextRelease(self.ctx) }
        }
    }

    /// Decodes the image file at `path`.
    pub fn decode_file(path: &Path) -> Result<arc::R<cg::Image>, String> {
        let url = cf::Url::with_file_path(path).ok_or("bad path")?;
        let src = cg::ImageSrc::with_url(&url, None)
            .ok_or_else(|| format!("cannot read an image at {}", path.display()))?;
        src.image_at(0, None)
            .ok_or_else(|| format!("cannot decode the image at {}", path.display()))
    }

    /// Encodes `img` as JPEG at `quality` (0.0 to 1.0).
    pub fn jpeg_bytes(img: &cg::Image, quality: f64) -> Result<Vec<u8>, String> {
        use cg::image::destination::{AddOptKey, OptKey};
        let mut data = cf::DataMut::with_capacity(0);
        let ty = cf::String::from_str("public.jpeg");
        let mut dst = cg::ImageDst::with_data(&mut data, &ty, 1).ok_or("no JPEG encoder")?;
        let q = cf::Number::from_f64(quality);
        // Both key types are newtypes of the same CFString; the quality key is
        // the one ImageIO wants in the per-image options.
        let key: &AddOptKey =
            unsafe { &*(OptKey::lossy_compression_quality() as *const OptKey).cast::<AddOptKey>() };
        let qt: &cf::Type = &q;
        let opts = cf::DictionaryOf::with_keys_values(&[key], &[qt]);
        dst.add_image(img, Some(&opts));
        if !dst.finalize() {
            return Err("JPEG encoding failed".to_string());
        }
        Ok(data.as_slice().to_vec())
    }

    /// Width and height of an encoded image, or None when it does not decode.
    #[cfg(test)]
    pub fn dimensions(bytes: &[u8]) -> Option<(usize, usize)> {
        let data = cf::Data::from_slice(bytes)?;
        let src = cg::ImageSrc::with_data(&data, None)?;
        let img = src.image_at(0, None)?;
        Some((img.width(), img.height()))
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: &cg::ColorSpace,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGBitmapContextCreateImage(ctx: *mut c_void) -> Option<arc::R<cg::Image>>;
        fn CGContextDrawImage(ctx: *mut c_void, rect: cg::Rect, image: &cg::Image);
        fn CGContextSetInterpolationQuality(ctx: *mut c_void, quality: i32);
        fn CGContextRelease(ctx: *mut c_void);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use base64::Engine;

    /// Writes a w x h JPEG with a gradient so it is not a flat color.
    fn write_jpeg(dir: &Path, name: &str, w: usize, h: usize) -> std::path::PathBuf {
        let mut bmp = mac::Bitmap::new(w, h).unwrap();
        for y in (0..h).step_by(7) {
            for x in 0..w {
                bmp.set_rgb(x, y, [(x * 255 / w) as u8, (y * 255 / h) as u8, 90]);
            }
        }
        let img = bmp.to_image().unwrap();
        let path = dir.join(name);
        std::fs::write(&path, mac::jpeg_bytes(&img, 0.9).unwrap()).unwrap();
        path
    }

    fn decoded_dims(block: &ImageBlock) -> (usize, usize) {
        assert_eq!(block.media_type, "image/jpeg");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&block.base64)
            .unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "JPEG magic");
        mac::dimensions(&bytes).unwrap()
    }

    #[test]
    fn retina_frame_is_downscaled_to_1280_wide() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_jpeg(dir.path(), "big.jpg", 2880, 1800);
        assert_eq!(decoded_dims(&for_send(&p).unwrap()), (1280, 800));
    }

    #[test]
    fn small_frame_is_never_upscaled() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_jpeg(dir.path(), "small.jpg", 1000, 600);
        assert_eq!(decoded_dims(&for_send(&p).unwrap()), (1000, 600));
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        assert!(for_send(Path::new("/nonexistent/frame.jpg")).is_err());
    }

    #[test]
    fn send_size_keeps_aspect() {
        assert_eq!(send_size(2880, 1800), (1280, 800));
        assert_eq!(send_size(1280, 720), (1280, 720));
        assert_eq!(send_size(1000, 600), (1000, 600));
        assert_eq!(send_size(3000, 1), (1280, 1));
    }
}
