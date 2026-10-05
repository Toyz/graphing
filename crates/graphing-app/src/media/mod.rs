//! Pictures: telling formats apart, decoding every frame of animated GIF,
//! WebP and AVIF, and choosing which frame shows when. The limits match
//! notesy's so a picture behaves the same in both apps: past them an
//! animation shows its first frame only.

mod avif;

use std::io::Cursor;
use std::sync::Arc;

use gpui_kit::{RenderImage, SvgRenderer};
use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, Delay, DynamicImage, Frame, RgbaImage};
use smallvec::SmallVec;

/// Longest side kept after decoding.
const MAX_SIDE: u32 = 1600;
/// Frames an animation may keep.
const MAX_FRAMES: usize = 300;
/// Pixels all of an animation's frames together may take (~64 MB).
const MAX_ANIMATION_PIXELS: usize = 16 * 1024 * 1024;
/// An animation's canvas, decoded whole per frame, may take this much.
const MAX_CANVAS_BYTES: u64 = 64 * 1024 * 1024;
/// Repaints for moving pictures: 20 a second at most, as in notesy.
pub const FRAME_GAP_MS: u64 = 50;

/// Each frame and how long it shows, in seconds (a still's one frame, 0).
type Frames = Vec<(RgbaImage, f64)>;

/// What a picture's bytes are, by their first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Avif,
    Bmp,
    Svg,
}

pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    Some(if bytes.starts_with(b"\x89PNG") {
        Kind::Png
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Kind::Jpeg
    } else if bytes.starts_with(b"GIF8") {
        Kind::Gif
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Kind::Webp
    } else if avif::is_avif(&bytes[..bytes.len().min(64)]) {
        Kind::Avif
    } else if bytes.starts_with(b"BM") {
        Kind::Bmp
    } else if bytes[..bytes.len().min(512)].windows(4).any(|w| w == b"<svg") {
        Kind::Svg
    } else {
        return None;
    })
}

/// Shrunk so the longest side is at most [`MAX_SIDE`].
fn fit(img: DynamicImage) -> RgbaImage {
    let img = if img.width() > MAX_SIDE || img.height() > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
    img.into_rgba8()
}

/// Every frame of an animation from `image`'s decoders, held to the limits.
fn animation<'a>(decoder: impl AnimationDecoder<'a>) -> Option<Frames> {
    let mut out = Frames::new();
    let mut pixels = 0usize;
    for frame in decoder.into_frames() {
        let frame = frame.ok()?;
        let (n, d) = frame.delay().numer_denom_ms();
        let secs = if d == 0 { 0.1 } else { f64::from(n) / f64::from(d) / 1000.0 };
        let buffer = frame.into_buffer();
        if u64::from(buffer.width()) * u64::from(buffer.height()) * 4 > MAX_CANVAS_BYTES {
            out.truncate(1);
            break;
        }
        let image = fit(DynamicImage::ImageRgba8(buffer));
        pixels += (image.width() * image.height()) as usize;
        out.push((image, if secs > 0.0 { secs } else { 0.1 }));
        if out.len() > MAX_FRAMES || pixels > MAX_ANIMATION_PIXELS {
            out.truncate(1);
            break;
        }
    }
    if let [only] = out.as_mut_slice() {
        only.1 = 0.0;
    }
    (!out.is_empty()).then_some(out)
}

/// Frames as gpui wants them: BGRA, each with its delay.
fn render_image(frames: Frames) -> Arc<RenderImage> {
    let frames: SmallVec<[Frame; 1]> = frames
        .into_iter()
        .map(|(mut img, secs)| {
            for px in img.pixels_mut() {
                px.0.swap(0, 2);
            }
            Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms((secs * 1000.0).round() as u32, 1))
        })
        .collect();
    Arc::new(RenderImage::new(frames))
}

/// Decode any supported picture, every frame of an animated one.
pub fn decode(bytes: &[u8], svg: SvgRenderer) -> Option<Arc<RenderImage>> {
    let frames = match sniff(bytes)? {
        Kind::Avif => avif::decode(bytes).ok()?,
        Kind::Gif => animation(GifDecoder::new(Cursor::new(bytes)).ok()?)?,
        Kind::Webp => {
            let decoder = WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if decoder.has_animation() {
                animation(decoder)?
            } else {
                vec![(fit(image::load_from_memory_with_format(bytes, image::ImageFormat::WebP).ok()?), 0.0)]
            }
        }
        Kind::Svg => return gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Svg, bytes.to_vec()).to_image_data(svg).ok(),
        Kind::Png | Kind::Jpeg | Kind::Bmp => vec![(fit(image::load_from_memory(bytes).ok()?), 0.0)],
    };
    Some(render_image(frames))
}

/// Whether a decoded picture moves.
pub fn animated(data: &RenderImage) -> bool {
    data.frame_count() > 1
}

/// The frame showing `ms` into a looping animation.
pub fn frame_at(data: &RenderImage, ms: u64) -> usize {
    let n = data.frame_count();
    if n <= 1 {
        return 0;
    }
    let delays: Vec<u64> = (0..n)
        .map(|i| {
            let (num, den) = data.delay(i).numer_denom_ms();
            if den == 0 { 100 } else { (u64::from(num) / u64::from(den)).max(20) }
        })
        .collect();
    let total: u64 = delays.iter().sum();
    let mut t = ms % total.max(1);
    for (i, d) in delays.iter().enumerate() {
        if t < *d {
            return i;
        }
        t -= d;
    }
    n - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gif(frames: u8) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            enc.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
            for i in 0..frames {
                let img = RgbaImage::from_pixel(8, 8, image::Rgba([i * 40, 0, 0, 255]));
                enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(100, 1))).unwrap();
            }
        }
        out
    }

    #[test]
    fn kinds_by_first_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n"), Some(Kind::Png));
        assert_eq!(sniff(&gif(1)), Some(Kind::Gif));
        assert_eq!(sniff(include_bytes!("testdata/still_alpha.avif")), Some(Kind::Avif));
        assert_eq!(sniff(b"<?xml version='1.0'?><svg/>"), Some(Kind::Svg));
        assert_eq!(sniff(b"hello"), None);
    }

    #[test]
    fn frames_follow_their_delays() {
        let frames = animation(GifDecoder::new(Cursor::new(gif(3))).unwrap()).unwrap();
        assert_eq!(frames.len(), 3);
        let data = render_image(frames);
        assert!(animated(&data));
        assert_eq!([0, 99, 100, 250, 300, 301].map(|ms| frame_at(&data, ms)), [0, 0, 1, 2, 0, 0]);
    }

    #[test]
    fn a_one_frame_gif_is_still() {
        let frames = animation(GifDecoder::new(Cursor::new(gif(1))).unwrap()).unwrap();
        assert_eq!((frames.len(), frames[0].1), (1, 0.0));
    }

    #[test]
    fn avif_animations_decode() {
        let data = render_image(avif::decode(include_bytes!("testdata/anim_alpha.avif")).unwrap());
        assert_eq!(data.frame_count(), 5);
    }
}
