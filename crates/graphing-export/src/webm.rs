//! WebM video of a diagram's animation: AV1 from rav1e (pure Rust), in a
//! small Matroska writer of our own (one video track, simple blocks).

use rav1e::prelude::*;

use crate::ExportError;

/// RGBA frames (each `w * h * 4`, shown `ms` milliseconds) as a WebM.
/// `progress` is called once per frame encoded.
pub fn encode(w: u32, h: u32, frames: &[(Vec<u8>, u32)], fps: f64, progress: &dyn Fn()) -> Result<Vec<u8>, ExportError> {
    let err = |e: &dyn std::fmt::Display| ExportError::Png(format!("video: {e}"));
    // 4:2:0 needs even sides.
    let (ew, eh) = ((w + 1) & !1, (h + 1) & !1);
    let step_ms = 1000.0 / fps;
    let enc = EncoderConfig {
        width: ew as usize,
        height: eh as usize,
        time_base: Rational::new(1, fps.round() as u64),
        bit_depth: 8,
        chroma_sampling: ChromaSampling::Cs420,
        pixel_range: PixelRange::Limited,
        color_description: Some(color::ColorDescription {
            color_primaries: color::ColorPrimaries::BT709,
            transfer_characteristics: color::TransferCharacteristics::BT709,
            matrix_coefficients: color::MatrixCoefficients::BT709,
        }),
        low_latency: true,
        quantizer: 70,
        max_key_frame_interval: (fps * 4.0).round() as u64,
        speed_settings: SpeedSettings::from_preset(10),
        // Tiles let the encoder use every core.
        tiles: 8,
        ..Default::default()
    };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let cfg = Config::new().with_encoder_config(enc).with_threads(threads);
    let mut ctx: Context<u8> = cfg.new_context().map_err(|e| err(&e))?;
    let mut packets: Vec<(Vec<u8>, bool)> = Vec::new();
    let drain = |ctx: &mut Context<u8>, packets: &mut Vec<(Vec<u8>, bool)>| -> Result<bool, ExportError> {
        loop {
            match ctx.receive_packet() {
                Ok(p) => packets.push((strip_delimiter(p.data), p.frame_type == FrameType::KEY)),
                Err(EncoderStatus::Encoded) => continue,
                Err(EncoderStatus::NeedMoreData) => return Ok(false),
                Err(EncoderStatus::LimitReached) => return Ok(true),
                Err(e) => return Err(err(&e)),
            }
        }
    };
    // Constant frame rate: a held frame repeats (and costs almost nothing).
    for (rgba, ms) in frames {
        progress();
        let repeats = ((*ms as f64) / step_ms).round().max(1.0) as usize;
        let (y, u, v) = to_yuv420(rgba, w, h, ew, eh);
        for _ in 0..repeats {
            let mut f = ctx.new_frame();
            f.planes[0].copy_from_raw_u8(&y, ew as usize, 1);
            f.planes[1].copy_from_raw_u8(&u, (ew / 2) as usize, 1);
            f.planes[2].copy_from_raw_u8(&v, (ew / 2) as usize, 1);
            ctx.send_frame(f).map_err(|e| err(&e))?;
            drain(&mut ctx, &mut packets)?;
        }
    }
    ctx.flush();
    while !drain(&mut ctx, &mut packets)? {}
    let header = ctx.container_sequence_header();
    Ok(mux(ew, eh, &header, &packets, step_ms))
}

/// Matroska stores AV1 without temporal delimiter OBUs.
fn strip_delimiter(data: Vec<u8>) -> Vec<u8> {
    // obu_type 2 with a size field: header byte, then a leb128 size of 0.
    if data.len() >= 2 && (data[0] >> 3) & 0x0f == 2 && data[0] & 0x02 != 0 && data[1] == 0 {
        return data[2..].to_vec();
    }
    data
}

/// BT.709 limited range, chroma averaged over 2x2; edges repeat to fill
/// the even-sized picture.
fn to_yuv420(rgba: &[u8], w: u32, h: u32, ew: u32, eh: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let px = |x: u32, y: u32| {
        let i = ((y.min(h - 1) * w + x.min(w - 1)) * 4) as usize;
        let a = rgba[i + 3] as f32 / 255.0;
        // Transparent areas over white.
        let c = |v: u8| v as f32 * a + 255.0 * (1.0 - a);
        (c(rgba[i]), c(rgba[i + 1]), c(rgba[i + 2]))
    };
    let mut y = vec![0u8; (ew * eh) as usize];
    for row in 0..eh {
        for col in 0..ew {
            let (r, g, b) = px(col, row);
            y[(row * ew + col) as usize] = (16.0 + (0.2126 * r + 0.7152 * g + 0.0722 * b) * 219.0 / 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    let (cw, ch) = (ew / 2, eh / 2);
    let (mut u, mut v) = (vec![0u8; (cw * ch) as usize], vec![0u8; (cw * ch) as usize]);
    for row in 0..ch {
        for col in 0..cw {
            let mut acc = (0.0, 0.0, 0.0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (r, g, b) = px(col * 2 + dx, row * 2 + dy);
                acc = (acc.0 + r / 4.0, acc.1 + g / 4.0, acc.2 + b / 4.0);
            }
            let (r, g, b) = acc;
            let i = (row * cw + col) as usize;
            u[i] = (128.0 + (-0.1146 * r - 0.3854 * g + 0.5 * b) * 224.0 / 255.0).round().clamp(0.0, 255.0) as u8;
            v[i] = (128.0 + (0.5 * r - 0.4542 * g - 0.0458 * b) * 224.0 / 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    (y, u, v)
}

/// An EBML element: id, an 8-byte size, the body.
fn el(id: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 12);
    let id_bytes = id.to_be_bytes();
    let skip = id_bytes.iter().position(|&b| b != 0).unwrap_or(3);
    out.extend_from_slice(&id_bytes[skip..]);
    out.push(0x01);
    out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

fn uint(id: u32, v: u64) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let skip = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    el(id, &bytes[skip..])
}

fn text(id: u32, s: &str) -> Vec<u8> {
    el(id, s.as_bytes())
}

fn float(id: u32, v: f64) -> Vec<u8> {
    el(id, &v.to_be_bytes())
}

/// One video track of AV1 `packets` (data, keyframe), one per `step_ms`.
fn mux(w: u32, h: u32, codec_private: &[u8], packets: &[(Vec<u8>, bool)], step_ms: f64) -> Vec<u8> {
    let header = [
        uint(0x4286, 1),
        uint(0x42F7, 1),
        uint(0x42F2, 4),
        uint(0x42F3, 8),
        text(0x4282, "webm"),
        uint(0x4287, 4),
        uint(0x4285, 2),
    ]
    .concat();
    let duration = packets.len() as f64 * step_ms;
    let info = el(0x1549A966, &[uint(0x2AD7B1, 1_000_000), float(0x4489, duration), text(0x4D80, "graphing"), text(0x5741, "graphing")].concat());
    let video = el(0xE0, &[uint(0xB0, w as u64), uint(0xBA, h as u64)].concat());
    let track = el(
        0xAE,
        &[uint(0xD7, 1), uint(0x73C5, 1), uint(0x83, 1), uint(0x9C, 0), text(0x86, "V_AV1"), el(0x63A2, codec_private), uint(0x23E383, (step_ms * 1e6).round() as u64), video].concat(),
    );
    let tracks = el(0x1654AE6B, &track);
    // A cluster per keyframe, so players can seek and loop.
    let mut clusters = Vec::new();
    let mut current: Option<(u64, Vec<u8>)> = None;
    for (i, (data, key)) in packets.iter().enumerate() {
        let ts = (i as f64 * step_ms).round() as u64;
        if *key || current.as_ref().is_none_or(|(start, _)| ts - start > 30_000) {
            if let Some((start, body)) = current.take() {
                clusters.extend(el(0x1F43B675, &[uint(0xE7, start), body].concat()));
            }
            current = Some((ts, Vec::new()));
        }
        let (start, body) = current.as_mut().expect("cluster open");
        let mut block = vec![0x81];
        block.extend_from_slice(&((ts - *start) as i16).to_be_bytes());
        block.push(if *key { 0x80 } else { 0x00 });
        block.extend_from_slice(data);
        body.extend(el(0xA3, &block));
    }
    if let Some((start, body)) = current {
        clusters.extend(el(0x1F43B675, &[uint(0xE7, start), body].concat()));
    }
    let segment = el(0x18538067, &[info, tracks, clusters].concat());
    [el(0x1A45DFA3, &header), segment].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_encode_their_ids_and_sizes() {
        assert_eq!(uint(0x4286, 1), [0x42, 0x86, 0x01, 0, 0, 0, 0, 0, 0, 1, 1]);
        assert_eq!(&el(0x1A45DFA3, &[])[..5], &[0x1A, 0x45, 0xDF, 0xA3, 0x01]);
        assert_eq!(strip_delimiter(vec![0x12, 0x00, 0x0a, 0x0b]), [0x0a, 0x0b]);
    }

    #[test]
    fn white_is_white_in_yuv() {
        let (y, u, v) = to_yuv420(&[255; 16], 2, 2, 2, 2);
        assert_eq!((y[0], u[0], v[0]), (235, 128, 128));
    }

    #[test]
    fn a_tiny_video_encodes() {
        let frame = |c: u8| (vec![c; 31 * 16 * 4], 100);
        let webm = encode(31, 16, &[frame(0), frame(255), frame(128)], 10.0, &|| {}).unwrap();
        assert!(webm.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]));
        assert!(webm.windows(5).any(|w| w == b"V_AV1"));
    }
}
