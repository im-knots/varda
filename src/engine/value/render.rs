//! Render/output configuration value types — plain, serializable data (no
//! `wgpu` / `winit` / `egui`) that describes *what* to render and *where* to
//! send it. Formerly `internal::renderer::config`; relocated here per
//! /spec/engine-value-types.md so the engine contract layer names these types
//! directly instead of reaching into `internal::renderer`. The GPU modules
//! (`context`, `tonemap`, `edge_blend`) `pub use` these to keep their existing
//! call paths working, and attach any framework-specific inherent impls there.

// ── Output rotation ──────────────────────────────────────────────────

/// Per-output rotation applied at the final blit stage.
/// For 90°/270°, intermediate textures are created at swapped dimensions
/// (portrait content for landscape projectors and vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OutputRotation {
    #[default]
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl OutputRotation {
    /// All rotation variants for UI dropdowns.
    pub const ALL: [OutputRotation; 4] = [
        OutputRotation::Deg0,
        OutputRotation::Deg90,
        OutputRotation::Deg180,
        OutputRotation::Deg270,
    ];

    /// GPU-side index (0–3) for the shader uniform.
    pub fn index(&self) -> u32 {
        match self {
            OutputRotation::Deg0 => 0,
            OutputRotation::Deg90 => 1,
            OutputRotation::Deg180 => 2,
            OutputRotation::Deg270 => 3,
        }
    }

    /// Whether this rotation swaps width and height.
    pub fn swaps_dimensions(&self) -> bool {
        matches!(self, OutputRotation::Deg90 | OutputRotation::Deg270)
    }

    /// Effective texture dimensions after rotation.
    /// For 0°/180° returns (w, h); for 90°/270° returns (h, w).
    pub fn effective_dimensions(&self, w: u32, h: u32) -> (u32, u32) {
        if self.swaps_dimensions() {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Human-readable label for UI display.
    pub fn label(&self) -> &'static str {
        match self {
            OutputRotation::Deg0 => "0°",
            OutputRotation::Deg90 => "90°",
            OutputRotation::Deg180 => "180°",
            OutputRotation::Deg270 => "270°",
        }
    }
}

// ── Output source ────────────────────────────────────────────────────

/// Content source that an output window can display
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum OutputSource {
    /// The master mix (final composited output)
    Master,
    /// A specific channel's composited output (by index)
    Channel(usize),
    /// A subset of channels composited together (sub-mix).
    /// Each channel contributes with its own opacity and blend mode.
    /// Master effects are NOT applied to sub-mixes.
    Channels(Vec<usize>),
    /// A specific deck's raw output (channel index, deck index)
    Deck(usize, usize),
    /// The domemaster fisheye output (equidistant azimuthal projection)
    Domemaster,
}

impl OutputSource {
    /// Returns the channel indices involved in this source, if any.
    pub fn channel_indices(&self) -> Option<Vec<usize>> {
        match self {
            OutputSource::Channel(idx) => Some(vec![*idx]),
            OutputSource::Channels(indices) => Some(indices.clone()),
            _ => None,
        }
    }
}

impl std::fmt::Display for OutputSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputSource::Master => write!(f, "Master"),
            OutputSource::Channel(idx) => write!(f, "Ch {}", idx),
            OutputSource::Channels(indices) => {
                let names: Vec<String> = indices.iter().map(|i| format!("Ch {}", i)).collect();
                write!(f, "{}", names.join("+"))
            }
            OutputSource::Deck(ch, dk) => write!(f, "Ch {} Deck {}", ch + 1, dk + 1),
            OutputSource::Domemaster => write!(f, "Domemaster"),
        }
    }
}

// ── Calibration mode ─────────────────────────────────────────────────

/// Per-output calibration display mode.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
pub enum CalibrationMode {
    /// Normal content rendering.
    #[default]
    Off,
    /// A single full-frame test card fills the whole output, bypassing surface
    /// geometry and warp — for physical projector alignment.
    Projector,
    /// Each surface shows a colored per-surface test card through its own warp —
    /// for verifying surface mapping and warp.
    Surfaces,
}

// ── Output target ────────────────────────────────────────────────────

/// Where an output sends its content — unified across windowed and headless outputs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum OutputTarget {
    /// Floating window (default)
    Windowed,
    /// Fullscreen/borderless on a specific monitor (identified by name + index)
    Display {
        /// Monitor name (e.g. "Built-in Retina Display", "HDMI-1")
        name: String,
        /// Index into the available monitors list (for lookup)
        monitor_index: usize,
    },
    /// Record frames to a video file via ffmpeg subprocess
    Recording {
        path: String,
        codec: RecordingCodec,
        /// Audio passthrough device NAME (None = silent). See spec/audio-passthrough.md.
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames via SRT (Secure Reliable Transport) through ffmpeg
    SrtStream {
        url: String,
        codec: SrtCodec,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames as HLS segments via ffmpeg
    HlsStream {
        name: String,
        codec: StreamingCodec,
        low_latency: bool,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames as DASH segments via ffmpeg
    DashStream {
        name: String,
        codec: StreamingCodec,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Push frames to an RTMP/RTMPS ingest endpoint via ffmpeg
    RtmpStream {
        url: String,
        codec: StreamingCodec,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Send frames over NDI network protocol
    NdiSend { sender_name: String },
    /// Publish frames via Syphon (macOS inter-app sharing)
    SyphonServer { server_name: String },
}

impl OutputTarget {
    /// Whether this target requires an OS window.
    pub fn is_windowed(&self) -> bool {
        matches!(self, OutputTarget::Windowed | OutputTarget::Display { .. })
    }

    /// Whether this target is headless (no OS window).
    pub fn is_headless(&self) -> bool {
        !self.is_windowed()
    }

    /// The selected audio passthrough device name, if this is an ffmpeg target
    /// configured with audio. `None` for video-only or non-ffmpeg targets.
    pub fn audio_device(&self) -> Option<&str> {
        match self {
            OutputTarget::Recording { audio_device, .. }
            | OutputTarget::SrtStream { audio_device, .. }
            | OutputTarget::HlsStream { audio_device, .. }
            | OutputTarget::DashStream { audio_device, .. }
            | OutputTarget::RtmpStream { audio_device, .. } => audio_device.as_deref(),
            _ => None,
        }
    }

    /// Return a clone of this target with the audio passthrough device replaced.
    /// No-op for non-ffmpeg targets. Lets the GUI flip the device without
    /// re-specifying every variant field.
    pub fn with_audio_device(&self, device: Option<String>) -> OutputTarget {
        let mut target = self.clone();
        match &mut target {
            OutputTarget::Recording { audio_device, .. }
            | OutputTarget::SrtStream { audio_device, .. }
            | OutputTarget::HlsStream { audio_device, .. }
            | OutputTarget::DashStream { audio_device, .. }
            | OutputTarget::RtmpStream { audio_device, .. } => *audio_device = device,
            _ => {}
        }
        target
    }
}

impl std::fmt::Display for OutputTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputTarget::Windowed => write!(f, "Windowed"),
            OutputTarget::Display { name, .. } => write!(f, "{}", name),
            OutputTarget::Recording { path, codec, .. } => write!(f, "Rec [{}]: {}", codec, path),
            OutputTarget::SrtStream { url, codec, .. } => write!(f, "SRT [{}]: {}", codec, url),
            OutputTarget::HlsStream {
                name,
                codec,
                low_latency,
                ..
            } => {
                if *low_latency {
                    write!(f, "LL-HLS [{}]: {}", codec, name)
                } else {
                    write!(f, "HLS [{}]: {}", codec, name)
                }
            }
            OutputTarget::DashStream { name, codec, .. } => write!(f, "DASH [{}]: {}", codec, name),
            OutputTarget::RtmpStream { url, codec, .. } => write!(f, "RTMP [{}]: {}", codec, url),
            OutputTarget::NdiSend { sender_name } => write!(f, "NDI: {}", sender_name),
            OutputTarget::SyphonServer { server_name } => write!(f, "Syphon: {}", server_name),
        }
    }
}

// ── ffmpeg codecs ────────────────────────────────────────────────────

/// Recording codec for ffmpeg subprocess.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum RecordingCodec {
    /// H.264 ultrafast preset (-c:v libx264 -preset ultrafast -crf 18)
    H264,
    /// H.265 / HEVC (-c:v libx265 -preset ultrafast -crf 20)
    H265,
    /// AV1 via SVT-AV1 (-c:v libsvtav1 -preset 10 -crf 28)
    AV1,
    /// ProRes 422 (-c:v prores_ks -profile:v 2)
    ProRes,
    /// ProRes 4444 with alpha (-c:v prores_ks -profile:v 4 -pix_fmt yuva444p10le)
    ProRes4444,
    /// HAP (-c:v hap -format hap)
    Hap,
    /// HAP Alpha (-c:v hap -format hap_alpha)
    HapAlpha,
    /// HAP Q (-c:v hap -format hap_q)
    HapQ,
}

impl std::fmt::Display for RecordingCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingCodec::H264 => write!(f, "H.264"),
            RecordingCodec::H265 => write!(f, "H.265 (HEVC)"),
            RecordingCodec::AV1 => write!(f, "AV1"),
            RecordingCodec::ProRes => write!(f, "ProRes 422"),
            RecordingCodec::ProRes4444 => write!(f, "ProRes 4444"),
            RecordingCodec::Hap => write!(f, "HAP"),
            RecordingCodec::HapAlpha => write!(f, "HAP Alpha"),
            RecordingCodec::HapQ => write!(f, "HAP Q"),
        }
    }
}

/// Streaming codec for SRT output.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum SrtCodec {
    /// H.264 ultrafast + zerolatency
    #[default]
    H264,
    /// H.265 / HEVC ultrafast + zerolatency
    H265,
}

impl std::fmt::Display for SrtCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SrtCodec::H264 => write!(f, "H.264"),
            SrtCodec::H265 => write!(f, "H.265 (HEVC)"),
        }
    }
}

/// Streaming codec for HLS/DASH output.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum StreamingCodec {
    /// H.264 ultrafast preset
    #[default]
    H264,
    /// H.265 / HEVC ultrafast preset
    H265,
    /// AV1 via SVT-AV1
    AV1,
}

impl std::fmt::Display for StreamingCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamingCodec::H264 => write!(f, "H.264"),
            StreamingCodec::H265 => write!(f, "H.265 (HEVC)"),
            StreamingCodec::AV1 => write!(f, "AV1"),
        }
    }
}

// ── Tonemap ──────────────────────────────────────────────────────────

/// Tonemapping mode selection.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
pub enum TonemapMode {
    /// Clamp to [0, 1] — equivalent to pre-tonemap behavior.
    Bypass = 0,
    /// ACES filmic curve — smooth highlight rolloff, preserves saturation.
    #[default]
    Aces = 1,
    /// Simple Reinhard: x/(x+1) per channel.
    Reinhard = 2,
    /// Reinhard with white point control, uses full SDR range.
    ReinhardExtended = 3,
    /// Hable/Uncharted 2 filmic curve — nice toe and shoulder.
    HableFilmic = 4,
    /// Gran Turismo style (Uchimura) — tunable shoulder and toe.
    Uchimura = 5,
    /// AMD Lottes — fast, invertible, high contrast.
    Lottes = 6,
    /// AgX — neutral, minimal hue shift, modern ACES alternative.
    AgX = 7,
    /// Khronos PBR Neutral — color-accurate, minimal look.
    KhronosPbrNeutral = 8,
}

// ── Edge blend ───────────────────────────────────────────────────────

/// Controls whether edge blend config is user-set or auto-computed from surface topology.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
    Default,
)]
pub enum EdgeBlendMode {
    /// User sets each edge manually (default — preserves existing behavior).
    #[default]
    Manual,
    /// Blend config is auto-derived from overlapping surfaces across outputs.
    Auto,
}

/// Per-edge blend configuration.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, utoipa::ToSchema)]
pub struct EdgeBlendEdge {
    pub enabled: bool,
    /// Blend zone width as fraction of output dimension (0.0–0.5).
    pub width: f32,
    /// Gamma curve exponent for the blend ramp (typically 1.0–3.0).
    pub gamma: f32,
}

impl Default for EdgeBlendEdge {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 0.1,
            gamma: 2.2,
        }
    }
}

/// Edge blending configuration for an output — four independent edges.
#[derive(
    Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, utoipa::ToSchema, Default,
)]
pub struct EdgeBlendConfig {
    pub left: EdgeBlendEdge,
    pub right: EdgeBlendEdge,
    pub top: EdgeBlendEdge,
    pub bottom: EdgeBlendEdge,
}

impl EdgeBlendConfig {
    /// Returns true if any edge has blending enabled.
    pub fn any_enabled(&self) -> bool {
        self.left.enabled || self.right.enabled || self.top.enabled || self.bottom.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── OutputRotation ───────────────────────────────────────────────

    #[test]
    fn rotation_default_is_deg0() {
        assert_eq!(OutputRotation::default(), OutputRotation::Deg0);
    }

    #[test]
    fn rotation_index_maps_each_variant() {
        assert_eq!(OutputRotation::Deg0.index(), 0);
        assert_eq!(OutputRotation::Deg90.index(), 1);
        assert_eq!(OutputRotation::Deg180.index(), 2);
        assert_eq!(OutputRotation::Deg270.index(), 3);
    }

    #[test]
    fn rotation_swaps_dimensions_only_for_quarter_turns() {
        assert!(!OutputRotation::Deg0.swaps_dimensions());
        assert!(OutputRotation::Deg90.swaps_dimensions());
        assert!(!OutputRotation::Deg180.swaps_dimensions());
        assert!(OutputRotation::Deg270.swaps_dimensions());
    }

    #[test]
    fn rotation_effective_dimensions_swaps_for_quarter_turns() {
        assert_eq!(
            OutputRotation::Deg0.effective_dimensions(1920, 1080),
            (1920, 1080)
        );
        assert_eq!(
            OutputRotation::Deg180.effective_dimensions(1920, 1080),
            (1920, 1080)
        );
        assert_eq!(
            OutputRotation::Deg90.effective_dimensions(1920, 1080),
            (1080, 1920)
        );
        assert_eq!(
            OutputRotation::Deg270.effective_dimensions(1920, 1080),
            (1080, 1920)
        );
    }

    #[test]
    fn rotation_label_per_variant() {
        assert_eq!(OutputRotation::Deg0.label(), "0°");
        assert_eq!(OutputRotation::Deg90.label(), "90°");
        assert_eq!(OutputRotation::Deg180.label(), "180°");
        assert_eq!(OutputRotation::Deg270.label(), "270°");
    }

    #[test]
    fn rotation_all_lists_every_variant_in_order() {
        assert_eq!(
            OutputRotation::ALL,
            [
                OutputRotation::Deg0,
                OutputRotation::Deg90,
                OutputRotation::Deg180,
                OutputRotation::Deg270,
            ]
        );
    }

    // ── OutputSource ─────────────────────────────────────────────────

    #[test]
    fn source_channel_indices_extracts_only_channel_variants() {
        assert_eq!(OutputSource::Master.channel_indices(), None);
        assert_eq!(OutputSource::Domemaster.channel_indices(), None);
        assert_eq!(OutputSource::Deck(0, 1).channel_indices(), None);
        assert_eq!(OutputSource::Channel(2).channel_indices(), Some(vec![2]));
        assert_eq!(
            OutputSource::Channels(vec![0, 3, 5]).channel_indices(),
            Some(vec![0, 3, 5])
        );
    }

    #[test]
    fn source_display_formats() {
        assert_eq!(OutputSource::Master.to_string(), "Master");
        assert_eq!(OutputSource::Channel(0).to_string(), "Ch 0");
        assert_eq!(OutputSource::Channels(vec![0, 1]).to_string(), "Ch 0+Ch 1");
        // Deck display is 1-indexed for humans.
        assert_eq!(OutputSource::Deck(0, 0).to_string(), "Ch 1 Deck 1");
        assert_eq!(OutputSource::Domemaster.to_string(), "Domemaster");
    }

    // ── OutputTarget ─────────────────────────────────────────────────

    #[test]
    fn target_windowed_predicates() {
        assert!(OutputTarget::Windowed.is_windowed());
        assert!(!OutputTarget::Windowed.is_headless());
        let display = OutputTarget::Display {
            name: "HDMI-1".into(),
            monitor_index: 0,
        };
        assert!(display.is_windowed());
        assert!(!display.is_headless());
    }

    #[test]
    fn target_headless_predicates() {
        let ndi = OutputTarget::NdiSend {
            sender_name: "Out".into(),
        };
        assert!(ndi.is_headless());
        assert!(!ndi.is_windowed());
        let rec = OutputTarget::Recording {
            path: "/tmp/out.mov".into(),
            codec: RecordingCodec::H264,
            audio_device: None,
        };
        assert!(rec.is_headless());
    }

    #[test]
    fn target_audio_device_only_for_configured_ffmpeg_targets() {
        // No audio configured → None even on an ffmpeg target.
        assert_eq!(
            OutputTarget::Recording {
                path: "/tmp/a.mov".into(),
                codec: RecordingCodec::H264,
                audio_device: None,
            }
            .audio_device(),
            None
        );
        // Configured audio → Some.
        assert_eq!(
            OutputTarget::SrtStream {
                url: "srt://host:9000".into(),
                codec: SrtCodec::H264,
                audio_device: Some("BlackHole 2ch".into()),
            }
            .audio_device(),
            Some("BlackHole 2ch")
        );
        // Non-ffmpeg targets never carry audio.
        assert_eq!(OutputTarget::Windowed.audio_device(), None);
        assert_eq!(
            OutputTarget::NdiSend {
                sender_name: "Out".into(),
            }
            .audio_device(),
            None
        );
    }

    #[test]
    fn target_with_audio_device_updates_ffmpeg_targets() {
        let rec = OutputTarget::Recording {
            path: "/tmp/a.mov".into(),
            codec: RecordingCodec::H264,
            audio_device: None,
        };
        let updated = rec.with_audio_device(Some("Mic".into()));
        assert_eq!(updated.audio_device(), Some("Mic"));
        // Clearing it back to None works too.
        assert_eq!(updated.with_audio_device(None).audio_device(), None);
    }

    #[test]
    fn target_with_audio_device_is_noop_for_non_ffmpeg() {
        let windowed = OutputTarget::Windowed;
        let updated = windowed.with_audio_device(Some("Mic".into()));
        // Still Windowed, still no audio device.
        assert_eq!(updated, OutputTarget::Windowed);
        assert_eq!(updated.audio_device(), None);
    }

    #[test]
    fn target_display_formats() {
        assert_eq!(OutputTarget::Windowed.to_string(), "Windowed");
        assert_eq!(
            OutputTarget::Display {
                name: "Built-in".into(),
                monitor_index: 1,
            }
            .to_string(),
            "Built-in"
        );
        assert_eq!(
            OutputTarget::HlsStream {
                name: "live".into(),
                codec: StreamingCodec::H264,
                low_latency: true,
                audio_device: None,
            }
            .to_string(),
            "LL-HLS [H.264]: live"
        );
        assert_eq!(
            OutputTarget::HlsStream {
                name: "live".into(),
                codec: StreamingCodec::H264,
                low_latency: false,
                audio_device: None,
            }
            .to_string(),
            "HLS [H.264]: live"
        );
    }

    // ── EdgeBlend ────────────────────────────────────────────────────

    #[test]
    fn edge_blend_edge_default_is_disabled_with_ramp_params() {
        let e = EdgeBlendEdge::default();
        assert!(!e.enabled);
        assert!((e.width - 0.1).abs() < 1e-6);
        assert!((e.gamma - 2.2).abs() < 1e-6);
    }

    #[test]
    fn edge_blend_config_default_has_nothing_enabled() {
        assert!(!EdgeBlendConfig::default().any_enabled());
    }

    #[test]
    fn edge_blend_config_any_enabled_detects_each_edge() {
        for pick in 0..4 {
            let mut cfg = EdgeBlendConfig::default();
            match pick {
                0 => cfg.left.enabled = true,
                1 => cfg.right.enabled = true,
                2 => cfg.top.enabled = true,
                _ => cfg.bottom.enabled = true,
            }
            assert!(cfg.any_enabled(), "edge {pick} should register as enabled");
        }
    }
}
