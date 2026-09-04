//! `FfmpegSubprocess` — shared ffmpeg lifecycle for recording and SRT streaming.
//!
//! Spawns an ffmpeg process with a background writer thread that feeds frames
//! via a bounded channel. The render thread never blocks on pipe writes — if
//! ffmpeg can't keep up (e.g. SRT listener waiting for client), frames are dropped.

use std::io::Write;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;

use crate::audio::PcmChunk;
use crate::engine::value::render::{
    AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
    PresentationRequest, RecordingCodec, ResolvedPresentation, RtmpCodecContract, SrtCodec,
    StreamingCodec,
};
use crate::renderer::{ReadbackFormat, ReadbackFrame};

/// Fully pinned video contract for one recording subprocess.
///
/// Resolution happens before the writer queue starts. A ten-bit request is only
/// accepted when both the encoder and the typed GPU readback can carry it.
#[derive(Debug, Clone)]
struct RecordingPlan {
    /// Runtime precision and encoded pixel format reported to output status.
    resolved: ResolvedPresentation,
    input_pixel_format: &'static str,
    output_pixel_format: &'static str,
    expected_readback: ReadbackFormat,
    video_args: Vec<String>,
    unpremultiply: bool,
}

impl RecordingPlan {
    fn resolve(
        codec: &RecordingCodec,
        request: PresentationRequest,
        readback: ReadbackFormat,
        encoder_help: Option<&str>,
    ) -> Self {
        let encoder = recording_encoder(codec);
        let ten_bit_output = ten_bit_output_format(codec);
        let codec_supports_ten_bit = ten_bit_output.is_some();
        let encoder_supports_ten_bit = ten_bit_output.is_some_and(|format| {
            encoder_help.is_some_and(|help| encoder_supports_pixel_format(help, format))
        });
        let required_readback = if matches!(codec, RecordingCodec::ProRes4444) {
            ReadbackFormat::Rgba16Unorm
        } else {
            ReadbackFormat::Rgb10A2
        };

        let fallback_reason = if request.depth != PresentationDepth::Sdr10 {
            None
        } else if !codec_supports_ten_bit {
            Some(format!(
                "{codec} is an eight-bit recording codec; 10-bit SDR is unavailable"
            ))
        } else if matches!(codec, RecordingCodec::ProRes4444)
            && readback == ReadbackFormat::Rgba16Float
        {
            Some(
                "10-bit ProRes 4444 requires integer RGBA16 readback; RGBA16 half-float bytes \
                 cannot be passed to FFmpeg as rgba64le"
                    .to_string(),
            )
        } else if matches!(codec, RecordingCodec::ProRes4444)
            && readback != ReadbackFormat::Rgba16Unorm
        {
            Some(
                "10-bit ProRes 4444 is unavailable because the GPU does not support normalized \
                 16-bit RGBA render targets"
                    .to_string(),
            )
        } else if required_readback != readback {
            Some(format!(
                "10-bit SDR recording requires packed RGB10 readback, but the active renderer \
                 supplies {}",
                readback_label(readback)
            ))
        } else if encoder_help.is_none() {
            Some(format!(
                "FFmpeg encoder '{encoder}' is unavailable or could not be queried"
            ))
        } else if !encoder_supports_ten_bit {
            Some(format!(
                "FFmpeg encoder '{encoder}' does not support {}",
                ten_bit_output.expect("ten-bit codec has an output format")
            ))
        } else {
            None
        };

        let use_ten_bit = request.depth == PresentationDepth::Sdr10 && fallback_reason.is_none();
        let (input_pixel_format, output_pixel_format, expected_readback) = if use_ten_bit {
            if matches!(codec, RecordingCodec::ProRes4444) {
                (
                    "rgba64le",
                    ten_bit_output.expect("resolved ten-bit recording has an output format"),
                    ReadbackFormat::Rgba16Unorm,
                )
            } else {
                (
                    packed_rgb10_input_format(),
                    ten_bit_output.expect("resolved ten-bit recording has an output format"),
                    ReadbackFormat::Rgb10A2,
                )
            }
        } else {
            (
                "rgba",
                eight_bit_output_format(codec),
                ReadbackFormat::Rgba8,
            )
        };
        let alpha_mode = if codec_preserves_alpha(codec) {
            AlphaMode::Straight
        } else {
            AlphaMode::Opaque
        };
        let resolved_depth = if use_ten_bit {
            PresentationDepth::Sdr10
        } else {
            PresentationDepth::Sdr8
        };
        let pixel_format = if use_ten_bit && matches!(codec, RecordingCodec::ProRes4444) {
            PresentationPixelFormat::Rgba16
        } else {
            PresentationPixelFormat::EncoderNative(output_pixel_format.to_string())
        };
        let resolved = ResolvedPresentation {
            requested: request.depth,
            resolved: resolved_depth,
            pixel_format,
            color_profile: PresentationColorProfile::Rec709Limited,
            alpha_mode,
            dither: request.dither,
            fallback_reason,
        };
        let video_args = recording_video_args(codec, resolved_depth, output_pixel_format);

        Self {
            resolved,
            input_pixel_format,
            output_pixel_format,
            expected_readback,
            video_args,
            unpremultiply: codec_preserves_alpha(codec),
        }
    }
}

fn recording_encoder(codec: &RecordingCodec) -> &'static str {
    match codec {
        RecordingCodec::H264 => "libx264",
        RecordingCodec::H265 => "libx265",
        RecordingCodec::AV1 => "libsvtav1",
        RecordingCodec::ProRes | RecordingCodec::ProRes4444 => "prores_ks",
        RecordingCodec::Hap | RecordingCodec::HapAlpha | RecordingCodec::HapQ => "hap",
    }
}

fn ten_bit_output_format(codec: &RecordingCodec) -> Option<&'static str> {
    match codec {
        RecordingCodec::H265 | RecordingCodec::AV1 => Some("yuv420p10le"),
        RecordingCodec::ProRes => Some("yuv422p10le"),
        RecordingCodec::ProRes4444 => Some("yuva444p10le"),
        RecordingCodec::H264
        | RecordingCodec::Hap
        | RecordingCodec::HapAlpha
        | RecordingCodec::HapQ => None,
    }
}

fn eight_bit_output_format(codec: &RecordingCodec) -> &'static str {
    match codec {
        RecordingCodec::H264 | RecordingCodec::H265 | RecordingCodec::AV1 => "yuv420p",
        RecordingCodec::ProRes => "yuv422p10le",
        RecordingCodec::ProRes4444 => "yuva444p10le",
        RecordingCodec::Hap | RecordingCodec::HapAlpha | RecordingCodec::HapQ => "rgba",
    }
}

fn codec_preserves_alpha(codec: &RecordingCodec) -> bool {
    matches!(codec, RecordingCodec::ProRes4444 | RecordingCodec::HapAlpha)
}

fn encoder_supports_pixel_format(help: &str, pixel_format: &str) -> bool {
    help.lines()
        .find(|line| line.trim_start().starts_with("Supported pixel formats:"))
        .is_some_and(|line| line.split_whitespace().any(|item| item == pixel_format))
}

fn readback_label(format: ReadbackFormat) -> &'static str {
    match format {
        ReadbackFormat::Rgba8 => "RGBA8",
        ReadbackFormat::Bgra8 => "BGRA8",
        ReadbackFormat::Rgb10A2 => "RGB10A2",
        ReadbackFormat::Rgba16Float => "RGBA16 half-float",
        ReadbackFormat::Rgba16Unorm => "RGBA16",
        ReadbackFormat::Uyvy => "UYVY",
        ReadbackFormat::P216 => "P216",
    }
}

fn preferred_recording_readback(
    codec: &RecordingCodec,
    request: PresentationRequest,
    rgba16_unorm_supported: bool,
) -> ReadbackFormat {
    if request.depth != PresentationDepth::Sdr10 {
        ReadbackFormat::Rgba8
    } else if matches!(codec, RecordingCodec::ProRes4444) && rgba16_unorm_supported {
        ReadbackFormat::Rgba16Unorm
    } else if matches!(codec, RecordingCodec::ProRes4444) {
        ReadbackFormat::Rgba8
    } else {
        ReadbackFormat::Rgb10A2
    }
}

/// wgpu `Rgb10a2Unorm` stores R in bits 0..9, G in 10..19, and B in
/// 20..29. FFmpeg names that little-endian word `x2bgr10le` (the component
/// names describe most-significant to least-significant fields).
const fn packed_rgb10_input_format() -> &'static str {
    "x2bgr10le"
}

fn unpremultiply_filter(input_pixel_format: &str) -> &'static str {
    if input_pixel_format == "rgba64le" {
        "setparams=alpha_mode=premultiplied,format=gbrap16le,unpremultiply=inplace=1"
    } else {
        "setparams=alpha_mode=premultiplied,format=gbrap,unpremultiply=inplace=1"
    }
}

fn recording_video_args(
    codec: &RecordingCodec,
    depth: PresentationDepth,
    output_pixel_format: &str,
) -> Vec<String> {
    let mut args: Vec<String> = match codec {
        RecordingCodec::H264 => vec![
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-profile:v",
            "high",
        ],
        RecordingCodec::H265 => {
            let profile = if depth == PresentationDepth::Sdr10 {
                "main10"
            } else {
                "main"
            };
            vec![
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-crf",
                "20",
                "-profile:v",
                profile,
            ]
        }
        RecordingCodec::AV1 => vec![
            "-c:v",
            "libsvtav1",
            "-preset",
            "10",
            "-crf",
            "28",
            "-profile:v",
            "0",
        ],
        RecordingCodec::ProRes => {
            vec!["-c:v", "prores_ks", "-profile:v", "2"]
        }
        RecordingCodec::ProRes4444 => {
            vec!["-c:v", "prores_ks", "-profile:v", "4", "-alpha_bits", "16"]
        }
        RecordingCodec::Hap => vec!["-c:v", "hap", "-format", "hap"],
        RecordingCodec::HapAlpha => vec!["-c:v", "hap", "-format", "hap_alpha"],
        RecordingCodec::HapQ => vec!["-c:v", "hap", "-format", "hap_q"],
    }
    .into_iter()
    .map(str::to_string)
    .collect();
    args.extend(
        [
            "-pix_fmt",
            output_pixel_format,
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
        ]
        .into_iter()
        .map(str::to_string),
    );
    args
}

fn probe_encoder_help(encoder: &str) -> Option<String> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-h", &format!("encoder={encoder}")])
        .output()
        .ok()?;
    output.status.success().then(|| {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        text
    })
}

#[derive(Debug, Clone, Copy, Default)]
// Each flag is an independent result from an installed FFmpeg capability probe.
#[allow(clippy::struct_excessive_bools)]
struct StreamingCapabilities {
    hevc_main10: bool,
    av1_10: bool,
    enhanced_hevc: bool,
    enhanced_av1: bool,
}

impl StreamingCapabilities {
    fn detect() -> Self {
        let hevc_help = probe_encoder_help("libx265");
        let av1_help = probe_encoder_help("libsvtav1");
        let hevc_main10 = hevc_help
            .as_deref()
            .is_some_and(|help| encoder_supports_pixel_format(help, "yuv420p10le"));
        let av1_10 = av1_help
            .as_deref()
            .is_some_and(|help| encoder_supports_pixel_format(help, "yuv420p10le"));
        Self {
            hevc_main10,
            av1_10,
            enhanced_hevc: hevc_main10 && probe_enhanced_flv("libx265", "main10"),
            enhanced_av1: av1_10 && probe_enhanced_flv("libsvtav1", "main"),
        }
    }

    fn installed() -> Self {
        static CAPABILITIES: std::sync::OnceLock<StreamingCapabilities> =
            std::sync::OnceLock::new();
        *CAPABILITIES.get_or_init(Self::detect)
    }
}

fn probe_enhanced_flv(encoder: &str, profile: &str) -> bool {
    Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=16x16:rate=1",
            "-frames:v",
            "1",
            "-c:v",
            encoder,
            "-pix_fmt",
            "yuv420p10le",
            "-profile:v",
            profile,
            "-f",
            "flv",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingProtocol {
    Srt,
    Hls,
    Dash,
    Rtmp(RtmpCodecContract),
}

/// Fully negotiated codec, pixel format, metadata, and muxer contract for a stream.
#[derive(Debug, Clone)]
pub(crate) struct StreamingPlan {
    /// Runtime precision and encoded pixel format reported to output status.
    pub resolved: ResolvedPresentation,
    input_pixel_format: &'static str,
    expected_readback: ReadbackFormat,
    effective_codec: StreamingCodec,
    video_args: Vec<String>,
    muxer_args: Vec<String>,
}

impl StreamingPlan {
    pub(crate) const fn expected_readback(&self) -> ReadbackFormat {
        self.expected_readback
    }

    fn resolve(
        protocol: StreamingProtocol,
        configured_codec: StreamingCodec,
        request: PresentationRequest,
        capabilities: StreamingCapabilities,
    ) -> Self {
        let unavailable = if request.depth == PresentationDepth::Sdr10 {
            match (protocol, &configured_codec) {
                (_, StreamingCodec::H264) => {
                    Some("H.264 streaming is limited to eight-bit SDR".to_string())
                }
                (StreamingProtocol::Srt, StreamingCodec::AV1) => {
                    Some("AV1 is not supported by the interoperable SRT MPEG-TS path".to_string())
                }
                (
                    StreamingProtocol::Srt | StreamingProtocol::Hls | StreamingProtocol::Dash,
                    StreamingCodec::H265,
                ) if !capabilities.hevc_main10 => {
                    Some("installed FFmpeg encoder lacks HEVC Main10".to_string())
                }
                (StreamingProtocol::Hls | StreamingProtocol::Dash, StreamingCodec::AV1)
                    if !capabilities.av1_10 =>
                {
                    Some("installed FFmpeg encoder lacks AV1 10-bit support".to_string())
                }
                (StreamingProtocol::Rtmp(RtmpCodecContract::Legacy), _) => {
                    Some("the endpoint contract is legacy RTMP".to_string())
                }
                (StreamingProtocol::Rtmp(RtmpCodecContract::Enhanced), StreamingCodec::H265)
                    if !capabilities.enhanced_hevc =>
                {
                    Some("FFmpeg cannot mux HEVC Main10 with Enhanced RTMP signaling".to_string())
                }
                (StreamingProtocol::Rtmp(RtmpCodecContract::Enhanced), StreamingCodec::AV1)
                    if !capabilities.enhanced_av1 =>
                {
                    Some("FFmpeg cannot mux AV1 10-bit with Enhanced RTMP signaling".to_string())
                }
                _ => None,
            }
        } else {
            None
        };
        let use_ten_bit = request.depth == PresentationDepth::Sdr10 && unavailable.is_none();
        let legacy_rtmp = matches!(protocol, StreamingProtocol::Rtmp(RtmpCodecContract::Legacy));
        let effective_codec = if legacy_rtmp
            || (request.depth == PresentationDepth::Sdr10 && unavailable.is_some())
            || (matches!(protocol, StreamingProtocol::Srt)
                && configured_codec == StreamingCodec::AV1)
        {
            StreamingCodec::H264
        } else {
            configured_codec
        };
        let resolved_depth = if use_ten_bit {
            PresentationDepth::Sdr10
        } else {
            PresentationDepth::Sdr8
        };
        let output_pixel_format = if use_ten_bit {
            "yuv420p10le"
        } else {
            "yuv420p"
        };
        let input_pixel_format = if use_ten_bit {
            packed_rgb10_input_format()
        } else {
            "rgba"
        };
        let expected_readback = if use_ten_bit {
            ReadbackFormat::Rgb10A2
        } else {
            ReadbackFormat::Rgba8
        };
        let video_args =
            streaming_video_args(&effective_codec, resolved_depth, output_pixel_format);
        let muxer_args = streaming_muxer_args(protocol, use_ten_bit, &effective_codec);
        let resolved = ResolvedPresentation {
            requested: request.depth,
            resolved: resolved_depth,
            pixel_format: PresentationPixelFormat::EncoderNative(output_pixel_format.to_string()),
            color_profile: PresentationColorProfile::Rec709Limited,
            alpha_mode: AlphaMode::Opaque,
            dither: request.dither,
            fallback_reason: unavailable,
        };
        Self {
            resolved,
            input_pixel_format,
            expected_readback,
            effective_codec,
            video_args,
            muxer_args,
        }
    }

    pub(crate) fn for_target(
        target: &crate::engine::value::render::OutputTarget,
        request: PresentationRequest,
    ) -> Option<Self> {
        let capabilities = StreamingCapabilities::installed();
        match target {
            crate::engine::value::render::OutputTarget::SrtStream { codec, .. } => {
                let codec = match codec {
                    SrtCodec::H264 => StreamingCodec::H264,
                    SrtCodec::H265 => StreamingCodec::H265,
                };
                Some(Self::resolve(
                    StreamingProtocol::Srt,
                    codec,
                    request,
                    capabilities,
                ))
            }
            crate::engine::value::render::OutputTarget::HlsStream { codec, .. } => Some(
                Self::resolve(StreamingProtocol::Hls, codec.clone(), request, capabilities),
            ),
            crate::engine::value::render::OutputTarget::DashStream { codec, .. } => {
                Some(Self::resolve(
                    StreamingProtocol::Dash,
                    codec.clone(),
                    request,
                    capabilities,
                ))
            }
            crate::engine::value::render::OutputTarget::RtmpStream {
                codec,
                codec_contract,
                ..
            } => Some(Self::resolve(
                StreamingProtocol::Rtmp(*codec_contract),
                codec.clone(),
                request,
                capabilities,
            )),
            _ => None,
        }
    }
}

fn streaming_video_args(
    codec: &StreamingCodec,
    depth: PresentationDepth,
    output_pixel_format: &str,
) -> Vec<String> {
    let mut args: Vec<String> = match codec {
        StreamingCodec::H264 => {
            vec![
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-tune",
                "zerolatency",
                "-profile:v",
                "high",
            ]
        }
        StreamingCodec::H265 => vec![
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-profile:v",
            if depth == PresentationDepth::Sdr10 {
                "main10"
            } else {
                "main"
            },
        ],
        StreamingCodec::AV1 => vec!["-c:v", "libsvtav1", "-preset", "10", "-profile:v", "0"],
    }
    .into_iter()
    .map(str::to_string)
    .collect();
    args.extend(
        [
            "-pix_fmt",
            output_pixel_format,
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
        ]
        .into_iter()
        .map(str::to_string),
    );
    args
}

fn streaming_muxer_args(
    protocol: StreamingProtocol,
    ten_bit: bool,
    codec: &StreamingCodec,
) -> Vec<String> {
    let args: Vec<&str> = match protocol {
        StreamingProtocol::Srt => vec!["-f", "mpegts"],
        StreamingProtocol::Hls if ten_bit => vec!["-tag:v", codec_tag(codec), "-f", "hls"],
        StreamingProtocol::Hls => vec!["-f", "hls"],
        StreamingProtocol::Dash => vec!["-tag:v", codec_tag(codec), "-f", "dash"],
        StreamingProtocol::Rtmp(RtmpCodecContract::Enhanced) if ten_bit => {
            vec!["-flvflags", "no_sequence_end+no_metadata", "-f", "flv"]
        }
        StreamingProtocol::Rtmp(_) => vec!["-f", "flv"],
    };
    args.into_iter().map(str::to_string).collect()
}

fn codec_tag(codec: &StreamingCodec) -> &'static str {
    match codec {
        StreamingCodec::H264 => "avc1",
        StreamingCodec::H265 => "hvc1",
        StreamingCodec::AV1 => "av01",
    }
}

fn hls_segment_args(dir: &str, low_latency: bool, use_fmp4: bool) -> Vec<String> {
    let (time, list_size, flags, extension) = if low_latency {
        ("1", "6", "independent_segments+delete_segments", "m4s")
    } else if use_fmp4 {
        ("2", "30", "delete_segments+independent_segments", "m4s")
    } else {
        ("2", "30", "delete_segments", "ts")
    };
    let mut args = vec![
        "-hls_time".to_string(),
        time.to_string(),
        "-hls_list_size".to_string(),
        list_size.to_string(),
        "-hls_flags".to_string(),
        flags.to_string(),
    ];
    if low_latency || use_fmp4 {
        args.extend([
            "-hls_segment_type".to_string(),
            "fmp4".to_string(),
            "-hls_fmp4_init_filename".to_string(),
            "init.mp4".to_string(),
        ]);
    }
    args.extend([
        "-hls_segment_filename".to_string(),
        format!("{dir}/seg_%05d.{extension}"),
    ]);
    args
}

/// Write a self-contained HTML player page into a stream directory.
/// Uses hls.js for HLS streams and dash.js for DASH streams.
/// For LL-HLS, enables hls.js low-latency mode with live-edge tuning.
fn write_stream_player(dir: &str, kind: &str, manifest_filename: &str, low_latency: bool) {
    let (lib_url, lib_setup) = match kind {
        "hls" if low_latency => (
            "https://cdn.jsdelivr.net/npm/hls.js@latest",
            format!(
                r"if(Hls.isSupported()){{var h=new Hls({{lowLatencyMode:true,liveSyncDurationCount:2,liveMaxLatencyDurationCount:4,maxBufferLength:4,backBufferLength:0}});h.loadSource('{manifest_filename}');h.attachMedia(v);}}else if(v.canPlayType('application/vnd.apple.mpegurl')){{v.src='{manifest_filename}';}}",
            ),
        ),
        "hls" => (
            "https://cdn.jsdelivr.net/npm/hls.js@latest",
            format!(
                r"if(Hls.isSupported()){{var h=new Hls();h.loadSource('{manifest_filename}');h.attachMedia(v);}}else if(v.canPlayType('application/vnd.apple.mpegurl')){{v.src='{manifest_filename}';}}",
            ),
        ),
        _ => (
            "https://cdn.jsdelivr.net/npm/dashjs@latest/dist/dash.all.min.js",
            format!(
                r"var p=dashjs.MediaPlayer().create();p.updateSettings({{streaming:{{delay:{{liveDelay:2}},buffer:{{fastSwitchEnabled:true}}}}}});p.initialize(v,'{manifest_filename}',true);v.play().catch(function(){{}});",
            ),
        ),
    };
    let title = if low_latency {
        format!("LL-{}", kind.to_uppercase())
    } else {
        kind.to_uppercase()
    };
    let html = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Varda — {title} stream</title>
<style>*{{margin:0;padding:0;background:#000}}video{{width:100vw;height:100vh;object-fit:contain}}</style>
<script src="{lib_url}"></script></head>
<body><video id="v" autoplay muted controls></video>
<script>var v=document.getElementById('v');{lib_setup}</script></body></html>"#,
    );
    let path = format!("{dir}/player.html");
    if let Err(e) = std::fs::write(&path, html) {
        log::warn!("Failed to write stream player to '{path}': {e}");
    }
}

/// Shared ffmpeg subprocess for recording and SRT streaming.
///
/// Frames are sent to a background writer thread via a bounded channel.
/// This prevents the render thread from blocking when ffmpeg's stdin pipe is full
/// (e.g. SRT listener waiting for a client connection).
pub struct FfmpegSubprocess {
    child: Child,
    /// Channel sender for frame data → writer thread
    frame_tx: Option<mpsc::SyncSender<Vec<u8>>>,
    /// Writer thread handle
    writer_thread: Option<std::thread::JoinHandle<()>>,
    /// Frame tallies, shared with the writer thread.
    counters: FrameCounters,
    /// Writer thread error flag (set when write fails during normal operation)
    write_failed: Arc<AtomicBool>,
    /// Set by `stop()` before killing ffmpeg — tells the writer thread that a
    /// broken pipe is expected and should not be logged as ERROR.
    shutting_down: Arc<AtomicBool>,
    /// Human-readable label (path or URL)
    label: String,
    /// Start time (for duration display)
    start_time: std::time::Instant,
    /// Whether `stop()` has already been called (prevent double-wait)
    stopped: bool,
    /// Optional audio passthrough side-channel (None = video-only).
    audio: Option<AudioPipe>,
    /// When true, `stop()` closes stdin and waits for ffmpeg to exit naturally
    /// (so it can finalize the container — e.g. write the MP4 moov atom).
    /// When false, `stop()` kills ffmpeg immediately (safe for streams, required
    /// when the writer thread may be blocked on a full network pipe).
    graceful_shutdown: bool,
    /// Typed frame contract negotiated before the FFmpeg process starts.
    frame_contract: Option<FrameContract>,
    /// Resolved precision selected by the recording or streaming adapter.
    presentation: Option<ResolvedPresentation>,
}

#[derive(Debug, Clone, Copy)]
struct FrameContract {
    format: ReadbackFormat,
    width: u32,
    height: u32,
}

/// Bounded channel capacity — 2 frames of buffer allows the writer thread
/// to stay one frame ahead without accumulating unbounded latency.
const FRAME_CHANNEL_CAPACITY: usize = 2;

/// Ceiling on repeated frames emitted to cover one gap, in
/// [`FfmpegSubprocess::start_writer_thread`]. Half a second at 60 fps: long
/// enough to ride out any hitch worth correcting, short enough that a genuine
/// freeze degrades into a shortened timeline rather than a burst of writes into
/// a pipe that is already struggling.
const MAX_PAD_FRAMES_PER_ARRIVAL: u64 = 30;

/// What the writer thread has put down the pipe, split by where it came from.
///
/// `written` counts frames the renderer actually produced and is the health
/// stat; `padded` counts repeats the writer inserted to cover gaps where it
/// did not. The two together are the length of the video timeline.
#[derive(Clone)]
struct FrameCounters {
    written: Arc<AtomicU64>,
    padded: Arc<AtomicU64>,
    /// Frames the render thread offered while the writer channel was full.
    /// Distinct from `padded`: a drop never entered the pipe.
    dropped: Arc<AtomicU64>,
}

impl FrameCounters {
    fn new() -> Self {
        Self {
            written: Arc::new(AtomicU64::new(0)),
            padded: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }
}

/// Offer one video frame to the writer without blocking.
///
/// `Ok` / `Full` both return true: the subprocess is still alive. `Full`
/// increments `dropped` so the operator can see backpressure. `Disconnected`
/// returns false.
fn try_enqueue_frame(
    tx: &mpsc::SyncSender<Vec<u8>>,
    dropped: &AtomicU64,
    label: &str,
    rgba: &[u8],
) -> bool {
    match tx.try_send(rgba.to_vec()) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(_)) => {
            let n = dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 || n.is_multiple_of(120) {
                log::warn!("ffmpeg video backpressure on '{label}': dropped {n} frames");
            }
            true
        }
        Err(mpsc::TrySendError::Disconnected(_)) => false,
    }
}

/// Compute video/buffer bitrate in kbps for RTMP output based on resolution and frame rate.
fn compute_rtmp_bitrate(width: u32, height: u32, fps: u32) -> (u32, u32) {
    let pixels = width * height;
    let base = match pixels {
        p if p <= 921_600 => 3000,   // ≤720p
        p if p <= 2_073_600 => 6000, // ≤1080p
        p if p <= 3_686_400 => 9000, // ≤1440p
        _ => 15000,                  // 4K+
    };
    let maxrate = if fps > 30 { base * 3 / 2 } else { base };
    (maxrate, maxrate * 2)
}

/// AAC output bitrate for passthrough audio.
const AUDIO_BITRATE: &str = "192k";
/// Normalized sample rate for streaming targets (Twitch/YouTube expect 48k AAC).
const STREAM_SAMPLE_RATE: &str = "48000";

/// Optional second (audio) input for an ffmpeg subprocess: a stream of raw
/// interleaved `f32` PCM plus the capture device's native format. Built from an
/// `AudioManager` PCM subscription; `None` keeps the byte-for-byte video-only path.
pub struct AudioInput {
    /// Raw interleaved PCM, drained by the audio writer thread into the socket.
    pub rx: crossbeam_channel::Receiver<PcmChunk>,
    /// Device native sample rate (Hz).
    pub sample_rate: u32,
    /// Device native channel count.
    pub channels: u16,
    /// Samples the capture callback discarded because this subscriber's channel
    /// was full. The writer replaces them with silence so the sample count keeps
    /// matching elapsed time — see [`AudioPipe::start`].
    pub lost_samples: Arc<AtomicU64>,
}

/// ffmpeg argument vectors + the live listener/receiver, computed before the
/// `Command` is assembled so audio input args can be interleaved after the video
/// input and audio output args before the destination.
struct PreparedAudio {
    in_args: Vec<String>,
    out_args: Vec<String>,
    listener: TcpListener,
    rx: crossbeam_channel::Receiver<PcmChunk>,
    lost_samples: Arc<AtomicU64>,
}

/// Build the ffmpeg audio input/output args and bind the loopback TCP endpoint
/// for an optional audio passthrough. `is_stream` selects the sample-rate policy:
/// native rate for Recording, normalized 48k for streaming targets (Decision 5).
fn prepare_audio(
    audio: Option<AudioInput>,
    is_stream: bool,
) -> anyhow::Result<Option<PreparedAudio>> {
    let Some(audio) = audio else {
        return Ok(None);
    };
    let (listener, audio_url) = create_audio_endpoint()?;
    // Input opts (must precede the audio `-i`); f32le matches the raw PCM tap.
    //
    // Timestamps come from the sample count, which is ffmpeg's default for a raw
    // input: sample N sits at N/sample_rate. This used to pass
    // `-use_wallclock_as_timestamps 1`, stamping each buffer with the moment it
    // arrived over the socket, and that was the cause of audio breaking up
    // whenever the renderer hitched. Arrival time is not a clock — it carries
    // scheduler jitter, and it stalls outright when ffmpeg stops draining the
    // socket to wait on the video pipe. Every one of those stalls was written
    // into the file as a timing hole.
    //
    // The capture device's sample clock has none of those problems: the hardware
    // delivers exactly `sample_rate` samples per second no matter what the rest
    // of the process is doing. It is the most accurate clock available here, so
    // it is the one the recording is built on. See /spec/av-sync.md.
    let in_args = vec![
        "-f".into(),
        "f32le".into(),
        "-ar".into(),
        audio.sample_rate.to_string(),
        "-ac".into(),
        audio.channels.to_string(),
        "-i".into(),
        audio_url,
    ];
    // Output opts: AAC, stereo downmix (Decision: stereo for v1), async resample
    // to absorb A/V drift; force 48k on streams, leave native on recordings.
    let mut out_args = vec![
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        AUDIO_BITRATE.into(),
        "-ac".into(),
        "2".into(),
        "-af".into(),
        "aresample=async=1:first_pts=0".into(),
    ];
    if is_stream {
        out_args.push("-ar".into());
        out_args.push(STREAM_SAMPLE_RATE.into());
    }
    // Explicit stream mapping once a second input exists.
    out_args.push("-map".into());
    out_args.push("0:v:0".into());
    out_args.push("-map".into());
    out_args.push("1:a:0".into());
    Ok(Some(PreparedAudio {
        in_args,
        out_args,
        listener,
        rx: audio.rx,
        lost_samples: audio.lost_samples,
    }))
}

/// Bind a loopback TCP listener on an ephemeral port and return it with the
/// `tcp://127.0.0.1:<port>` URL ffmpeg connects to as the audio input. Loopback
/// TCP is the cross-platform second-input transport (no `mkfifo`/named pipes and
/// no new crate, per the audio-passthrough transport decision).
fn create_audio_endpoint() -> anyhow::Result<(TcpListener, String)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| anyhow::anyhow!("Failed to bind audio TCP listener: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| anyhow::anyhow!("Failed to read audio listener address: {e}"))?
        .port();
    Ok((listener, format!("tcp://127.0.0.1:{port}")))
}

/// Start the audio writer thread for a prepared passthrough, if any. Called
/// after the ffmpeg child is spawned so the writer can accept ffmpeg's connection.
fn finalize_audio(
    prepared: Option<PreparedAudio>,
    label: String,
) -> anyhow::Result<Option<AudioPipe>> {
    match prepared {
        Some(p) => Ok(Some(AudioPipe::start(
            p.listener,
            p.rx,
            p.lost_samples,
            label,
        )?)),
        None => Ok(None),
    }
}

/// Audio side-channel for an [`FfmpegSubprocess`]: a loopback TCP connection plus
/// a writer thread that drains raw PCM into it, symmetric with the video writer.
pub struct AudioPipe {
    /// Set before teardown so an expected broken pipe isn't logged as ERROR.
    shutting_down: Arc<AtomicBool>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
    /// PCM chunks written to the socket so far (health stat).
    frames_written: Arc<AtomicU64>,
    /// Samples of silence spliced in to replace PCM lost to backpressure.
    silence_spliced: Arc<AtomicU64>,
}

/// Samples of silence written per `write_all` when filling a gap. Only used on
/// the rare backpressure path, so a modest buffer is plenty.
const SILENCE_BLOCK: usize = 4096;

impl AudioPipe {
    /// Start the audio writer thread. It accepts ffmpeg's connection to the
    /// loopback listener, then drains `rx` into the stream as f32le bytes.
    ///
    /// `lost_samples` counts PCM the capture callback had to discard because
    /// this pipe was backed up. The writer replaces each lost sample with a
    /// sample of silence before writing the next real chunk. That matters now
    /// that timestamps come from the sample count: a gap left unfilled does not
    /// read as a gap, it pulls every later sample earlier, so a single dropout
    /// would desynchronise the rest of the recording. Filling it costs a brief
    /// mute and keeps the timeline exact.
    fn start(
        listener: TcpListener,
        rx: crossbeam_channel::Receiver<PcmChunk>,
        lost_samples: Arc<AtomicU64>,
        label: String,
    ) -> anyhow::Result<Self> {
        let shutting_down = Arc::new(AtomicBool::new(false));
        let frames_written = Arc::new(AtomicU64::new(0));
        let silence_spliced = Arc::new(AtomicU64::new(0));
        let sd = shutting_down.clone();
        let fw = frames_written.clone();
        let spliced = silence_spliced.clone();
        // Non-blocking accept so teardown can interrupt a wait for an ffmpeg that
        // never connects (e.g. it died at startup) instead of a wedged thread.
        listener
            .set_nonblocking(true)
            .map_err(|e| anyhow::anyhow!("Failed to set audio listener non-blocking: {e}"))?;
        let writer_thread = std::thread::Builder::new()
            .name(format!("ffmpeg-audio-{label}"))
            .spawn(move || {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            if sd.load(Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                        Err(e) => {
                            if !sd.load(Ordering::SeqCst) {
                                log::error!("audio TCP accept failed for '{label}': {e}");
                            }
                            return;
                        }
                    }
                };
                // Blocking writes once connected; disable Nagle to minimize latency.
                if let Err(e) = stream.set_nonblocking(false) {
                    log::error!("audio TCP set-blocking failed for '{label}': {e}");
                    return;
                }
                let _ = stream.set_nodelay(true);
                let silence = [0f32; SILENCE_BLOCK];
                loop {
                    match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(chunk) => {
                            // Restore any time the capture callback had to throw
                            // away, before the samples that follow it.
                            let mut lost = lost_samples.swap(0, Ordering::Relaxed);
                            if lost > 0 {
                                spliced.fetch_add(lost, Ordering::Relaxed);
                                log::warn!(
                                    "audio backpressure on '{label}': spliced {lost} samples of \
                                     silence to hold the timeline"
                                );
                            }
                            while lost > 0 {
                                let n = lost.min(SILENCE_BLOCK as u64) as usize;
                                let bytes: &[u8] = bytemuck::cast_slice(&silence[..n]);
                                if stream.write_all(bytes).is_err() {
                                    return;
                                }
                                lost -= n as u64;
                            }

                            let bytes: &[u8] = bytemuck::cast_slice(&chunk.samples);
                            if let Err(e) = stream.write_all(bytes) {
                                if sd.load(Ordering::SeqCst) {
                                    log::debug!(
                                        "audio pipe closed during shutdown for '{label}': {e}"
                                    );
                                } else {
                                    log::error!("audio pipe write error for '{label}': {e}");
                                }
                                return;
                            }
                            fw.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            if sd.load(Ordering::SeqCst) {
                                let _ = stream.flush();
                                return;
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            let _ = stream.flush();
                            return;
                        }
                    }
                }
            })
            .map_err(|e| anyhow::anyhow!("Failed to spawn audio writer thread: {e}"))?;

        Ok(Self {
            shutting_down,
            silence_spliced,
            writer_thread: Some(writer_thread),
            frames_written,
        })
    }

    /// Tear down the writer thread. Idempotent. Setting `shutting_down` unblocks
    /// a pending accept-poll (~20ms) or `recv_timeout` drain (~100ms).
    fn stop(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
    }

    /// PCM chunks written to the socket so far.
    fn frames_written(&self) -> u64 {
        self.frames_written.load(Ordering::Relaxed)
    }

    /// Samples of silence written in place of PCM lost to backpressure.
    fn silence_spliced(&self) -> u64 {
        self.silence_spliced.load(Ordering::Relaxed)
    }
}

impl Drop for AudioPipe {
    fn drop(&mut self) {
        self.stop();
    }
}

impl FfmpegSubprocess {
    /// Start the background writer thread that drains the channel into ffmpeg stdin.
    /// Start the video writer thread.
    ///
    /// The raw video input is declared at a constant `fps`, so ffmpeg times each
    /// frame by its position in the stream: frame N is at N/fps regardless of
    /// when it was produced. That makes a missing frame invisible in the video
    /// but *silent* about time — the recorded timeline simply comes out shorter
    /// than the session was. The audio track is built on the capture device's
    /// sample clock and stays true to real time, so every skipped frame used to
    /// pull the two apart a little more, and the drift accumulated for as long
    /// as the recording ran.
    ///
    /// So when the renderer misses its slot, the writer repeats the previous
    /// frame often enough to cover the gap. Repeating a frame is nearly free to
    /// encode — it differs from its predecessor in nothing — and it keeps the
    /// file constant-frame-rate, which is what editors want. See
    /// /spec/av-sync.md.
    fn start_writer_thread(
        mut stdin: std::process::ChildStdin,
        rx: mpsc::Receiver<Vec<u8>>,
        fps: u32,
        counters: FrameCounters,
        write_failed: Arc<AtomicBool>,
        shutting_down: Arc<AtomicBool>,
        label: String,
    ) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name(format!("ffmpeg-writer-{label}"))
            .spawn(move || {
                let fps = f64::from(fps.max(1));
                // Frames emitted so far, padding included. Counted separately
                // from `frames_written` so the health stat still reports what
                // the renderer actually produced.
                let mut emitted: u64 = 0;
                // Anchored on the *first* frame, not on this thread starting.
                // ffmpeg takes a moment to come up and the caller may not have
                // a frame ready the instant it does; timing from spawn would
                // read that startup as a gap and open every recording with a
                // frozen still.
                let mut started: Option<std::time::Instant> = None;

                for frame in rx {
                    let pad = started.map_or_else(
                        || {
                            started = Some(std::time::Instant::now());
                            0
                        },
                        |t| Self::pad_count(t.elapsed(), fps, emitted),
                    );
                    for _ in 0..pad {
                        if let Err(e) = stdin.write_all(&frame) {
                            Self::report_write_error(&e, &shutting_down, &write_failed, &label);
                            return;
                        }
                        emitted += 1;
                    }
                    if pad > 0 {
                        counters.padded.fetch_add(pad, Ordering::Relaxed);
                    }

                    if let Err(e) = stdin.write_all(&frame) {
                        Self::report_write_error(&e, &shutting_down, &write_failed, &label);
                        return;
                    }
                    emitted += 1;
                    counters.written.fetch_add(1, Ordering::Relaxed);
                }
                // Channel closed — normal shutdown, flush stdin
                let _ = stdin.flush();
            })
            .expect("failed to spawn ffmpeg writer thread")
    }

    /// How many repeated frames to emit before the frame that just arrived.
    ///
    /// `emitted` is everything written so far, padding included. If real time
    /// has moved further than that, the difference is the renderer's shortfall
    /// and repeating the previous frame covers it. Capped at
    /// [`MAX_PAD_FRAMES_PER_ARRIVAL`]: past that the app was not really
    /// recording anyway, and a burst of writes into a pipe that is already
    /// behind would make things worse rather than better.
    fn pad_count(elapsed: std::time::Duration, fps: f64, emitted: u64) -> u64 {
        let due = (elapsed.as_secs_f64() * fps) as u64;
        due.saturating_sub(emitted).min(MAX_PAD_FRAMES_PER_ARRIVAL)
    }

    fn report_write_error(
        e: &std::io::Error,
        shutting_down: &Arc<AtomicBool>,
        write_failed: &Arc<AtomicBool>,
        label: &str,
    ) {
        if shutting_down.load(Ordering::SeqCst) {
            log::debug!("ffmpeg pipe closed during shutdown for '{label}': {e}");
        } else {
            log::error!("ffmpeg write error for '{label}': {e}");
            write_failed.store(true, Ordering::SeqCst);
        }
    }

    /// Spawn an ffmpeg recording subprocess.
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, or if the `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
    #[cfg(test)]
    fn spawn_recording(
        path: &str,
        codec: &RecordingCodec,
        width: u32,
        height: u32,
        fps: u32,
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        Self::spawn_recording_with_presentation(
            path,
            codec,
            width,
            height,
            fps,
            audio,
            PresentationRequest::default(),
            ReadbackFormat::Rgba8,
        )
    }

    /// Resolve recording precision before configuring the headless render target.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured FFmpeg encoder cannot be queried.
    pub fn probe_recording_presentation(
        codec: &RecordingCodec,
        request: PresentationRequest,
        rgba16_unorm_supported: bool,
    ) -> anyhow::Result<ResolvedPresentation> {
        let encoder = recording_encoder(codec);
        let encoder_help = probe_encoder_help(encoder).ok_or_else(|| {
            anyhow::anyhow!("FFmpeg encoder '{encoder}' is not installed or cannot be queried")
        })?;
        Ok(RecordingPlan::resolve(
            codec,
            request,
            preferred_recording_readback(codec, request, rgba16_unorm_supported),
            Some(&encoder_help),
        )
        .resolved)
    }

    /// Spawn a recording after resolving codec, FFmpeg, and typed readback
    /// capabilities as one complete presentation path.
    ///
    /// # Errors
    ///
    /// Returns an error if the audio endpoint or FFmpeg process cannot start.
    ///
    /// # Panics
    ///
    /// Panics if FFmpeg's piped stdin is unavailable or the bounded writer
    /// thread cannot be spawned.
    #[allow(
        clippy::too_many_arguments,
        reason = "the spawn boundary mirrors FFmpeg's fixed recording inputs"
    )]
    pub fn spawn_recording_with_presentation(
        path: &str,
        codec: &RecordingCodec,
        width: u32,
        height: u32,
        fps: u32,
        audio: Option<AudioInput>,
        request: PresentationRequest,
        readback_format: ReadbackFormat,
    ) -> anyhow::Result<Self> {
        let encoder = recording_encoder(codec);
        let encoder_help = probe_encoder_help(encoder).ok_or_else(|| {
            anyhow::anyhow!("FFmpeg encoder '{encoder}' is not installed or cannot be queried")
        })?;
        let plan = RecordingPlan::resolve(codec, request, readback_format, Some(&encoder_help));
        // Recording keeps the device's native sample rate (Decision 5).
        let prepared = prepare_audio(audio, false)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", plan.input_pixel_format])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(a_in);
        if plan.unpremultiply {
            cmd.args(["-vf", unpremultiply_filter(plan.input_pixel_format)]);
        }
        cmd.args(&plan.video_args);
        cmd.args(a_out)
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg: {e}. Is ffmpeg installed?"))?;

        log::info!(
            "Recording started: {path} ({codec}, {width}x{height} @ {fps}fps, {} -> {})",
            plan.input_pixel_format,
            plan.output_pixel_format
        );

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            path.to_string(),
        );
        let audio = finalize_audio(prepared, path.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
            write_failed,
            shutting_down,
            label: path.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: true,
            frame_contract: Some(FrameContract {
                format: plan.expected_readback,
                width,
                height,
            }),
            presentation: Some(plan.resolved),
        })
    }

    /// Spawn an ffmpeg SRT streaming subprocess in listener (server) mode.
    /// Starts an SRT server on the specified port and broadcasts frames to connected clients.
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, or if the `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
    pub fn spawn_srt(
        url: &str,
        codec: &super::context::SrtCodec,
        request: PresentationRequest,
        width: u32,
        height: u32,
        fps: u32,
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Streaming target: normalize audio to 48k (Decision 5).
        let prepared = prepare_audio(audio, true)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        // Ensure listener mode so ffmpeg acts as an SRT server
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{url}&mode=listener")
        } else {
            format!("{url}?mode=listener")
        };

        let configured_codec = match codec {
            super::context::SrtCodec::H264 => StreamingCodec::H264,
            super::context::SrtCodec::H265 => StreamingCodec::H265,
        };
        let plan = StreamingPlan::resolve(
            StreamingProtocol::Srt,
            configured_codec,
            request,
            StreamingCapabilities::installed(),
        );

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", plan.input_pixel_format])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(a_in)
            .args(&plan.video_args)
            .args(a_out)
            .args(&plan.muxer_args)
            .arg(&srt_url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!("Failed to spawn ffmpeg for SRT: {e}. Is ffmpeg installed?")
        })?;

        log::info!("SRT server started: {srt_url} ({width}x{height} @ {fps}fps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            url.to_string(),
        );
        let audio = finalize_audio(prepared, url.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
            write_failed,
            shutting_down,
            label: url.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: false,
            frame_contract: Some(FrameContract {
                format: plan.expected_readback,
                width,
                height,
            }),
            presentation: Some(plan.resolved),
        })
    }

    /// Spawn an ffmpeg HLS output subprocess.
    /// Writes HLS segments to `.varda/streams/<name>/` with `-hls_list_size 0` for VOD archive.
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, if the stream output directory cannot be created, or if the
    /// `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
    // Arguments mirror the persisted HLS target plus frame/audio transport.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_hls(
        name: &str,
        codec: &super::context::StreamingCodec,
        request: PresentationRequest,
        width: u32,
        height: u32,
        fps: u32,
        low_latency: bool,
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Streaming target: normalize audio to 48k (Decision 5).
        let prepared = prepare_audio(audio, true)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        let dir = format!(".varda/streams/{name}");
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create HLS output dir '{dir}': {e}"))?;
        let playlist = format!("{dir}/index.m3u8");
        write_stream_player(&dir, "hls", "index.m3u8", low_latency);

        let plan = StreamingPlan::resolve(
            StreamingProtocol::Hls,
            codec.clone(),
            request,
            StreamingCapabilities::installed(),
        );

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", plan.input_pixel_format])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(a_in)
            .args(&plan.video_args)
            .args(a_out)
            .args(&plan.muxer_args);

        cmd.args(hls_segment_args(
            &dir,
            low_latency,
            plan.resolved.resolved == PresentationDepth::Sdr10
                || plan.effective_codec != StreamingCodec::H264,
        ));

        cmd.arg(&playlist)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!("Failed to spawn ffmpeg for HLS: {e}. Is ffmpeg installed?")
        })?;

        let mode = if low_latency { "LL-HLS" } else { "HLS" };
        log::info!("{mode} output started: {playlist} ({width}x{height} @ {fps}fps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            name.to_string(),
        );
        let audio = finalize_audio(prepared, name.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
            write_failed,
            shutting_down,
            label: name.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: false,
            frame_contract: Some(FrameContract {
                format: plan.expected_readback,
                width,
                height,
            }),
            presentation: Some(plan.resolved),
        })
    }

    /// Spawn an ffmpeg RTMP output subprocess.
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, or if the `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
    // Arguments mirror the persisted RTMP target plus frame/audio transport.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_rtmp(
        url: &str,
        codec: &super::context::StreamingCodec,
        codec_contract: RtmpCodecContract,
        request: PresentationRequest,
        width: u32,
        height: u32,
        fps: u32,
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Streaming target: normalize audio to 48k (Decision 5).
        let prepared = prepare_audio(audio, true)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        let plan = StreamingPlan::resolve(
            StreamingProtocol::Rtmp(codec_contract),
            codec.clone(),
            request,
            StreamingCapabilities::installed(),
        );

        let (maxrate, bufsize) = compute_rtmp_bitrate(width, height, fps);
        let gop = fps * 2;

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", plan.input_pixel_format])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(a_in)
            .args(&plan.video_args)
            .args(["-b:v", &format!("{maxrate}k")])
            .args(["-maxrate", &format!("{maxrate}k")])
            .args(["-bufsize", &format!("{bufsize}k")])
            .args(["-g", &gop.to_string()])
            .args(a_out)
            .args(&plan.muxer_args)
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!("Failed to spawn ffmpeg for RTMP: {e}. Is ffmpeg installed?")
        })?;

        log::info!("RTMP output started: {url} ({width}x{height} @ {fps}fps, {maxrate}kbps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let label = url.to_string();
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            label.clone(),
        );
        let audio = finalize_audio(prepared, label.clone())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
            write_failed,
            shutting_down,
            label,
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: false,
            frame_contract: Some(FrameContract {
                format: plan.expected_readback,
                width,
                height,
            }),
            presentation: Some(plan.resolved),
        })
    }

    /// Spawn an ffmpeg DASH output subprocess.
    /// Writes DASH segments to `.varda/streams/<name>/` with `-window_size 0` for VOD archive.
    ///
    /// # Errors
    ///
    /// Returns an error if the audio passthrough endpoint cannot be prepared or
    /// started, if the stream output directory cannot be created, or if the
    /// `ffmpeg` binary cannot be spawned (not installed).
    ///
    /// # Panics
    ///
    /// Panics if ffmpeg's stdin was not piped, or if the writer thread cannot
    /// be spawned — both indicate the process/thread limits are exhausted.
    pub fn spawn_dash(
        name: &str,
        codec: &super::context::StreamingCodec,
        request: PresentationRequest,
        width: u32,
        height: u32,
        fps: u32,
        audio: Option<AudioInput>,
    ) -> anyhow::Result<Self> {
        // Streaming target: normalize audio to 48k (Decision 5).
        let prepared = prepare_audio(audio, true)?;
        let empty: Vec<String> = Vec::new();
        let (a_in, a_out) = match &prepared {
            Some(p) => (&p.in_args, &p.out_args),
            None => (&empty, &empty),
        };
        let dir = format!(".varda/streams/{name}");
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("Failed to create DASH output dir '{dir}': {e}"))?;
        let manifest = format!("{dir}/manifest.mpd");
        write_stream_player(&dir, "dash", "manifest.mpd", false);

        let plan = StreamingPlan::resolve(
            StreamingProtocol::Dash,
            codec.clone(),
            request,
            StreamingCapabilities::installed(),
        );

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pix_fmt", plan.input_pixel_format])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(a_in)
            .args(&plan.video_args)
            .args(a_out)
            .args(&plan.muxer_args)
            .args(["-seg_duration", "2"])
            .args(["-window_size", "30"])
            .args(["-extra_window_size", "5"])
            .arg(&manifest)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!("Failed to spawn ffmpeg for DASH: {e}. Is ffmpeg installed?")
        })?;

        log::info!("DASH output started: {manifest} ({width}x{height} @ {fps}fps)");

        let stdin = child.stdin.take().expect("ffmpeg stdin not piped");
        let counters = FrameCounters::new();
        let write_failed = Arc::new(AtomicBool::new(false));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(FRAME_CHANNEL_CAPACITY);
        let writer_thread = Self::start_writer_thread(
            stdin,
            rx,
            fps,
            counters.clone(),
            write_failed.clone(),
            shutting_down.clone(),
            name.to_string(),
        );
        let audio = finalize_audio(prepared, name.to_string())?;

        Ok(Self {
            child,
            frame_tx: Some(tx),
            writer_thread: Some(writer_thread),
            counters,
            write_failed,
            shutting_down,
            label: name.to_string(),
            start_time: std::time::Instant::now(),
            stopped: false,
            audio,
            graceful_shutdown: false,
            frame_contract: Some(FrameContract {
                format: plan.expected_readback,
                width,
                height,
            }),
            presentation: Some(plan.resolved),
        })
    }

    /// Presentation selected for this FFmpeg output.
    pub fn presentation(&self) -> Option<&ResolvedPresentation> {
        self.presentation.as_ref()
    }

    /// Feed one typed GPU readback frame to an FFmpeg subprocess.
    ///
    /// The format, dimensions, and stride must match the contract negotiated
    /// before FFmpeg was spawned. A mismatch stops bytes from entering the raw
    /// video pipe, where they would otherwise silently desynchronize frames.
    pub fn feed_readback_frame(&mut self, frame: &ReadbackFrame) -> bool {
        let Some(contract) = self.frame_contract else {
            log::error!(
                "typed frame sent without an FFmpeg contract for '{}'",
                self.label
            );
            return false;
        };
        if frame.format() != contract.format
            || frame.width() != contract.width
            || frame.height() != contract.height
        {
            log::error!(
                "FFmpeg frame contract mismatch for '{}': expected {} {}x{}, got {} {}x{}",
                self.label,
                readback_label(contract.format),
                contract.width,
                contract.height,
                readback_label(frame.format()),
                frame.width(),
                frame.height()
            );
            return false;
        }
        let bytes_per_pixel = match contract.format {
            ReadbackFormat::Rgba8
            | ReadbackFormat::Bgra8
            | ReadbackFormat::Rgb10A2
            | ReadbackFormat::P216 => 4,
            ReadbackFormat::Rgba16Float | ReadbackFormat::Rgba16Unorm => 8,
            ReadbackFormat::Uyvy => 2,
        };
        let expected_stride = contract.width * bytes_per_pixel;
        if frame.stride() != expected_stride {
            log::error!(
                "FFmpeg frame stride mismatch for '{}': expected {}, got {}",
                self.label,
                expected_stride,
                frame.stride()
            );
            return false;
        }
        self.feed_frame(frame.bytes())
    }

    /// Feed a frame of raw data to a byte-oriented streaming subprocess.
    /// Never blocks — drops the frame if the writer thread can't keep up.
    /// Returns false if the subprocess has failed (write error or process exited).
    pub fn feed_frame(&mut self, rgba: &[u8]) -> bool {
        // Check if writer thread reported an error
        if self.write_failed.load(Ordering::SeqCst) {
            self.drain_stderr();
            return false;
        }
        // Check if ffmpeg already exited (non-blocking)
        if let Some(status) = self.child.try_wait().ok().flatten() {
            if !status.success() {
                self.drain_stderr();
                log::error!(
                    "ffmpeg exited with status {} for '{}' before frame could be written",
                    status,
                    self.label
                );
            }
            return false;
        }
        if let Some(ref tx) = self.frame_tx {
            if try_enqueue_frame(tx, &self.counters.dropped, &self.label, rgba) {
                true
            } else {
                self.drain_stderr();
                false
            }
        } else {
            false
        }
    }

    /// Read and log any stderr output from ffmpeg.
    /// Each line is classified individually: lines containing error indicators
    /// are logged at ERROR, everything else (version info, codec config) at DEBUG.
    fn drain_stderr(&mut self) {
        if let Some(mut stderr) = self.child.stderr.take() {
            Self::drain_stderr_pipe(&mut stderr, &self.label);
        }
    }

    /// Static helper: drain an ffmpeg stderr pipe and log each line.
    fn drain_stderr_pipe(stderr: &mut std::process::ChildStderr, label: &str) {
        use std::io::Read;
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf);
        if !buf.is_empty() {
            for line in buf.lines().take(30) {
                let lower = line.to_ascii_lowercase();
                if lower.contains("error")
                    || lower.contains("failed")
                    || lower.contains("invalid")
                    || lower.contains("fatal")
                {
                    log::error!("ffmpeg [{label}]: {line}");
                } else {
                    log::debug!("ffmpeg [{label}]: {line}");
                }
            }
        }
    }

    /// Stop the subprocess. For recordings (`graceful_shutdown`), the heavy
    /// work (joining threads, waiting for ffmpeg to write the moov atom) runs
    /// on a detached background thread so the caller (UI / main thread) returns
    /// immediately. For streams, kills ffmpeg inline (fast).
    /// Idempotent — safe to call multiple times.
    ///
    /// # Panics
    ///
    /// On the recording path, panics if the placeholder child process or the
    /// background finalize thread cannot be spawned.
    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;

        let duration = self.start_time.elapsed();

        // 1. Signal shutdown so writer threads know a broken pipe is expected
        self.shutting_down.store(true, Ordering::SeqCst);

        // 2. Drop the sender to close the channel — no more frames queued
        drop(self.frame_tx.take());

        if self.graceful_shutdown {
            // --- Recording path: finalize on a background thread ---
            // Move all owned resources out of `self` so the thread owns them.
            let mut audio = self.audio.take();
            let writer_thread = self.writer_thread.take();
            let mut child = std::mem::replace(
                &mut self.child,
                // Placeholder — never used again (stopped == true).
                Command::new("true")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("failed to spawn placeholder"),
            );
            let label = self.label.clone();
            let counters = self.counters.clone();
            let stderr = child.stderr.take();

            std::thread::Builder::new()
                .name(format!("ffmpeg-finalize-{label}"))
                .spawn(move || {
                    const FINALIZE_TIMEOUT: std::time::Duration =
                        std::time::Duration::from_secs(30);

                    // 3a. Tear down the audio writer/socket so ffmpeg sees EOF on
                    //     both inputs.
                    if let Some(ref mut a) = audio {
                        a.stop();
                    }

                    // 3b. Join the video writer thread — drains remaining ≤2
                    //     frames, flushes & drops stdin → ffmpeg sees video EOF.
                    if let Some(handle) = writer_thread {
                        let _ = handle.join();
                    }

                    // 4. Wait for ffmpeg to finalize the container (moov atom).
                    let deadline = std::time::Instant::now() + FINALIZE_TIMEOUT;
                    loop {
                        match child.try_wait() {
                            Ok(Some(_status)) => break,
                            Ok(None) => {
                                if std::time::Instant::now() >= deadline {
                                    log::warn!(
                                        "ffmpeg did not exit within {}s for '{}', killing",
                                        FINALIZE_TIMEOUT.as_secs(),
                                        label
                                    );
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    break;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                            Err(e) => {
                                log::error!("Failed to wait for ffmpeg '{label}': {e}");
                                break;
                            }
                        }
                    }

                    // 5. Log completion
                    if let Some(mut pipe) = stderr {
                        Self::drain_stderr_pipe(&mut pipe, &label);
                    }
                    let frames = counters.written.load(Ordering::Relaxed);
                    log::info!(
                        "ffmpeg finished: {} ({} frames{}, {:.1}s)",
                        label,
                        frames,
                        Self::pad_summary(counters.padded.load(Ordering::Relaxed)),
                        duration.as_secs_f32()
                    );
                })
                .expect("failed to spawn ffmpeg finalize thread");
        } else {
            // --- Streaming path: kill immediately (inline, fast) ---

            // 3. Kill ffmpeg BEFORE joining the writer thread. The writer
            //    thread may be blocked on stdin.write_all() (e.g. SRT listener
            //    with a full pipe buffer). Killing the child breaks the pipe,
            //    which unblocks the write and lets the thread exit.
            let _ = self.child.kill();

            // 3b. Tear down the audio side-channel (socket + writer thread).
            //     Done after the kill so a writer blocked on a full socket sees
            //     a broken pipe.
            if let Some(audio) = self.audio.as_mut() {
                audio.stop();
            }

            // 4. Now safe to join — the writer thread will see a broken pipe
            //    or a closed channel and exit promptly.
            if let Some(handle) = self.writer_thread.take() {
                let _ = handle.join();
            }

            // 5. Reap the child process
            let _ = self.child.wait();

            let frames = self.counters.written.load(Ordering::Relaxed);
            let padded = self.counters.padded.load(Ordering::Relaxed);
            self.drain_stderr();
            log::info!(
                "ffmpeg finished: {} ({} frames{}, {:.1}s)",
                self.label,
                frames,
                Self::pad_summary(padded),
                duration.as_secs_f32()
            );
        }
    }

    /// Completion-log fragment naming repeated frames, empty when there were
    /// none. Padding is not an error — it is how a hitchy session still comes
    /// out in sync — but it is worth knowing the renderer struggled.
    fn pad_summary(padded: u64) -> String {
        if padded == 0 {
            String::new()
        } else {
            format!(" + {padded} repeated to cover renderer gaps")
        }
    }

    /// Duration since the subprocess was started.
    pub fn duration(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Number of frames written so far.
    pub fn frames_written(&self) -> u64 {
        self.counters.written.load(Ordering::Relaxed)
    }

    /// Number of audio PCM chunks written to the socket so far, or `None` for a
    /// video-only output (no audio passthrough).
    pub fn audio_frames_written(&self) -> Option<u64> {
        self.audio.as_ref().map(AudioPipe::frames_written)
    }

    /// Repeated frames emitted to cover gaps where the renderer missed its
    /// slot. Nonzero means the session dropped frames; the recording is still
    /// in sync, but the visible result is a brief freeze.
    pub fn frames_padded(&self) -> u64 {
        self.counters.padded.load(Ordering::Relaxed)
    }

    /// Frames offered while the writer channel was full. The take continued;
    /// those pixels never reached ffmpeg.
    pub fn frames_dropped(&self) -> u64 {
        self.counters.dropped.load(Ordering::Relaxed)
    }

    /// Samples of silence spliced into the audio to replace PCM lost to
    /// backpressure. Nonzero means audio was audibly interrupted, as opposed to
    /// merely delayed. `None` for a video-only output.
    pub fn audio_silence_spliced(&self) -> Option<u64> {
        self.audio.as_ref().map(AudioPipe::silence_spliced)
    }

    /// The label (path or URL) for this subprocess.
    pub fn label(&self) -> &str {
        &self.label
    }
}

impl Drop for FfmpegSubprocess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::value::render::{PresentationDepth, PresentationRequest};
    use crate::renderer::ReadbackFormat;

    /// Check if ffmpeg is available on this system.
    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg").arg("-version").output().is_ok()
    }

    // ── SRT URL mode injection tests (pure logic) ──────────────────

    #[test]
    fn spawn_srt_url_adds_listener_mode() {
        // Verify the URL mode injection logic without spawning
        let url = "srt://127.0.0.1:9001";
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{url}&mode=listener")
        } else {
            format!("{url}?mode=listener")
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?mode=listener");
    }

    #[test]
    fn spawn_srt_url_preserves_existing_mode() {
        let url = "srt://127.0.0.1:9001?mode=caller";
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{url}&mode=listener")
        } else {
            format!("{url}?mode=listener")
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?mode=caller");
    }

    #[test]
    fn spawn_srt_url_appends_to_existing_params() {
        let url = "srt://127.0.0.1:9001?latency=0";
        let srt_url = if url.contains("mode=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{url}&mode=listener")
        } else {
            format!("{url}?mode=listener")
        };
        assert_eq!(srt_url, "srt://127.0.0.1:9001?latency=0&mode=listener");
    }

    // ── Recording codec display ────────────────────────────────────

    #[test]
    fn recording_codec_display() {
        use crate::renderer::context::SrtCodec;

        assert_eq!(format!("{}", RecordingCodec::H264), "H.264");
        assert_eq!(format!("{}", RecordingCodec::H265), "H.265 (HEVC)");
        assert_eq!(format!("{}", RecordingCodec::AV1), "AV1");
        assert_eq!(format!("{}", RecordingCodec::ProRes), "ProRes 422");
        assert_eq!(format!("{}", RecordingCodec::ProRes4444), "ProRes 4444");
        assert_eq!(format!("{}", RecordingCodec::Hap), "HAP");
        assert_eq!(format!("{}", RecordingCodec::HapAlpha), "HAP Alpha");
        assert_eq!(format!("{}", RecordingCodec::HapQ), "HAP Q");

        // SrtCodec display
        assert_eq!(format!("{}", SrtCodec::H264), "H.264");
        assert_eq!(format!("{}", SrtCodec::H265), "H.265 (HEVC)");
    }

    fn ten_bit_request() -> PresentationRequest {
        PresentationRequest {
            depth: PresentationDepth::Sdr10,
            dither: true,
        }
    }

    #[test]
    fn hap_family_ten_bit_request_falls_back_by_product_contract() {
        for codec in [
            RecordingCodec::Hap,
            RecordingCodec::HapAlpha,
            RecordingCodec::HapQ,
        ] {
            let plan = RecordingPlan::resolve(
                &codec,
                ten_bit_request(),
                ReadbackFormat::Rgb10A2,
                Some("Supported pixel formats: rgba"),
            );
            assert_eq!(plan.resolved.requested, PresentationDepth::Sdr10, "{codec}");
            assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8, "{codec}");
            assert_eq!(plan.input_pixel_format, "rgba", "{codec}");
            assert!(
                plan.resolved
                    .fallback_reason
                    .as_deref()
                    .unwrap()
                    .contains("HAP"),
                "{codec}: {:?}",
                plan.resolved.fallback_reason
            );
        }
    }

    #[test]
    fn h264_ten_bit_request_falls_back_by_product_contract() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::H264,
            ten_bit_request(),
            ReadbackFormat::Rgb10A2,
            Some("Supported pixel formats: yuv420p yuv420p10le"),
        );

        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(plan.input_pixel_format, "rgba");
        assert_eq!(plan.output_pixel_format, "yuv420p");
        assert!(
            plan.resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("H.264")
        );
    }

    #[test]
    fn hevc_main10_uses_packed_rgb10_and_explicit_rec709_metadata() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::H265,
            ten_bit_request(),
            ReadbackFormat::Rgb10A2,
            Some("Supported pixel formats: yuv420p yuv420p10le yuv422p10le"),
        );

        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr10);
        assert_eq!(plan.input_pixel_format, "x2bgr10le");
        assert_eq!(plan.output_pixel_format, "yuv420p10le");
        assert!(has_pair(&plan.video_args, "-profile:v", "main10"));
        assert!(has_pair(&plan.video_args, "-color_primaries", "bt709"));
        assert!(has_pair(&plan.video_args, "-color_trc", "bt709"));
        assert!(has_pair(&plan.video_args, "-colorspace", "bt709"));
        assert!(has_pair(&plan.video_args, "-color_range", "tv"));
    }

    #[test]
    fn ten_bit_codec_falls_back_when_renderer_only_supplies_rgba8() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::ProRes,
            ten_bit_request(),
            ReadbackFormat::Rgba8,
            Some("Supported pixel formats: yuv422p10le"),
        );

        assert_eq!(plan.resolved.requested, PresentationDepth::Sdr10);
        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(plan.input_pixel_format, "rgba");
        assert_eq!(
            plan.resolved.fallback_reason.as_deref(),
            Some(
                "10-bit SDR recording requires packed RGB10 readback, but the active renderer \
                 supplies RGBA8"
            )
        );
    }

    #[test]
    fn ten_bit_codec_falls_back_when_encoder_lacks_required_pixel_format() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::AV1,
            ten_bit_request(),
            ReadbackFormat::Rgb10A2,
            Some("Supported pixel formats: yuv420p"),
        );

        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert!(
            plan.resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("does not support yuv420p10le")
        );
    }

    #[test]
    fn every_recording_codec_pins_its_eight_bit_contract() {
        let cases = [
            (RecordingCodec::H264, "yuv420p"),
            (RecordingCodec::H265, "yuv420p"),
            (RecordingCodec::AV1, "yuv420p"),
            (RecordingCodec::ProRes, "yuv422p10le"),
            (RecordingCodec::ProRes4444, "yuva444p10le"),
            (RecordingCodec::Hap, "rgba"),
            (RecordingCodec::HapAlpha, "rgba"),
            (RecordingCodec::HapQ, "rgba"),
        ];
        for (codec, expected_output) in cases {
            let plan = RecordingPlan::resolve(
                &codec,
                PresentationRequest::default(),
                ReadbackFormat::Rgba8,
                Some(&format!("Supported pixel formats: {expected_output}")),
            );
            assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8, "{codec}");
            assert_eq!(plan.input_pixel_format, "rgba", "{codec}");
            assert_eq!(plan.output_pixel_format, expected_output, "{codec}");
            assert!(has_pair(&plan.video_args, "-pix_fmt", expected_output));
            assert!(has_pair(&plan.video_args, "-color_primaries", "bt709"));
            assert!(has_pair(&plan.video_args, "-color_trc", "bt709"));
            assert!(has_pair(&plan.video_args, "-colorspace", "bt709"));
            assert!(has_pair(&plan.video_args, "-color_range", "tv"));
        }
    }

    #[test]
    fn supported_opaque_ten_bit_codecs_pin_profiles_and_formats() {
        let cases = [
            (RecordingCodec::H265, "yuv420p10le", "main10"),
            (RecordingCodec::AV1, "yuv420p10le", "0"),
            (RecordingCodec::ProRes, "yuv422p10le", "2"),
        ];
        for (codec, output, profile) in cases {
            let plan = RecordingPlan::resolve(
                &codec,
                ten_bit_request(),
                ReadbackFormat::Rgb10A2,
                Some(&format!("Supported pixel formats: {output}")),
            );
            assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr10, "{codec}");
            assert_eq!(plan.input_pixel_format, "x2bgr10le", "{codec}");
            assert_eq!(plan.output_pixel_format, output, "{codec}");
            assert!(has_pair(&plan.video_args, "-profile:v", profile));
        }
    }

    #[test]
    fn missing_encoder_probe_never_claims_ten_bit() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::H265,
            ten_bit_request(),
            ReadbackFormat::Rgb10A2,
            None,
        );
        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert!(
            plan.resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("unavailable")
        );
    }

    #[test]
    fn prores_4444_uses_integer_rgba16_and_explicit_alpha_contract() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::ProRes4444,
            ten_bit_request(),
            ReadbackFormat::Rgba16Unorm,
            Some("Supported pixel formats: yuva444p10le"),
        );

        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr10);
        assert_eq!(plan.resolved.pixel_format, PresentationPixelFormat::Rgba16);
        assert_eq!(plan.input_pixel_format, "rgba64le");
        assert_eq!(plan.output_pixel_format, "yuva444p10le");
        assert_eq!(plan.expected_readback, ReadbackFormat::Rgba16Unorm);
        assert!(plan.unpremultiply);
        assert!(has_pair(&plan.video_args, "-profile:v", "4"));
        assert!(has_pair(&plan.video_args, "-pix_fmt", "yuva444p10le"));
        assert!(has_pair(&plan.video_args, "-alpha_bits", "16"));
        assert_eq!(
            unpremultiply_filter(plan.input_pixel_format),
            "setparams=alpha_mode=premultiplied,format=gbrap16le,unpremultiply=inplace=1"
        );
    }

    #[test]
    fn prores_4444_rejects_half_float_bytes_as_rgba64le() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::ProRes4444,
            ten_bit_request(),
            ReadbackFormat::Rgba16Float,
            Some("Supported pixel formats: yuva444p10le"),
        );

        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert!(
            plan.resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("integer RGBA16")
        );
    }

    #[test]
    fn prores_4444_falls_back_when_gpu_lacks_rgba16_unorm() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::ProRes4444,
            ten_bit_request(),
            ReadbackFormat::Rgba8,
            Some("Supported pixel formats: yuva444p10le"),
        );

        assert_eq!(plan.resolved.requested, PresentationDepth::Sdr10);
        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert!(
            plan.resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("GPU does not support")
        );
    }

    #[test]
    fn prores_4444_falls_back_when_encoder_lacks_yuva444p10le() {
        let plan = RecordingPlan::resolve(
            &RecordingCodec::ProRes4444,
            ten_bit_request(),
            ReadbackFormat::Rgba16Unorm,
            Some("Supported pixel formats: yuv422p10le"),
        );

        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert!(
            plan.resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("does not support yuva444p10le")
        );
    }

    #[test]
    fn prores_4444_preserves_known_rgb_and_alpha_ramp() {
        const WIDTH: u32 = 16;
        const HEIGHT: u32 = 16;

        let Some(help) = probe_encoder_help("prores_ks") else {
            eprintln!("Skipping test: prores_ks encoder unavailable");
            return;
        };
        if !encoder_supports_pixel_format(&help, "yuva444p10le") {
            eprintln!("Skipping test: prores_ks lacks yuva444p10le");
            return;
        }

        let mut input = Vec::with_capacity((WIDTH * HEIGHT * 8) as usize);
        for _y in 0..HEIGHT {
            for x in 0..WIDTH {
                let alpha = x as f32 / (WIDTH - 1) as f32;
                for straight in [0.75_f32, 0.5, 0.25] {
                    input.extend_from_slice(
                        &((straight * alpha * f32::from(u16::MAX)).round() as u16).to_le_bytes(),
                    );
                }
                input.extend_from_slice(
                    &((alpha * f32::from(u16::MAX)).round() as u16).to_le_bytes(),
                );
            }
        }

        let path = std::env::temp_dir().join(format!(
            "varda-prores-alpha-{}-{}.mov",
            std::process::id(),
            crate::deck::generate_short_uuid()
        ));
        let mut encoder = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "rawvideo"])
            .args(["-pix_fmt", "rgba64le", "-s", "16x16", "-i", "-"])
            .args(["-vf", unpremultiply_filter("rgba64le")])
            .args(["-frames:v", "1", "-c:v", "prores_ks"])
            .args(["-profile:v", "4", "-alpha_bits", "16"])
            .args(["-pix_fmt", "yuva444p10le"])
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn prores alpha encode");
        encoder
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(&input)
            .expect("write RGBA16 frame");
        let encode_output = encoder.wait_with_output().expect("finish prores encode");
        assert!(
            encode_output.status.success(),
            "{}",
            String::from_utf8_lossy(&encode_output.stderr)
        );

        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba64le",
                "-",
            ])
            .output()
            .expect("decode prores alpha frame");
        let _ = std::fs::remove_file(path);
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        let words: Vec<u16> = decoded
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| u16::from_le_bytes(*bytes))
            .collect();
        assert_eq!(words.len(), (WIDTH * HEIGHT * 4) as usize);

        let samples: Vec<[u16; 4]> = [5_usize, 10, 15]
            .into_iter()
            .map(|x| {
                let offset = x * 4;
                words[offset..offset + 4].try_into().unwrap()
            })
            .collect();
        for [red, green, blue, _alpha] in &samples {
            assert!(red.abs_diff(49_151) < 256, "red code {red}");
            assert!(green.abs_diff(32_768) < 256, "green code {green}");
            assert!(blue.abs_diff(16_384) < 256, "blue code {blue}");
        }
        assert!(samples[0][3] < samples[1][3] && samples[1][3] < samples[2][3]);
        assert!(samples[2][3].abs_diff(u16::MAX) < 32);
    }

    #[test]
    fn wgpu_rgb10a2_channel_order_maps_to_ffmpeg_x2bgr10le() {
        // wgpu packs a red-only texel as 0x0000_03ff. FFmpeg's
        // x2bgr10le layout assigns those low ten bits to R; x2rgb10le would
        // decode the same word as blue.
        let red_only_wgpu_word = 0x0000_03ff_u32;
        assert_eq!(red_only_wgpu_word & 0x3ff, 1023);
        assert_eq!(packed_rgb10_input_format(), "x2bgr10le");
    }

    #[test]
    fn recording_preflight_selects_renderer_storage_before_spawn() {
        assert_eq!(
            preferred_recording_readback(&RecordingCodec::H265, ten_bit_request(), true),
            ReadbackFormat::Rgb10A2
        );
        assert_eq!(
            preferred_recording_readback(
                &RecordingCodec::H264,
                PresentationRequest::default(),
                true
            ),
            ReadbackFormat::Rgba8
        );
        assert_eq!(
            preferred_recording_readback(&RecordingCodec::ProRes4444, ten_bit_request(), true),
            ReadbackFormat::Rgba16Unorm
        );
        assert_eq!(
            preferred_recording_readback(&RecordingCodec::ProRes4444, ten_bit_request(), false),
            ReadbackFormat::Rgba8
        );
    }

    fn all_streaming_capabilities() -> StreamingCapabilities {
        StreamingCapabilities {
            hevc_main10: true,
            av1_10: true,
            enhanced_hevc: true,
            enhanced_av1: true,
        }
    }

    #[test]
    fn srt_hevc_main10_plan_pins_mpegts_and_rec709() {
        let plan = StreamingPlan::resolve(
            StreamingProtocol::Srt,
            StreamingCodec::H265,
            ten_bit_request(),
            all_streaming_capabilities(),
        );
        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr10);
        assert_eq!(plan.input_pixel_format, "x2bgr10le");
        assert_eq!(plan.expected_readback, ReadbackFormat::Rgb10A2);
        assert!(has_pair(&plan.video_args, "-profile:v", "main10"));
        assert!(has_pair(&plan.video_args, "-pix_fmt", "yuv420p10le"));
        assert!(has_pair(&plan.video_args, "-color_primaries", "bt709"));
        assert!(has_pair(&plan.video_args, "-color_trc", "bt709"));
        assert!(has_pair(&plan.video_args, "-colorspace", "bt709"));
        assert!(has_pair(&plan.video_args, "-color_range", "tv"));
        assert!(has_pair(&plan.muxer_args, "-f", "mpegts"));
    }

    #[test]
    fn hls_and_dash_ten_bit_plans_pin_isobmff_codec_tags() {
        let hls = StreamingPlan::resolve(
            StreamingProtocol::Hls,
            StreamingCodec::H265,
            ten_bit_request(),
            all_streaming_capabilities(),
        );
        assert_eq!(hls.resolved.resolved, PresentationDepth::Sdr10);
        assert!(has_pair(&hls.muxer_args, "-tag:v", "hvc1"));
        assert!(has_pair(&hls.muxer_args, "-f", "hls"));
        let hls_segments = hls_segment_args("stream", false, true);
        assert!(has_pair(&hls_segments, "-hls_segment_type", "fmp4"));
        assert!(has_pair(
            &hls_segments,
            "-hls_segment_filename",
            "stream/seg_%05d.m4s"
        ));

        let dash = StreamingPlan::resolve(
            StreamingProtocol::Dash,
            StreamingCodec::AV1,
            ten_bit_request(),
            all_streaming_capabilities(),
        );
        assert_eq!(dash.resolved.resolved, PresentationDepth::Sdr10);
        assert!(has_pair(&dash.muxer_args, "-tag:v", "av01"));
        assert!(has_pair(&dash.muxer_args, "-f", "dash"));
    }

    #[test]
    fn legacy_rtmp_forces_truthful_h264_eight_bit_fallback() {
        let plan = StreamingPlan::resolve(
            StreamingProtocol::Rtmp(RtmpCodecContract::Legacy),
            StreamingCodec::H265,
            ten_bit_request(),
            all_streaming_capabilities(),
        );
        assert_eq!(plan.resolved.requested, PresentationDepth::Sdr10);
        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(plan.expected_readback, ReadbackFormat::Rgba8);
        assert!(
            plan.resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("legacy RTMP")
        );
        assert!(has_pair(&plan.video_args, "-c:v", "libx264"));
        assert!(has_pair(&plan.video_args, "-pix_fmt", "yuv420p"));
        assert!(has_pair(&plan.muxer_args, "-f", "flv"));
    }

    #[test]
    fn enhanced_rtmp_requires_ffmpeg_muxer_capability() {
        let missing = StreamingCapabilities {
            hevc_main10: true,
            enhanced_hevc: false,
            ..all_streaming_capabilities()
        };
        let fallback = StreamingPlan::resolve(
            StreamingProtocol::Rtmp(RtmpCodecContract::Enhanced),
            StreamingCodec::H265,
            ten_bit_request(),
            missing,
        );
        assert_eq!(fallback.resolved.resolved, PresentationDepth::Sdr8);
        assert!(
            fallback
                .resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("Enhanced RTMP")
        );

        let supported = StreamingPlan::resolve(
            StreamingProtocol::Rtmp(RtmpCodecContract::Enhanced),
            StreamingCodec::AV1,
            ten_bit_request(),
            all_streaming_capabilities(),
        );
        assert_eq!(supported.resolved.resolved, PresentationDepth::Sdr10);
        assert!(has_pair(&supported.video_args, "-c:v", "libsvtav1"));
        assert!(has_pair(
            &supported.muxer_args,
            "-flvflags",
            "no_sequence_end+no_metadata"
        ));
    }

    #[test]
    fn ten_bit_stream_falls_back_when_encoder_lacks_profile() {
        let plan = StreamingPlan::resolve(
            StreamingProtocol::Dash,
            StreamingCodec::AV1,
            ten_bit_request(),
            StreamingCapabilities::default(),
        );
        assert_eq!(plan.resolved.resolved, PresentationDepth::Sdr8);
        assert!(
            plan.resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("AV1 10-bit")
        );
    }

    #[test]
    fn streaming_resolver_covers_protocol_codec_contract_matrix() {
        let cases = [
            (
                StreamingProtocol::Srt,
                StreamingCodec::H264,
                PresentationDepth::Sdr8,
            ),
            (
                StreamingProtocol::Srt,
                StreamingCodec::H265,
                PresentationDepth::Sdr10,
            ),
            (
                StreamingProtocol::Hls,
                StreamingCodec::H264,
                PresentationDepth::Sdr8,
            ),
            (
                StreamingProtocol::Hls,
                StreamingCodec::H265,
                PresentationDepth::Sdr10,
            ),
            (
                StreamingProtocol::Hls,
                StreamingCodec::AV1,
                PresentationDepth::Sdr10,
            ),
            (
                StreamingProtocol::Dash,
                StreamingCodec::H264,
                PresentationDepth::Sdr8,
            ),
            (
                StreamingProtocol::Dash,
                StreamingCodec::H265,
                PresentationDepth::Sdr10,
            ),
            (
                StreamingProtocol::Dash,
                StreamingCodec::AV1,
                PresentationDepth::Sdr10,
            ),
            (
                StreamingProtocol::Rtmp(RtmpCodecContract::Legacy),
                StreamingCodec::H264,
                PresentationDepth::Sdr8,
            ),
            (
                StreamingProtocol::Rtmp(RtmpCodecContract::Legacy),
                StreamingCodec::H265,
                PresentationDepth::Sdr8,
            ),
            (
                StreamingProtocol::Rtmp(RtmpCodecContract::Legacy),
                StreamingCodec::AV1,
                PresentationDepth::Sdr8,
            ),
            (
                StreamingProtocol::Rtmp(RtmpCodecContract::Enhanced),
                StreamingCodec::H264,
                PresentationDepth::Sdr8,
            ),
            (
                StreamingProtocol::Rtmp(RtmpCodecContract::Enhanced),
                StreamingCodec::H265,
                PresentationDepth::Sdr10,
            ),
            (
                StreamingProtocol::Rtmp(RtmpCodecContract::Enhanced),
                StreamingCodec::AV1,
                PresentationDepth::Sdr10,
            ),
        ];
        for (protocol, codec, expected) in cases {
            let plan = StreamingPlan::resolve(
                protocol,
                codec.clone(),
                ten_bit_request(),
                all_streaming_capabilities(),
            );
            assert_eq!(
                plan.resolved.resolved, expected,
                "{protocol:?} {codec:?} resolved incorrectly"
            );
        }
    }

    // ── Subprocess lifecycle (requires ffmpeg) ─────────────────────

    #[test]
    fn spawn_recording_h264_and_feed_frames() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir();
        let path = dir.join("varda_test_recording.mp4");
        let path_str = path.to_str().unwrap();

        let mut sub =
            FfmpegSubprocess::spawn_recording(path_str, &RecordingCodec::H264, 64, 64, 30, None)
                .expect("failed to spawn recording");

        assert_eq!(sub.label(), path_str);
        assert_eq!(sub.frames_written(), 0);

        // Feed a few frames
        let frame = vec![0u8; 64 * 64 * 4]; // black RGBA
        for _ in 0..5 {
            let ok = sub.feed_frame(&frame);
            assert!(ok, "feed_frame should succeed");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Stop and verify
        sub.stop();
        assert!(sub.duration().as_millis() > 0);

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_recording_stop_is_idempotent() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir();
        let path = dir.join("varda_test_idempotent.mp4");
        let path_str = path.to_str().unwrap();

        let mut sub =
            FfmpegSubprocess::spawn_recording(path_str, &RecordingCodec::H264, 64, 64, 30, None)
                .unwrap();
        sub.stop();
        sub.stop();

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_srt_and_stop() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        // Use a high port unlikely to conflict
        let url = "srt://127.0.0.1:19876";
        let mut sub = FfmpegSubprocess::spawn_srt(
            url,
            &crate::renderer::context::SrtCodec::H264,
            PresentationRequest::default(),
            64,
            64,
            30,
            None,
        )
        .expect("failed to spawn SRT");

        assert_eq!(sub.label(), url);
        assert_eq!(sub.frames_written(), 0);

        // Feed a frame (won't block because of background writer thread)
        let frame = vec![128u8; 64 * 64 * 4];
        let _ = sub.feed_frame(&frame);

        // Stop cleanly
        sub.stop();
    }

    #[test]
    fn feed_frame_returns_false_after_stop() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir();
        let path = dir.join("varda_test_after_stop.mp4");
        let path_str = path.to_str().unwrap();

        let mut sub =
            FfmpegSubprocess::spawn_recording(path_str, &RecordingCodec::H264, 64, 64, 30, None)
                .unwrap();

        sub.stop();

        // After stop, feed_frame should return false (channel closed)
        let frame = vec![0u8; 64 * 64 * 4];
        assert!(!sub.feed_frame(&frame));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn recording_prores_codec() {
        if !ffmpeg_available() {
            eprintln!("Skipping test: ffmpeg not available");
            return;
        }
        let dir = std::env::temp_dir();
        let path = dir.join("varda_test_prores.mov");
        let path_str = path.to_str().unwrap();

        let mut sub =
            FfmpegSubprocess::spawn_recording(path_str, &RecordingCodec::ProRes, 64, 64, 30, None)
                .expect("failed to spawn ProRes recording");

        let frame = vec![0u8; 64 * 64 * 4];
        for _ in 0..3 {
            sub.feed_frame(&frame);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        sub.stop();

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn frame_channel_capacity_is_bounded() {
        // Verify the channel capacity constant
        assert_eq!(FRAME_CHANNEL_CAPACITY, 2);
    }

    #[test]
    fn full_channel_counts_as_a_drop_and_keeps_the_pipe_alive() {
        let (tx, _rx) = mpsc::sync_channel(1);
        tx.send(vec![0u8; 4]).unwrap();
        let dropped = AtomicU64::new(0);
        assert!(
            try_enqueue_frame(&tx, &dropped, "test-rec", &[1, 2, 3, 4]),
            "Full must not look like a dead subprocess"
        );
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        assert!(try_enqueue_frame(&tx, &dropped, "test-rec", &[5, 6, 7, 8]));
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn disconnected_channel_is_a_hard_failure() {
        let (tx, rx) = mpsc::sync_channel(1);
        drop(rx);
        let dropped = AtomicU64::new(0);
        assert!(!try_enqueue_frame(&tx, &dropped, "test-rec", &[1, 2, 3, 4]));
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    // ── Phase 19b: audio passthrough arg construction ──────────────

    fn dummy_audio(sample_rate: u32, channels: u16) -> AudioInput {
        let (_tx, rx) = crossbeam_channel::bounded::<PcmChunk>(4);
        AudioInput {
            rx,
            sample_rate,
            channels,
            lost_samples: Arc::new(AtomicU64::new(0)),
        }
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn prepare_audio_none_is_video_only() {
        // No audio input → no socket, no args: the video-only path is unchanged.
        let prepared = prepare_audio(None, false).unwrap();
        assert!(prepared.is_none());
    }

    #[test]
    fn prepare_audio_recording_uses_native_rate() {
        let p = prepare_audio(Some(dummy_audio(44100, 2)), false)
            .unwrap()
            .expect("audio prepared");
        // Input: raw f32le at the device's native rate + channel count.
        assert!(has_pair(&p.in_args, "-f", "f32le"));
        assert!(has_pair(&p.in_args, "-ar", "44100"));
        assert!(has_pair(&p.in_args, "-ac", "2"));
        assert!(p.in_args.contains(&"-i".to_string()));
        // Output: AAC, stereo downmix, async resample, explicit mapping.
        assert!(has_pair(&p.out_args, "-c:a", "aac"));
        assert!(has_pair(&p.out_args, "-ac", "2"));
        assert!(has_pair(&p.out_args, "-map", "0:v:0"));
        assert!(has_pair(&p.out_args, "-map", "1:a:0"));
        // Recording must NOT force 48k — native rate is preserved (Decision 5).
        assert!(!has_pair(&p.out_args, "-ar", "48000"));
    }

    /// Audio must be timed by its own sample count, never by when it turned up.
    ///
    /// This is the whole of the recording's timebase. A raw f32le input with no
    /// timestamp option gets PTS from the sample count, which is exactly the
    /// device's clock; adding `-use_wallclock_as_timestamps` replaces that with
    /// the moment each buffer reached the socket. The two look identical while
    /// everything is keeping up and diverge the instant anything stalls —
    /// notably when ffmpeg stops draining the audio socket because it is
    /// waiting on the video pipe. Every such stall lands in the file as a hole
    /// in the audio timeline, which is what "the music goes off-beat and drops
    /// bits when the renderer hitches" sounded like.
    ///
    /// Checked on both policies because all five spawners share `prepare_audio`
    /// and the streaming ones would otherwise regress unnoticed.
    #[test]
    fn audio_is_timed_by_its_sample_clock_not_by_arrival() {
        for is_stream in [false, true] {
            let p = prepare_audio(Some(dummy_audio(48000, 2)), is_stream)
                .unwrap()
                .expect("audio prepared");
            assert!(
                !p.in_args
                    .contains(&"-use_wallclock_as_timestamps".to_string()),
                "audio timestamps must come from the sample count, not arrival time \
                 (is_stream={is_stream}); see /spec/av-sync.md"
            );
        }
    }

    #[test]
    fn prepare_audio_stream_forces_48k() {
        let p = prepare_audio(Some(dummy_audio(44100, 2)), true)
            .unwrap()
            .expect("audio prepared");
        // Stream targets normalize to 48k (Decision 5).
        assert!(has_pair(&p.out_args, "-ar", "48000"));
    }

    #[test]
    fn prepare_audio_binds_tcp_endpoint() {
        let p = prepare_audio(Some(dummy_audio(48000, 1)), false)
            .unwrap()
            .expect("audio prepared");
        // The second input is the loopback TCP URL of the bound listener.
        let port = p.listener.local_addr().expect("listener addr").port();
        let expected = format!("tcp://127.0.0.1:{port}");
        assert!(
            p.in_args.contains(&expected),
            "audio input should be the bound loopback TCP URL"
        );
        // Mono device still reported faithfully on the input side.
        assert!(has_pair(&p.in_args, "-ac", "1"));
    }

    // ── A/V sync: holding the video timeline against the audio clock ────

    /// A renderer keeping up must not be padded at all.
    ///
    /// Padding is repair work; doing it when nothing is broken would inflate
    /// every recording with duplicate frames and make the encoder do work for
    /// nothing.
    #[test]
    fn a_renderer_on_time_is_never_padded() {
        let fps = 60.0;
        // Frame N arrives exactly on its slot, having already emitted N frames.
        for n in 0..120u64 {
            let elapsed =
                std::time::Duration::from_secs_f64(f64::from(u32::try_from(n).unwrap()) / fps);
            assert_eq!(
                FfmpegSubprocess::pad_count(elapsed, fps, n),
                0,
                "frame {n} arrived on time and should need no padding"
            );
        }
    }

    /// A gap in the renderer is covered by exactly as many frames as it swallowed.
    ///
    /// This is what keeps the video timeline honest. Raw video is timed by
    /// position — frame N is at N/fps whenever it was made — so a frame that is
    /// never written does not register as a pause, it shortens the recording.
    /// The audio track is timed by the capture device's sample clock and stays
    /// true to real time, so every unwritten frame used to slide the two apart,
    /// permanently and cumulatively. Repeating the last frame across the gap
    /// costs almost nothing to encode and keeps the two clocks agreeing.
    #[test]
    fn a_renderer_gap_is_covered_frame_for_frame() {
        let fps = 60.0;
        // 10 frames written, then a 100 ms stall: 6 frames' worth of real time
        // has passed unrecorded (10/60 s = 166.7 ms, +100 ms = 266.7 ms = 16
        // frames due), so 6 repeats bring the timeline back to real time.
        let elapsed = std::time::Duration::from_secs_f64(10.0 / fps + 0.1);
        assert_eq!(FfmpegSubprocess::pad_count(elapsed, fps, 10), 6);
    }

    /// A long freeze degrades gracefully instead of flooding the pipe.
    #[test]
    fn a_long_freeze_is_capped_rather_than_burst_written() {
        let fps = 60.0;
        let elapsed = std::time::Duration::from_secs(30);
        assert_eq!(
            FfmpegSubprocess::pad_count(elapsed, fps, 0),
            MAX_PAD_FRAMES_PER_ARRIVAL,
            "a 30 s stall must not emit 1800 frames in one go"
        );
    }

    /// Running ahead of the clock never produces negative padding.
    #[test]
    fn a_renderer_ahead_of_the_clock_is_not_padded() {
        let fps = 60.0;
        assert_eq!(
            FfmpegSubprocess::pad_count(std::time::Duration::from_millis(1), fps, 100),
            0
        );
    }

    /// PCM lost to backpressure is replaced by an equal span of silence.
    ///
    /// Now that audio is timed by its sample count, a dropped chunk does not
    /// leave a hole — it pulls everything after it earlier, so one dropout
    /// would put the rest of the recording out of sync with the picture.
    /// Writing silence in its place costs a brief mute and keeps the count
    /// exact. The assertion is on the byte count for that reason: it is the
    /// only thing the timeline is made of.
    #[test]
    fn lost_audio_is_replaced_by_an_equal_span_of_silence() {
        use std::io::Read as _;

        const CHUNK: usize = 64;
        const LOST: u64 = 500;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = crossbeam_channel::bounded::<PcmChunk>(4);
        let lost_samples = Arc::new(AtomicU64::new(LOST));

        let mut pipe = AudioPipe::start(listener, rx, lost_samples, "test".into()).expect("pipe");

        // Stand in for ffmpeg: connect and read everything written.
        let mut client = std::net::TcpStream::connect(addr).expect("connect");
        tx.send(PcmChunk {
            samples: vec![0.5; CHUNK],
        })
        .expect("send");

        let expected_samples = LOST as usize + CHUNK;
        let mut buf = vec![0u8; expected_samples * 4];
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("timeout");
        client.read_exact(&mut buf).expect("read all PCM");

        let samples: &[f32] = bytemuck::cast_slice(&buf);
        assert!(
            samples[..LOST as usize].iter().all(|s| *s == 0.0),
            "the gap should be filled with silence"
        );
        assert!(
            samples[LOST as usize..].iter().all(|s| *s == 0.5),
            "the real chunk should follow the silence, intact"
        );
        assert_eq!(pipe.silence_spliced(), LOST);

        pipe.stop();
    }

    /// With nothing lost, not a single extra sample is invented.
    #[test]
    fn an_unimpeded_audio_pipe_writes_only_captured_samples() {
        use std::io::Read as _;

        const CHUNK: usize = 32;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = crossbeam_channel::bounded::<PcmChunk>(4);
        let mut pipe = AudioPipe::start(
            listener,
            rx,
            Arc::new(AtomicU64::new(0)),
            "test-clean".into(),
        )
        .expect("pipe");

        let mut client = std::net::TcpStream::connect(addr).expect("connect");
        tx.send(PcmChunk {
            samples: vec![0.25; CHUNK],
        })
        .expect("send");

        let mut buf = vec![0u8; CHUNK * 4];
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("timeout");
        client.read_exact(&mut buf).expect("read chunk");

        let samples: &[f32] = bytemuck::cast_slice(&buf);
        assert!(samples.iter().all(|s| *s == 0.25));
        assert_eq!(pipe.silence_spliced(), 0);

        pipe.stop();
    }

    #[test]
    fn compute_rtmp_bitrate_720p() {
        let (maxrate, bufsize) = compute_rtmp_bitrate(1280, 720, 30);
        assert_eq!(maxrate, 3000);
        assert_eq!(bufsize, 6000);
    }

    #[test]
    fn compute_rtmp_bitrate_1080p() {
        let (maxrate, bufsize) = compute_rtmp_bitrate(1920, 1080, 30);
        assert_eq!(maxrate, 6000);
        assert_eq!(bufsize, 12000);
    }

    #[test]
    fn compute_rtmp_bitrate_1080p60() {
        let (maxrate, bufsize) = compute_rtmp_bitrate(1920, 1080, 60);
        assert_eq!(maxrate, 9000);
        assert_eq!(bufsize, 18000);
    }

    #[test]
    fn compute_rtmp_bitrate_4k() {
        let (maxrate, bufsize) = compute_rtmp_bitrate(3840, 2160, 30);
        assert_eq!(maxrate, 15000);
        assert_eq!(bufsize, 30000);
    }
}
