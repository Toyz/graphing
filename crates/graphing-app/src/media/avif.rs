//! AVIF images, still and animated (an AVIF image sequence), alpha and all:
//! the file read by mp4parse, its AV1 decoded by rav1d a frame at a time,
//! and each frame turned from YUV into RGBA by yuv before the next is
//! decoded. What comes out is held to the limits GIF and WebP are (see
//! [`super`]): an animation too long or too large shows its first frame.
//!
//! Ported from notesy (`notesy-ui/src/images/avif.rs`), so both apps read
//! the same files the same way.

use std::collections::VecDeque;
use std::io::Cursor;
use std::ops::Range;

use image::{DynamicImage, RgbaImage};
use mp4parse::unstable::{create_sample_table, CheckedInteger, Indice};
use mp4parse::{read_avif, MediaContext, ParseStrictness, TrackType};
use re_rav1d::pixel::{MatrixCoefficients, YUVRange};
use re_rav1d::{Decoder, Picture, PixelLayout, PlanarImageComponent as C, Settings};
use yuv::{YuvPlanarImage, YuvRange, YuvStandardMatrix};

use super::{Frames, MAX_ANIMATION_PIXELS, MAX_CANVAS_BYTES, MAX_FRAMES, fit};

/// The most pixels a frame may have: 8192 × 8192.
const MAX_FRAME_PIXELS: u32 = 8192 * 8192;
const BAD: &str = "Couldn't read image";

/// Whether `head`, a file's first bytes, is an AVIF's: its `ftyp` box names
/// `avif` (a still) or `avis` (an animation), as its brand or among those
/// it's compatible with.
pub(super) fn is_avif(head: &[u8]) -> bool {
    if head.get(4..8) != Some(b"ftyp") {
        return false;
    }
    let size = head.get(0..4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let end = size.clamp(16, head.len().max(16));
    // The brand, a version, then the brands it's compatible with.
    let compatible = head.get(16..end).unwrap_or_default().as_chunks::<4>().0.iter().map(|b| b.as_slice());
    head.get(8..12).into_iter().chain(compatible).any(|b| b == b"avif" || b == b"avis")
}

/// Every frame of the AVIF `bytes`, with how long each shows (a still's
/// one frame, 0).
pub(super) fn decode(bytes: &[u8]) -> Result<Frames, &'static str> {
    let file = read_avif(&mut Cursor::new(bytes), ParseStrictness::Normal).map_err(|_| BAD)?;
    let premultiplied = file.premultiplied_alpha;
    if let Some(sequence) = &file.sequence
        && let Some(frames) = animation(bytes, sequence, premultiplied)?
    {
        return Ok(frames);
    }
    // A still: its image, and the one of its alpha beside it. (One made of
    // tiles, a grid of images, isn't read.)
    let color = file.primary_item_coded_data().ok_or("Unsupported image format")?;
    let picture = decode_one(color)?;
    let alpha = file.alpha_item_coded_data().map(decode_one).transpose()?;
    let image = rgba(&picture, alpha.as_ref(), premultiplied)?;
    Ok(vec![(fit(DynamicImage::ImageRgba8(image)), 0.0)])
}

/// An AVIF sequence's frames, with its alpha track's beside them where it
/// has one. `None` when it has no track of pictures to play.
fn animation(bytes: &[u8], sequence: &MediaContext, premultiplied: bool) -> Result<Option<Frames>, &'static str> {
    let Some(color) = sequence.tracks.iter().find(|t| matches!(t.track_type, TrackType::Picture | TrackType::Video)) else { return Ok(None) };
    let alpha = sequence
        .tracks
        .iter()
        .find(|t| t.track_type == TrackType::AuxiliaryVideo && t.tref.as_ref().zip(color.track_id).is_some_and(|(r, id)| r.has_auxl_reference(id)));
    let timescale = color.timescale.map(|t| t.0).filter(|&t| t > 0).unwrap_or(1000) as f64;
    let samples = create_sample_table(color, CheckedInteger(0)).ok_or(BAD)?;
    let alpha_samples = alpha.map(|t| create_sample_table(t, CheckedInteger(0)).ok_or(BAD)).transpose()?;
    // Longer than an animation may be: its first frame only.
    let too_long = samples.len() > MAX_FRAMES;
    let durations: Vec<f64> = samples.iter().map(|s| (s.end_composition.0 - s.start_composition.0).max(0) as f64 / timescale).collect();
    let (mut colors, mut alphas) = (Stream::new()?, alpha_samples.as_ref().map(|_| Stream::new()).transpose()?);
    let (mut out, mut pixels): (Frames, usize) = (Vec::new(), 0);
    for (i, sample) in samples.iter().enumerate() {
        colors.feed(bytes.get(span(sample)).ok_or(BAD)?, i)?;
        if let (Some(alphas), Some(s)) = (&mut alphas, alpha_samples.as_ref().and_then(|a| a.get(i))) {
            alphas.feed(bytes.get(span(s)).ok_or(BAD)?, i)?;
        }
        // Each frame as it comes out, turned into RGBA before the next is decoded.
        while let Some(picture) = colors.ready.pop_front() {
            let alpha = alphas.as_mut().and_then(|a| a.ready.pop_front());
            let secs = picture.timestamp().and_then(|t| durations.get(usize::try_from(t).ok()?)).copied().unwrap_or(0.1);
            let canvas = u64::from(picture.width()) * u64::from(picture.height()) * 4 > MAX_CANVAS_BYTES;
            let image = fit(DynamicImage::ImageRgba8(rgba(&picture, alpha.as_ref(), premultiplied)?));
            pixels += (image.width() * image.height()) as usize;
            out.push((image, if secs > 0.0 { secs } else { 0.1 }));
            // Too long, or too large to keep every frame of: the first is the image.
            if too_long || canvas || pixels > MAX_ANIMATION_PIXELS {
                out.truncate(1);
                out[0].1 = 0.0;
                return Ok(Some(out));
            }
        }
    }
    if let [only] = out.as_mut_slice() {
        only.1 = 0.0;
    }
    Ok((!out.is_empty()).then_some(out))
}

/// Where a sample is in the file.
fn span(sample: &Indice) -> Range<usize> {
    sample.start_offset.0 as usize..sample.end_offset.0 as usize
}

/// A track's AV1 decoder, and the pictures it's put out, in order.
struct Stream {
    decoder: Decoder,
    ready: VecDeque<Picture>,
}

impl Stream {
    fn new() -> Result<Self, &'static str> {
        let mut settings = Settings::new();
        // A worker thread already: one here, and a picture out for each frame in.
        settings.set_n_threads(1);
        settings.set_max_frame_delay(1);
        settings.set_frame_size_limit(MAX_FRAME_PIXELS);
        Ok(Self { decoder: Decoder::with_settings(&settings).map_err(|_| BAD)?, ready: VecDeque::new() })
    }

    /// One sample's AV1 in; what pictures that makes are `ready`.
    fn feed(&mut self, data: &[u8], index: usize) -> Result<(), &'static str> {
        let mut sent = self.decoder.send_data(data.to_vec(), None, Some(index as i64), None);
        loop {
            match sent {
                Ok(()) => break,
                // Its pictures to be taken first, then the rest of it.
                Err(e) if e.is_again() => {
                    self.take()?;
                    sent = self.decoder.send_pending_data();
                }
                Err(_) => return Err(BAD),
            }
        }
        self.take()
    }

    fn take(&mut self) -> Result<(), &'static str> {
        loop {
            match self.decoder.get_picture() {
                Ok(picture) => self.ready.push_back(picture),
                Err(e) if e.is_again() => return Ok(()),
                Err(_) => return Err(BAD),
            }
        }
    }
}

/// The one picture a still's AV1 makes.
fn decode_one(data: &[u8]) -> Result<Picture, &'static str> {
    let mut stream = Stream::new()?;
    stream.feed(data, 0)?;
    stream.ready.pop_front().ok_or(BAD)
}

fn matrix(picture: &Picture) -> YuvStandardMatrix {
    match picture.matrix_coefficients() {
        MatrixCoefficients::BT709 => YuvStandardMatrix::Bt709,
        MatrixCoefficients::BT2020NonConstantLuminance | MatrixCoefficients::BT2020ConstantLuminance => YuvStandardMatrix::Bt2020,
        MatrixCoefficients::ST240M => YuvStandardMatrix::Smpte240,
        MatrixCoefficients::BT470M => YuvStandardMatrix::Fcc,
        // BT.601, and unspecified, which libavif takes as BT.601.
        _ => YuvStandardMatrix::Bt601,
    }
}

/// A plane of more than 8 bits a sample, as its samples, and its stride in them.
fn plane_u16(picture: &Picture, c: C) -> (Vec<u16>, u32) {
    let bytes: &[u8] = &picture.plane(c);
    (bytes.as_chunks::<2>().0.iter().map(|b| u16::from_ne_bytes(*b)).collect(), picture.stride(c) / 2)
}

/// Sample `v` of `bits` bits, from limited range (16 to 235 at 8 bits) or
/// full, as 0 to 255.
fn to_8_bits(v: u32, bits: u32, limited: bool) -> u8 {
    let max = (1u32 << bits) - 1;
    let v = if limited {
        let (low, span) = (16u32 << (bits - 8), 219u32 << (bits - 8));
        (v.saturating_sub(low) * max / span).min(max)
    } else {
        v.min(max)
    };
    ((v * 255 + max / 2) / max) as u8
}

/// `picture`, with `alpha`'s luma as its alpha where it has one, as RGBA.
fn rgba(picture: &Picture, alpha: Option<&Picture>, premultiplied: bool) -> Result<RgbaImage, &'static str> {
    let (w, h) = (picture.width(), picture.height());
    let mut out = vec![255u8; w as usize * h as usize * 4];
    let range = match picture.color_range() {
        YUVRange::Limited => YuvRange::Limited,
        YUVRange::Full => YuvRange::Full,
    };
    let bits = picture.bits_per_component().map_or(8, |b| b.0 as u32);
    let layout = picture.pixel_layout();
    // RGB kept as it is (G, B, R in the Y, U, V planes), as ffmpeg writes an RGB source.
    let identity = matches!(picture.matrix_coefficients(), MatrixCoefficients::Identity);
    let stride = w * 4;
    if layout == PixelLayout::I400 || identity {
        let sample = |c: C, x: usize, y: usize| -> u32 {
            let s = picture.stride(c) as usize;
            let plane = picture.plane(c);
            if picture.bit_depth() == 8 {
                u32::from(plane[y * s + x])
            } else {
                u32::from(u16::from_ne_bytes([plane[y * s + x * 2], plane[y * s + x * 2 + 1]]))
            }
        };
        let limited = range == YuvRange::Limited && !identity;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let o = (y * w as usize + x) * 4;
                let g = to_8_bits(sample(C::Y, x, y), bits, limited);
                if identity {
                    out[o] = to_8_bits(sample(C::V, x, y), bits, limited);
                    out[o + 1] = g;
                    out[o + 2] = to_8_bits(sample(C::U, x, y), bits, limited);
                } else {
                    out[o..o + 3].fill(g);
                }
            }
        }
    } else if picture.bit_depth() == 8 {
        let (y, u, v) = (picture.plane(C::Y), picture.plane(C::U), picture.plane(C::V));
        let image = YuvPlanarImage {
            y_plane: &y,
            y_stride: picture.stride(C::Y),
            u_plane: &u,
            u_stride: picture.stride(C::U),
            v_plane: &v,
            v_stride: picture.stride(C::V),
            width: w,
            height: h,
        };
        // Color between chroma samples blended, not repeated: sharp edges stay clean.
        match layout {
            PixelLayout::I420 => yuv::yuv420_to_rgba_bilinear(&image, &mut out, stride, range, matrix(picture)),
            PixelLayout::I422 => yuv::yuv422_to_rgba_bilinear(&image, &mut out, stride, range, matrix(picture)),
            _ => yuv::yuv444_to_rgba(&image, &mut out, stride, range, matrix(picture)),
        }
        .map_err(|_| BAD)?;
    } else {
        let ((y, ys), (u, us), (v, vs)) = (plane_u16(picture, C::Y), plane_u16(picture, C::U), plane_u16(picture, C::V));
        let image = YuvPlanarImage { y_plane: &y, y_stride: ys, u_plane: &u, u_stride: us, v_plane: &v, v_stride: vs, width: w, height: h };
        let m = matrix(picture);
        match (layout, bits) {
            (PixelLayout::I420, 10) => yuv::i010_to_rgba(&image, &mut out, stride, range, m),
            (PixelLayout::I422, 10) => yuv::i210_to_rgba(&image, &mut out, stride, range, m),
            (_, 10) => yuv::i410_to_rgba(&image, &mut out, stride, range, m),
            (PixelLayout::I420, _) => yuv::i012_to_rgba(&image, &mut out, stride, range, m),
            (PixelLayout::I422, _) => yuv::i212_to_rgba(&image, &mut out, stride, range, m),
            _ => yuv::i412_to_rgba(&image, &mut out, stride, range, m),
        }
        .map_err(|_| BAD)?;
    }
    if let Some(alpha) = alpha {
        if (alpha.width(), alpha.height()) != (w, h) {
            return Err(BAD);
        }
        let bits = alpha.bits_per_component().map_or(8, |b| b.0 as u32);
        let limited = matches!(alpha.color_range(), YUVRange::Limited);
        let (plane, s) = (alpha.plane(C::Y), alpha.stride(C::Y) as usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let v = if alpha.bit_depth() == 8 { u32::from(plane[y * s + x]) } else { u32::from(u16::from_ne_bytes([plane[y * s + x * 2], plane[y * s + x * 2 + 1]])) };
                out[(y * w as usize + x) * 4 + 3] = to_8_bits(v, bits, limited);
            }
        }
        // Colors kept times their alpha: taken back out, as the renderer wants them.
        if premultiplied {
            for px in out.as_chunks_mut::<4>().0 {
                let a = u32::from(px[3]);
                if a != 0 && a != 255 {
                    for c in &mut px[..3] {
                        *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
        }
    }
    RgbaImage::from_raw(w, h, out).ok_or(BAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANIMATED_ALPHA: &[u8] = include_bytes!("testdata/anim_alpha.avif");
    const ANIMATED_FFMPEG: &[u8] = include_bytes!("testdata/anim_ffmpeg.avif");
    const STILL_ALPHA: &[u8] = include_bytes!("testdata/still_alpha.avif");
    const STILL_10_BIT: &[u8] = include_bytes!("testdata/still_10b_422_limited.avif");

    #[test]
    fn an_animated_avif_plays_each_frame_for_its_time_with_its_alpha() {
        let frames = decode(ANIMATED_ALPHA).unwrap();
        let secs: Vec<f64> = frames.iter().map(|(_, s)| *s).collect();
        assert_eq!(secs, [0.2, 0.4, 0.1, 0.3, 0.5]);
        assert!(frames.iter().all(|(image, _)| image.dimensions() == (64, 48)));
        assert!(frames[0].0.pixels().any(|p| p.0[3] < 255), "its alpha track read");
        assert_ne!(frames[0].0.as_raw(), frames[1].0.as_raw(), "each frame its own");
        // Made by ffmpeg: an RGB source kept as RGB (the identity matrix), no alpha.
        let frames = decode(ANIMATED_FFMPEG).unwrap();
        assert_eq!(frames.len(), 5);
        assert!(frames.iter().all(|(image, secs)| image.dimensions() == (64, 48) && (secs - 0.2).abs() < 1e-9));
    }

    #[test]
    fn a_still_avif_is_one_frame_at_any_depth() {
        let frames = decode(STILL_ALPHA).unwrap();
        assert_eq!((frames.len(), frames[0].1, frames[0].0.dimensions()), (1, 0.0, (64, 48)));
        assert!(frames[0].0.pixels().any(|p| p.0[3] < 255));
        let frames = decode(STILL_10_BIT).unwrap();
        assert_eq!((frames.len(), frames[0].0.dimensions()), (1, (64, 48)));
    }

    #[test]
    fn an_avif_is_known_by_its_first_bytes_and_a_broken_one_fails() {
        assert!(is_avif(ANIMATED_ALPHA) && is_avif(STILL_ALPHA) && is_avif(ANIMATED_FFMPEG));
        assert!(!is_avif(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
        assert!(!is_avif(b"ftyp"));
        // Cut short, or bytes changed: an error, never a panic.
        for cut in [10, 100, ANIMATED_ALPHA.len() / 2, ANIMATED_ALPHA.len() - 1] {
            let _ = decode(&ANIMATED_ALPHA[..cut]);
        }
        let mut flipped = ANIMATED_ALPHA.to_vec();
        for i in (40..flipped.len()).step_by(97) {
            flipped[i] ^= 0x5a;
        }
        let _ = decode(&flipped);
    }
}
