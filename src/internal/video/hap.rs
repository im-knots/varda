//! HAP decoder: demuxes with ffmpeg, parses HAP frames and Snappy-decompresses
//! the `BCn` blocks for direct GPU upload.

use anyhow::{Context, Result, bail};
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

/// BC4/RGTC1, alpha only (Hap Q Alpha's alpha plane).
const FMT_BC4: u8 = 0x01;

fn tex_fmt(t: u8) -> Result<HapTextureFormat> {
    match t & 0x0F {
        x if x == (FMT_BC1 & 0x0F) => Ok(HapTextureFormat::Bc1),
        x if x == (FMT_BC3 & 0x0F) => Ok(HapTextureFormat::Bc3),
        x if x == (FMT_BC7 & 0x0F) => Ok(HapTextureFormat::Bc7),
        x if x == (FMT_YCOCG & 0x0F) => Ok(HapTextureFormat::Bc3YCoCg),
        x if x == (FMT_BC4 & 0x0F) => Ok(HapTextureFormat::Bc4),
        other => bail!("Unsupported HAP format nibble: 0x{other:X}"),
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
        c => bail!("Unknown HAP compressor: 0x{c:02X}"),
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
            x => bail!("Unknown chunk compressor: 0x{x:02X}"),
        }
        ao += sz;
    }
    Ok(())
}

/// A decoded HAP frame: one plane, or two for HAP Q Alpha.
pub enum HapFrame {
    /// Single texture plane (Hap, Hap Alpha, Hap Q, Hap R).
    Single { format: HapTextureFormat },
    /// Two planes (HAP Q Alpha): color (`YCoCg` BC3) and alpha (BC4).
    DualPlane {
        color_format: HapTextureFormat,
        alpha_format: HapTextureFormat,
    },
}

/// Decodes a HAP packet into `BCn` data. Single-plane data goes to `out`; for
/// HAP Q Alpha, color goes to `out` and alpha to `alpha_out`.
///
/// # Errors
///
/// Returns an error if the header is malformed, the section type or texture
/// format is unknown, or Snappy/LZ4 decompression fails.
pub fn decode_hap_frame(
    packet_data: &[u8],
    out: &mut Vec<u8>,
    alpha_out: &mut Vec<u8>,
) -> Result<HapFrame> {
    let h = parse_header(packet_data)?;
    let section_data = &packet_data[h.header_size..h.header_size + h.data_length];

    if h.section_type == SECTION_MULTI_IMAGE {
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

/// Reads the color-plane texture format from a packet's section headers,
/// without decompressing. ffmpeg uses one codec id for all HAP variants, so
/// this sizes the texture and staging buffers before playback.
///
/// # Errors
///
/// Returns an error if the header is malformed or no color-plane section
/// with a known texture format is present.
pub fn detect_hap_format(packet_data: &[u8]) -> Result<HapTextureFormat> {
    let h = parse_header(packet_data)?;
    if h.section_type == SECTION_MULTI_IMAGE {
        let section_data = &packet_data[h.header_size..h.header_size + h.data_length];
        let mut pos = 0;
        let mut color_fmt = None;
        while pos < section_data.len() {
            let sub = parse_header(&section_data[pos..])?;
            // The non-alpha plane carries the color format.
            if !matches!(tex_fmt(sub.section_type)?, HapTextureFormat::Bc4) {
                color_fmt = Some(tex_fmt(sub.section_type)?);
            }
            pos += sub.header_size + sub.data_length;
        }
        color_fmt.context("HAP multi-image missing color plane")
    } else {
        tex_fmt(h.section_type)
    }
}

pub struct HapFrameResult<'a> {
    pub color_data: &'a [u8],
    pub color_format: HapTextureFormat,
    /// Alpha plane data (HAP Q Alpha only).
    pub alpha_data: Option<&'a [u8]>,
    /// Alpha plane texture format (HAP Q Alpha only).
    pub alpha_format: Option<HapTextureFormat>,
}

/// HAP player: demuxes with ffmpeg and decodes HAP frames to `BCn` data.
///
/// # Safety
///
/// `Send` for the same reason as `VideoPlayer`: it exclusively owns its
/// ffmpeg state and is never used from two threads at once.
pub struct HapPlayer {
    ictx: ffmpeg::format::context::Input,
    video_stream_index: usize,
    width: u32,
    height: u32,
    texture_format: HapTextureFormat,
    /// Whether this file produces dual-plane frames (HAP Q Alpha).
    pub is_dual_plane: bool,
    /// Loop mode, speed, in/out points and position.
    pub playback: PlaybackState,
    frame_data: Vec<u8>,
    /// Alpha plane buffer (HAP Q Alpha).
    alpha_data: Vec<u8>,
}

// SAFETY: exclusive ownership of the ffmpeg allocations, no concurrent use.
unsafe impl Send for HapPlayer {}

impl HapPlayer {
    /// Opens a HAP video file.
    ///
    /// # Errors
    ///
    /// Returns an error if FFmpeg cannot be initialized, the file cannot be
    /// opened, or it has no video stream.
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
        let fps = f64::from(rate.0) / f64::from(rate.1);
        let duration = if ictx.duration() > 0 {
            ictx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
        } else {
            0.0
        };
        let color_buf = initial_format.frame_byte_size(width, height);
        let alpha_buf = HapTextureFormat::Bc4.frame_byte_size(width, height);
        log::info!("HAP video: {width}x{height} @ {fps:.2}fps, {duration:.2}s, {initial_format:?}");
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

    /// Returns the next frame as compressed `BCn` data.
    ///
    /// # Errors
    ///
    /// Returns an error if a packet fails to decode or a seek (reverse playback,
    /// loop wrap) fails.
    pub fn next_frame(&mut self) -> Result<Option<HapFrameResult<'_>>> {
        // Only suspension stops here. A paused clip still calls `advance_frame`,
        // since a modulator on the playhead can move it while paused.
        if self.playback.suspended {
            return Ok(None);
        }
        let result = self.playback.advance_frame();

        // Nothing to decode. HAP keeps no current-frame buffer, so return None and
        // the caller keeps the existing texture.
        if result.frames_to_decode == 0 && !result.needs_seek {
            return Ok(None);
        }

        if self.playback.reverse || result.needs_seek {
            self.seek(self.playback.position)?;
        }

        // Skip intermediate frames when speed > 1.
        let target_frames = result.frames_to_decode.max(1);
        let mut decoded_count = 0u32;
        // Wraps taken while fetching this frame. A wrap that yields no packet means
        // the demuxer is stuck; wrapping again would spin at 100% CPU.
        let mut wraps = 0u32;
        loop {
            let Some((stream, packet)) = self.ictx.packets().next() else {
                // `packets()` returns `None` for read errors as well as end of stream,
                // and a latched I/O error repeats on every read. Give up for this tick;
                // the caller holds the frame and the next tick retries.
                wraps += 1;
                if wraps > 1 {
                    log::warn!(
                        "HAP demuxer produced no packets after wrapping to {:.3}s — \
                         holding the current frame",
                        self.playback.position
                    );
                    return Ok(None);
                }
                // End of stream: hold while chasing, otherwise apply the loop mode.
                if self.playback.chasing {
                    return Ok(None);
                }
                match self.playback.loop_mode {
                    LoopMode::Loop => {
                        self.playback.position = self.playback.in_point;
                        self.seek(self.playback.position)?;
                    }
                    LoopMode::PingPong => {
                        // `advance_frame()` may already have flipped `reverse`, so set it
                        // from which boundary the position is nearer.
                        let out_pt = self.playback.effective_out();
                        let in_pt = self.playback.in_point;
                        let mid = f64::midpoint(in_pt, out_pt);
                        if self.playback.position >= mid {
                            self.playback.reverse = true;
                            self.playback.position = out_pt - (1.0 / self.playback.frame_rate);
                        } else {
                            self.playback.reverse = false;
                            self.playback.position = in_pt;
                        }
                        self.seek(self.playback.position)?;
                    }
                    LoopMode::OneShot => {
                        self.playback.playing = false;
                        return Ok(None);
                    }
                    LoopMode::HoldLast => {
                        return Ok(None);
                    }
                }
                continue;
            };

            if stream.index() != self.video_stream_index {
                continue;
            }
            let Some(data) = packet.data() else {
                continue;
            };
            decoded_count += 1;
            if decoded_count < target_frames {
                continue;
            }

            let frame = decode_hap_frame(data, &mut self.frame_data, &mut self.alpha_data)?;
            return Ok(Some(match frame {
                HapFrame::Single { format } => {
                    self.texture_format = format;
                    self.is_dual_plane = false;
                    HapFrameResult {
                        color_data: &self.frame_data,
                        color_format: format,
                        alpha_data: None,
                        alpha_format: None,
                    }
                }
                HapFrame::DualPlane {
                    color_format,
                    alpha_format,
                } => {
                    self.texture_format = color_format;
                    self.is_dual_plane = true;
                    HapFrameResult {
                        color_data: &self.frame_data,
                        color_format,
                        alpha_data: Some(&self.alpha_data),
                        alpha_format: Some(alpha_format),
                    }
                }
            }));
        }
    }

    /// Seeks the demuxer to `time_secs` and updates the playback position.
    ///
    /// # Errors
    ///
    /// Returns an error if the FFmpeg seek fails.
    pub fn seek(&mut self, time_secs: f64) -> Result<()> {
        let ts = (time_secs * f64::from(ffmpeg::ffi::AV_TIME_BASE)) as i64;
        // Clear latched EOF or I/O errors; otherwise every later read repeats
        // them and the loop wrap never gets another packet.
        self.ictx.clear_eof();
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

    /// Builds a HAP section with a 4-byte header.
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

    /// Builds a HAP section with an 8-byte header (payloads >= 16 MB, or when the
    /// first 3 bytes are zero).
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
        // BC1, uncompressed.
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
            HapFrame::DualPlane { .. } => panic!("Expected Single frame"),
        }
    }

    #[test]
    fn test_decode_single_frame_snappy() {
        // BC3, Snappy.
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
            HapFrame::DualPlane { .. } => panic!("Expected Single frame"),
        }
    }

    #[test]
    fn detect_hap_format_single_bc1() {
        let packet = make_section(COMPRESSOR_NONE | (FMT_BC1 & 0x0F), &[0xAA; 16]);
        assert_eq!(detect_hap_format(&packet).unwrap(), HapTextureFormat::Bc1);
    }

    #[test]
    fn detect_hap_format_single_bc7() {
        let packet = make_section(COMPRESSOR_SNAPPY | (FMT_BC7 & 0x0F), &[0xBB; 16]);
        assert_eq!(detect_hap_format(&packet).unwrap(), HapTextureFormat::Bc7);
    }

    #[test]
    fn detect_hap_format_multi_image_returns_color_plane() {
        // HAP Q Alpha: YCoCg color plane plus BC4 alpha plane.
        let color = make_section(COMPRESSOR_NONE | (FMT_YCOCG & 0x0F), &[0xCC; 16]);
        let alpha = make_section(COMPRESSOR_NONE | (FMT_BC4 & 0x0F), &[0xDD; 8]);
        let mut payload = color;
        payload.extend_from_slice(&alpha);
        let packet = make_section(SECTION_MULTI_IMAGE, &payload);
        assert_eq!(
            detect_hap_format(&packet).unwrap(),
            HapTextureFormat::Bc3YCoCg
        );
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
            HapFrame::DualPlane { .. } => panic!("Expected Single frame"),
        }
    }
}
