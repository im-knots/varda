//! HAP video codec decoder — demuxes with ffmpeg, parses HAP frames,
//! Snappy-decompresses BCn texture blocks for direct GPU upload.

use anyhow::{bail, Context, Result};
use std::path::Path;
extern crate ffmpeg_next as ffmpeg;
use super::{HapTextureFormat, LoopMode, PlaybackState};
use ffmpeg::format::input;
use ffmpeg::media::Type;

const COMPRESSOR_NONE: u8 = 0xA0;
const COMPRESSOR_SNAPPY: u8 = 0xB0;
const COMPRESSOR_COMPLEX: u8 = 0xC0;
const FMT_BC1: u8 = 0x0B;
const FMT_BC3: u8 = 0x0E;
const FMT_YCOCG: u8 = 0x0F;
const FMT_BC7: u8 = 0x0C;
const SECTION_MULTI_IMAGE: u8 = 0x0D;
const SECTION_DECODE_INSTRUCTIONS: u8 = 0x01;
const CHUNK_COMPRESSOR_TABLE: u8 = 0x02;
const CHUNK_SIZE_TABLE: u8 = 0x03;
const CHUNK_OFFSET_TABLE: u8 = 0x04;
const CHUNK_UNCOMPRESSED: u8 = 0x0A;
const CHUNK_SNAPPY_ID: u8 = 0x0B;

struct SectionHeader {
    section_type: u8,
    data_length: usize,
    header_size: usize,
}

fn parse_header(data: &[u8]) -> Result<SectionHeader> {
    if data.len() < 4 {
        bail!("HAP header too short");
    }
    let (data_length, header_size) = if data[0] == 0 && data[1] == 0 && data[2] == 0 {
        if data.len() < 8 {
            bail!("HAP 8-byte header incomplete");
        }
        (
            u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize,
            8,
        )
    } else {
        (
            data[0] as usize | ((data[1] as usize) << 8) | ((data[2] as usize) << 16),
            4,
        )
    };
    Ok(SectionHeader {
        section_type: data[3],
        data_length,
        header_size,
    })
}

/// BC4/RGTC1 nibble (alpha-only, used in Hap Q Alpha's alpha plane)
const FMT_BC4: u8 = 0x01;

fn tex_fmt(t: u8) -> Result<HapTextureFormat> {
    match t & 0x0F {
        x if x == (FMT_BC1 & 0x0F) => Ok(HapTextureFormat::Bc1),
        x if x == (FMT_BC3 & 0x0F) => Ok(HapTextureFormat::Bc3),
        x if x == (FMT_BC7 & 0x0F) => Ok(HapTextureFormat::Bc7),
        x if x == (FMT_YCOCG & 0x0F) => Ok(HapTextureFormat::Bc3YCoCg),
        x if x == (FMT_BC4 & 0x0F) => Ok(HapTextureFormat::Bc4),
        other => bail!("Unsupported HAP format nibble: 0x{:X}", other),
    }
}

fn snappy_decompress(src: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let len = snap::raw::decompress_len(src).context("Snappy len")?;
    out.resize(len, 0);
    snap::raw::Decoder::new()
        .decompress(src, out)
        .context("Snappy decompress")?;
    Ok(())
}

fn decode_section(typ: u8, data: &[u8], out: &mut Vec<u8>) -> Result<HapTextureFormat> {
    let fmt = tex_fmt(typ)?;
    match typ & 0xF0 {
        COMPRESSOR_NONE => {
            out.clear();
            out.extend_from_slice(data);
        }
        COMPRESSOR_SNAPPY => {
            snappy_decompress(data, out)?;
        }
        COMPRESSOR_COMPLEX => {
            decode_chunked(data, out)?;
        }
        c => bail!("Unknown HAP compressor: 0x{:02X}", c),
    }
    Ok(fmt)
}

fn decode_chunked(data: &[u8], output: &mut Vec<u8>) -> Result<()> {
    let h = parse_header(data)?;
    if h.section_type != SECTION_DECODE_INSTRUCTIONS {
        bail!("Expected decode instructions, got 0x{:02X}", h.section_type);
    }
    let instr = &data[h.header_size..h.header_size + h.data_length];
    let fdata = &data[h.header_size + h.data_length..];
    let (mut ct, mut st, mut ot): (&[u8], &[u8], Option<&[u8]>) = (&[], &[], None);
    let mut p = 0;
    while p < instr.len() {
        let s = parse_header(&instr[p..])?;
        let d = &instr[p + s.header_size..p + s.header_size + s.data_length];
        match s.section_type {
            CHUNK_COMPRESSOR_TABLE => ct = d,
            CHUNK_SIZE_TABLE => st = d,
            CHUNK_OFFSET_TABLE => ot = Some(d),
            _ => {}
        }
        p += s.header_size + s.data_length;
    }
    let n = ct.len();
    if st.len() < n * 4 {
        bail!("Chunk size table too short");
    }
    output.clear();
    let mut ao: usize = 0;
    let mut tmp = Vec::new();
    for i in 0..n {
        let sz =
            u32::from_le_bytes([st[i * 4], st[i * 4 + 1], st[i * 4 + 2], st[i * 4 + 3]]) as usize;
        let off = ot.map_or(ao, |o| {
            u32::from_le_bytes([o[i * 4], o[i * 4 + 1], o[i * 4 + 2], o[i * 4 + 3]]) as usize
        });
        let chunk = &fdata[off..off + sz];
        match ct[i] {
            CHUNK_UNCOMPRESSED => output.extend_from_slice(chunk),
            CHUNK_SNAPPY_ID => {
                snappy_decompress(chunk, &mut tmp)?;
                output.extend_from_slice(&tmp);
            }
            x => bail!("Unknown chunk compressor: 0x{:02X}", x),
        }
        ao += sz;
    }
    Ok(())
}

/// Result of decoding a HAP frame — single plane or dual plane (HAP Q Alpha).
pub enum HapFrame {
    /// Single texture plane (Hap, Hap Alpha, Hap Q, Hap R).
    Single { format: HapTextureFormat },
    /// Dual texture planes (HAP Q Alpha): color (YCoCg BC3) + alpha (BC4).
    DualPlane {
        color_format: HapTextureFormat,
        alpha_format: HapTextureFormat,
    },
}

/// Decode a raw HAP packet into decompressed BCn data.
/// For single-plane: data goes into `out`.
/// For dual-plane (HAP Q Alpha): color goes into `out`, alpha goes into `alpha_out`.
pub fn decode_hap_frame(
    packet_data: &[u8],
    out: &mut Vec<u8>,
    alpha_out: &mut Vec<u8>,
) -> Result<HapFrame> {
    let h = parse_header(packet_data)?;
    let section_data = &packet_data[h.header_size..h.header_size + h.data_length];

    if h.section_type == SECTION_MULTI_IMAGE {
        // Multi-image: parse all sub-sections
        let mut pos = 0;
        let mut color_fmt = None;
        let mut alpha_fmt = None;

        while pos < section_data.len() {
            let sub = parse_header(&section_data[pos..])?;
            let sub_data =
                &section_data[pos + sub.header_size..pos + sub.header_size + sub.data_length];
            let fmt = tex_fmt(sub.section_type)?;

            match fmt {
                HapTextureFormat::Bc4 => {
                    // Alpha plane
                    alpha_fmt = Some(decode_section(sub.section_type, sub_data, alpha_out)?);
                }
                _ => {
                    // Color plane
                    color_fmt = Some(decode_section(sub.section_type, sub_data, out)?);
                }
            }
            pos += sub.header_size + sub.data_length;
        }

        let cf = color_fmt.context("HAP multi-image missing color plane")?;
        let af = alpha_fmt.unwrap_or(HapTextureFormat::Bc4);
        Ok(HapFrame::DualPlane {
            color_format: cf,
            alpha_format: af,
        })
    } else {
        let fmt = decode_section(h.section_type, section_data, out)?;
        Ok(HapFrame::Single { format: fmt })
    }
}

/// Result of a HapPlayer::next_frame() call.
pub struct HapFrameResult<'a> {
    /// Color plane data (always present).
    pub color_data: &'a [u8],
    /// Color plane texture format.
    pub color_format: HapTextureFormat,
    /// Alpha plane data (only for HAP Q Alpha dual-plane frames).
    pub alpha_data: Option<&'a [u8]>,
    /// Alpha plane texture format (only for dual-plane).
    pub alpha_format: Option<HapTextureFormat>,
}

/// HAP video player — demuxes container with ffmpeg, decodes HAP frames to BCn data.
///
/// # Safety: Send
/// Same rationale as `VideoPlayer` — exclusive ownership of C-allocated ffmpeg state.
/// Transferred between threads but never accessed concurrently.
pub struct HapPlayer {
    ictx: ffmpeg::format::context::Input,
    video_stream_index: usize,
    width: u32,
    height: u32,
    texture_format: HapTextureFormat,
    /// Whether this file produces dual-plane frames (HAP Q Alpha).
    pub is_dual_plane: bool,
    /// Shared playback state (loop mode, speed, in/out points, position).
    pub playback: PlaybackState,
    /// Color plane buffer.
    frame_data: Vec<u8>,
    /// Alpha plane buffer (for dual-plane HAP Q Alpha).
    alpha_data: Vec<u8>,
}

// SAFETY: See doc comment on HapPlayer. Exclusive ownership of C allocations, no concurrent use.
unsafe impl Send for HapPlayer {}

impl HapPlayer {
    /// Create a new HAP player from a video file path.
    pub fn new<P: AsRef<Path>>(path: P, initial_format: HapTextureFormat) -> Result<Self> {
        ffmpeg::init().context("Failed to initialize FFmpeg")?;
        let ictx = input(&path).context("Failed to open HAP video")?;
        let video_stream = ictx
            .streams()
            .best(Type::Video)
            .context("No video stream")?;
        let video_stream_index = video_stream.index();
        let params = video_stream.parameters();
        let codec_ctx = ffmpeg::codec::context::Context::from_parameters(params)?;
        let decoder = codec_ctx.decoder().video()?;
        let width = decoder.width();
        let height = decoder.height();
        let rate = video_stream.rate();
        let fps = rate.0 as f64 / rate.1 as f64;
        let duration = if ictx.duration() > 0 {
            ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64
        } else {
            0.0
        };
        let color_buf = initial_format.frame_byte_size(width, height);
        let alpha_buf = HapTextureFormat::Bc4.frame_byte_size(width, height);
        log::info!(
            "HAP video: {}x{} @ {:.2}fps, {:.2}s, {:?}",
            width,
            height,
            fps,
            duration,
            initial_format
        );
        Ok(Self {
            ictx,
            video_stream_index,
            width,
            height,
            texture_format: initial_format,
            is_dual_plane: false,
            playback: PlaybackState::new(duration, fps),
            frame_data: vec![0u8; color_buf],
            alpha_data: vec![0u8; alpha_buf],
        })
    }

    /// Get the next frame as compressed BCn data.
    pub fn next_frame(&mut self) -> Result<Option<HapFrameResult<'_>>> {
        if !self.playback.playing {
            return Ok(None);
        }
        let result = self.playback.advance_frame();

        // No frames to decode — hold current (HAP doesn't have a "current frame" buffer,
        // so return None and let the caller keep the existing texture)
        if result.frames_to_decode == 0 && !result.needs_seek {
            return Ok(None);
        }

        if self.playback.reverse || result.needs_seek {
            self.seek(self.playback.position)?;
        }

        // Decode frames, skipping intermediate ones for speed > 1
        let target_frames = result.frames_to_decode.max(1);
        let mut decoded_count = 0u32;
        loop {
            match self.ictx.packets().next() {
                Some((stream, packet)) => {
                    if stream.index() == self.video_stream_index {
                        if let Some(data) = packet.data() {
                            decoded_count += 1;
                            if decoded_count >= target_frames {
                                let frame = decode_hap_frame(
                                    data,
                                    &mut self.frame_data,
                                    &mut self.alpha_data,
                                )?;
                                match frame {
                                    HapFrame::Single { format } => {
                                        self.texture_format = format;
                                        self.is_dual_plane = false;
                                        return Ok(Some(HapFrameResult {
                                            color_data: &self.frame_data,
                                            color_format: format,
                                            alpha_data: None,
                                            alpha_format: None,
                                        }));
                                    }
                                    HapFrame::DualPlane {
                                        color_format,
                                        alpha_format,
                                    } => {
                                        self.texture_format = color_format;
                                        self.is_dual_plane = true;
                                        return Ok(Some(HapFrameResult {
                                            color_data: &self.frame_data,
                                            color_format,
                                            alpha_data: Some(&self.alpha_data),
                                            alpha_format: Some(alpha_format),
                                        }));
                                    }
                                }
                            }
                            // Skip intermediate frames for speed > 1
                        }
                    }
                }
                None => {
                    // End of stream — handle loop modes inline
                    match self.playback.loop_mode {
                        LoopMode::Loop => {
                            self.playback.position = self.playback.in_point;
                            self.seek(self.playback.position)?;
                            continue;
                        }
                        LoopMode::PingPong => {
                            // EOS: flip direction and seek to the opposite boundary.
                            // advance_frame() may have already flipped `reverse`,
                            // so we set it explicitly based on which boundary we hit.
                            // Forward EOS (at end) → go reverse from out-point.
                            // Reverse EOS (at start) → go forward from in-point.
                            // Use position to determine which boundary we're near.
                            let out_pt = self.playback.effective_out();
                            let in_pt = self.playback.in_point;
                            let mid = (in_pt + out_pt) / 2.0;
                            if self.playback.position >= mid {
                                // Near end → reverse
                                self.playback.reverse = true;
                                self.playback.position = out_pt - (1.0 / self.playback.frame_rate);
                            } else {
                                // Near start → forward
                                self.playback.reverse = false;
                                self.playback.position = in_pt;
                            }
                            self.seek(self.playback.position)?;
                            continue;
                        }
                        LoopMode::OneShot => {
                            self.playback.playing = false;
                            return Ok(None);
                        }
                        LoopMode::HoldLast => {
                            return Ok(None);
                        }
                    }
                }
            }
        }
    }

    pub fn seek(&mut self, time_secs: f64) -> Result<()> {
        let ts = (time_secs * ffmpeg::ffi::AV_TIME_BASE as f64) as i64;
        self.ictx.seek(ts, ..ts)?;
        self.playback.position = time_secs;
        Ok(())
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn frame_rate(&self) -> f64 {
        self.playback.frame_rate
    }
    pub fn duration(&self) -> f64 {
        self.playback.duration
    }
    pub fn texture_format(&self) -> HapTextureFormat {
        self.texture_format
    }
    pub fn is_playing(&self) -> bool {
        self.playback.playing
    }
    pub fn set_playing(&mut self, playing: bool) {
        self.playback.playing = playing;
    }
    pub fn is_looping(&self) -> bool {
        self.playback.loop_mode == LoopMode::Loop
    }
    pub fn set_looping(&mut self, looping: bool) {
        self.playback.loop_mode = if looping {
            LoopMode::Loop
        } else {
            LoopMode::OneShot
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal HAP section: 4-byte header + payload.
    fn make_section(section_type: u8, payload: &[u8]) -> Vec<u8> {
        let len = payload.len();
        assert!(len < 0x00FF_FFFF, "payload too large for 4-byte header");
        let mut buf = vec![
            (len & 0xFF) as u8,
            ((len >> 8) & 0xFF) as u8,
            ((len >> 16) & 0xFF) as u8,
            section_type,
        ];
        buf.extend_from_slice(payload);
        buf
    }

    /// Build a HAP section with 8-byte header (for payloads >= 16 MB or when first 3 bytes are zero).
    fn make_section_long(section_type: u8, payload: &[u8]) -> Vec<u8> {
        let len = payload.len() as u32;
        let mut buf = vec![0u8, 0, 0, section_type];
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn test_parse_header_4byte() {
        let data = make_section(0xAB, &[1, 2, 3, 4, 5]);
        let h = parse_header(&data).unwrap();
        assert_eq!(h.section_type, 0xAB);
        assert_eq!(h.data_length, 5);
        assert_eq!(h.header_size, 4);
    }

    #[test]
    fn test_parse_header_8byte() {
        let data = make_section_long(0xCD, &[10, 20, 30]);
        let h = parse_header(&data).unwrap();
        assert_eq!(h.section_type, 0xCD);
        assert_eq!(h.data_length, 3);
        assert_eq!(h.header_size, 8);
    }

    #[test]
    fn test_parse_header_too_short() {
        assert!(parse_header(&[1, 2]).is_err());
    }

    #[test]
    fn test_tex_fmt_bc1() {
        assert_eq!(tex_fmt(FMT_BC1).unwrap(), HapTextureFormat::Bc1);
    }

    #[test]
    fn test_tex_fmt_bc3() {
        assert_eq!(tex_fmt(FMT_BC3).unwrap(), HapTextureFormat::Bc3);
    }

    #[test]
    fn test_tex_fmt_bc7() {
        assert_eq!(tex_fmt(FMT_BC7).unwrap(), HapTextureFormat::Bc7);
    }

    #[test]
    fn test_tex_fmt_ycocg() {
        assert_eq!(tex_fmt(FMT_YCOCG).unwrap(), HapTextureFormat::Bc3YCoCg);
    }

    #[test]
    fn test_tex_fmt_bc4() {
        assert_eq!(tex_fmt(FMT_BC4).unwrap(), HapTextureFormat::Bc4);
    }

    #[test]
    fn test_tex_fmt_unsupported() {
        assert!(tex_fmt(0x09).is_err());
    }

    #[test]
    fn test_decode_single_frame_uncompressed() {
        // BC1 uncompressed: section_type = COMPRESSOR_NONE | FMT_BC1
        let section_type = COMPRESSOR_NONE | (FMT_BC1 & 0x0F);
        let payload = vec![0xAA; 32]; // 32 bytes of fake BCn data
        let packet = make_section(section_type, &payload);

        let mut out = Vec::new();
        let mut alpha_out = Vec::new();
        let result = decode_hap_frame(&packet, &mut out, &mut alpha_out).unwrap();

        match result {
            HapFrame::Single { format } => {
                assert_eq!(format, HapTextureFormat::Bc1);
                assert_eq!(out, payload);
            }
            _ => panic!("Expected Single frame"),
        }
    }

    #[test]
    fn test_decode_single_frame_snappy() {
        // BC3 with Snappy compression
        let section_type = COMPRESSOR_SNAPPY | (FMT_BC3 & 0x0F);
        let original = vec![0xBB; 64];
        let compressed = snap::raw::Encoder::new().compress_vec(&original).unwrap();
        let packet = make_section(section_type, &compressed);

        let mut out = Vec::new();
        let mut alpha_out = Vec::new();
        let result = decode_hap_frame(&packet, &mut out, &mut alpha_out).unwrap();

        match result {
            HapFrame::Single { format } => {
                assert_eq!(format, HapTextureFormat::Bc3);
                assert_eq!(out, original);
            }
            _ => panic!("Expected Single frame"),
        }
    }

    #[test]
    fn test_decode_single_frame_ycocg() {
        let section_type = COMPRESSOR_NONE | (FMT_YCOCG & 0x0F);
        let payload = vec![0xCC; 48];
        let packet = make_section(section_type, &payload);

        let mut out = Vec::new();
        let mut alpha_out = Vec::new();
        let result = decode_hap_frame(&packet, &mut out, &mut alpha_out).unwrap();

        match result {
            HapFrame::Single { format } => {
                assert_eq!(format, HapTextureFormat::Bc3YCoCg);
                assert_eq!(out, payload);
            }
            _ => panic!("Expected Single frame"),
        }
    }
}
