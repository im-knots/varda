mod effect;
mod render;
mod source;

pub use render::get_current_date;

use crate::isf::{ISFPass, ISFShader};
use crate::params::ShaderParams;
use crate::renderer::{BlitPipeline, ComputePipeline, HapConvertPipeline, UnifiedPipeline};
use crate::video::{HapTextureFormat, PlaybackSnapshot, VideoCommand, VideoDecodeHandle};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Generate a short 8-character hex UUID for entity identity.
pub fn generate_short_uuid() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

/// Scaling mode for non-shader sources (images, video)
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum ScalingMode {
    /// Scale to fill the entire target, cropping edges if aspect ratio differs
    #[default]
    Fill,
    /// Scale to fit within the target, letterboxing if aspect ratio differs
    Fit,
    /// Stretch to exactly match target dimensions (may distort)
    Stretch,
    /// No scaling, center at native resolution
    Center,
}

impl ScalingMode {
    /// Compute UV scale and offset for blitting source into target
    /// Returns (uv_scale, uv_offset) to transform target UVs to source UVs
    pub fn compute_uv_transform(
        &self,
        source_w: u32,
        source_h: u32,
        target_w: u32,
        target_h: u32,
    ) -> ([f32; 2], [f32; 2]) {
        let src_aspect = source_w as f32 / source_h as f32;
        let tgt_aspect = target_w as f32 / target_h as f32;

        match self {
            ScalingMode::Stretch => ([1.0, 1.0], [0.0, 0.0]),
            ScalingMode::Fill => {
                if src_aspect > tgt_aspect {
                    let scale_x = tgt_aspect / src_aspect;
                    let offset_x = (1.0 - scale_x) * 0.5;
                    ([scale_x, 1.0], [offset_x, 0.0])
                } else {
                    let scale_y = src_aspect / tgt_aspect;
                    let offset_y = (1.0 - scale_y) * 0.5;
                    ([1.0, scale_y], [0.0, offset_y])
                }
            }
            ScalingMode::Fit => {
                if src_aspect > tgt_aspect {
                    let scale_y = src_aspect / tgt_aspect;
                    let offset_y = (1.0 - scale_y) * 0.5;
                    ([1.0, scale_y], [0.0, offset_y])
                } else {
                    let scale_x = tgt_aspect / src_aspect;
                    let offset_x = (1.0 - scale_x) * 0.5;
                    ([scale_x, 1.0], [offset_x, 0.0])
                }
            }
            ScalingMode::Center => {
                let scale_x = target_w as f32 / source_w as f32;
                let scale_y = target_h as f32 / source_h as f32;
                let offset_x = (1.0 - scale_x) * 0.5;
                let offset_y = (1.0 - scale_y) * 0.5;
                ([scale_x, scale_y], [offset_x, offset_y])
            }
        }
    }
}

/// Double-buffered staging buffers for non-blocking GPU texture uploads.
///
/// Uses a ping-pong pattern: CPU writes to buffer\[current\], GPU copies from
/// buffer\[1-current\]. By the time we circle back two frames later, the GPU
/// is done with the buffer and it can be re-mapped without stalling.
///
/// This eliminates the per-frame staging buffer allocation that
/// `queue.write_texture()` performs internally, which can block for 2-9ms
/// under GPU saturation.
pub struct VideoStagingBuffers {
    buffers: [wgpu::Buffer; 2],
    current: usize,
    mapped: [Arc<AtomicBool>; 2],
    /// Bytes per row padded to wgpu::COPY_BYTES_PER_ROW_ALIGNMENT (256).
    padded_bpr: u32,
    /// Unpadded bytes per row (actual source data stride).
    unpadded_bpr: u32,
    /// Number of rows (height for RGBA, blocks_y for compressed).
    rows: u32,
    /// Tracks which buffers need map_async after the next queue.submit().
    needs_remap: [bool; 2],
}

impl VideoStagingBuffers {
    /// Create a new double-buffered staging pair.
    /// Buffers start unmapped — call `request_remap()` after the first
    /// `queue.submit()` to begin the mapping lifecycle.
    pub fn new(device: &wgpu::Device, unpadded_bpr: u32, rows: u32, label: &str) -> Self {
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bpr = (unpadded_bpr + align - 1) & !(align - 1);
        let buffer_size = (padded_bpr as u64) * (rows as u64);

        let make_buf = |idx: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{} Staging {}", label, idx)),
                size: buffer_size,
                usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };

        let mapped_0 = Arc::new(AtomicBool::new(false));
        let mapped_1 = Arc::new(AtomicBool::new(false));

        Self {
            buffers: [make_buf(0), make_buf(1)],
            current: 0,
            mapped: [mapped_0, mapped_1],
            padded_bpr,
            unpadded_bpr,
            rows,
            needs_remap: [true, true],
        }
    }

    /// Write frame data into the current staging buffer and encode a copy
    /// to the destination texture. Returns true if the upload was performed.
    pub fn upload(
        &mut self,
        data: &[u8],
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        encoder: &mut wgpu::CommandEncoder,
    ) -> bool {
        let idx = self.current;
        if !self.mapped[idx].load(Ordering::Acquire) {
            // Buffer not yet mapped — skip this upload.
            // The stale texture from last frame will remain on screen.
            return false;
        }

        {
            let buf = &self.buffers[idx];
            let mut view = buf.slice(..).get_mapped_range_mut();
            if self.padded_bpr == self.unpadded_bpr {
                // Row stride matches — single memcpy
                let copy_len = (self.unpadded_bpr as usize) * (self.rows as usize);
                view.slice(..copy_len).copy_from_slice(&data[..copy_len]);
            } else {
                // Need to copy row-by-row with padding
                for row in 0..self.rows as usize {
                    let src_start = row * self.unpadded_bpr as usize;
                    let dst_start = row * self.padded_bpr as usize;
                    view.slice(dst_start..dst_start + self.unpadded_bpr as usize)
                        .copy_from_slice(&data[src_start..src_start + self.unpadded_bpr as usize]);
                }
            }
        }

        self.buffers[idx].unmap();
        self.mapped[idx].store(false, Ordering::Release);

        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffers[idx],
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bpr),
                    rows_per_image: Some(self.rows),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        // Mark for re-mapping after submit
        self.needs_remap[idx] = true;

        // Advance to next buffer
        self.current = 1 - self.current;
        true
    }

    /// Request re-mapping of any buffers that were used since the last call.
    /// **Must be called AFTER `queue.submit()`** — calling `map_async` before
    /// submit can complete synchronously on UMA/Metal, leaving the buffer
    /// mapped during submit (which is a validation error).
    pub fn request_remap(&mut self) {
        for i in 0..2 {
            if self.needs_remap[i] {
                self.needs_remap[i] = false;
                let flag = self.mapped[i].clone();
                self.buffers[i]
                    .slice(..)
                    .map_async(wgpu::MapMode::Write, move |result| {
                        if result.is_ok() {
                            flag.store(true, Ordering::Release);
                        }
                    });
            }
        }
    }
}

/// Source type for a deck - what generates the base image
pub enum DeckSource {
    /// ISF shader generator
    Shader {
        shader: ISFShader,
        pipeline: UnifiedPipeline,
        pass_buffers: HashMap<String, PassBuffer>,
        passes: Vec<ISFPass>,
        /// GPU textures loaded from ISF IMPORTED images (sorted by name for deterministic binding)
        imported_textures: Vec<(String, wgpu::Texture, wgpu::TextureView)>,
        /// Preprocessor texture slots for analyzer-driven textures
        preprocessor_textures: Vec<PreprocessorSlot>,
    },
    /// Video file playback (ffmpeg CPU decode → RGBA, background decode thread)
    Video {
        handle: VideoDecodeHandle,
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
        staging: VideoStagingBuffers,
    },
    /// HAP video playback (GPU-native BCn, background decode thread)
    HapVideo {
        handle: VideoDecodeHandle,
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        alpha_texture: Option<wgpu::Texture>,
        alpha_texture_view: Option<wgpu::TextureView>,
        dummy_alpha_view: wgpu::TextureView,
        convert_pipeline: HapConvertPipeline,
        blit_pipeline: BlitPipeline,
        hap_format: HapTextureFormat,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
        staging: VideoStagingBuffers,
        alpha_staging: Option<VideoStagingBuffers>,
    },
    /// Static image
    Image {
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
    /// Solid color fill
    SolidColor { color: [f64; 4] },
    /// External live source (camera, NDI, Syphon, SRT, HLS, DASH, RTMP)
    ExternalSource {
        kind: ExternalSourceKind,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
    /// GLSL compute shader generator
    ComputeShader {
        shader: ISFShader,
        pipeline: ComputePipeline,
    },
}

/// Discriminant for external source types sharing the same DeckSource layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalSourceKind {
    Camera(crate::camera::CameraId),
    Ndi(usize),
    Syphon(usize),
    Srt(usize),
    Hls(usize),
    Dash(usize),
    Rtmp(usize),
    Html(usize),
}

impl ExternalSourceKind {
    /// Get the source type string for serialization.
    pub fn source_type(&self) -> &str {
        match self {
            Self::Camera(_) => "camera",
            Self::Ndi(_) => "ndi",
            Self::Syphon(_) => "syphon",
            Self::Srt(_) => "srt",
            Self::Hls(_) => "hls",
            Self::Dash(_) => "dash",
            Self::Rtmp(_) => "rtmp",
            Self::Html(_) => "html",
        }
    }

    /// Render label for logging/debug.
    pub fn label(&self) -> &str {
        match self {
            Self::Camera(_) => "Camera",
            Self::Ndi(_) => "NDI",
            Self::Syphon(_) => "Syphon",
            Self::Srt(_) | Self::Hls(_) | Self::Dash(_) | Self::Rtmp(_) => "Stream",
            Self::Html(_) => "HTML",
        }
    }
}

/// A preprocessor texture slot — holds a GPU texture that gets updated with analyzer output.
pub struct PreprocessorSlot {
    /// Name prefix for shader uniforms (e.g. "depth" → `depth_depth_map`)
    pub name: String,
    /// Analyzer type this preprocessor needs (e.g. "depth_estimate")
    pub analyzer_type: String,
    /// Options to pass when starting the analyzer
    pub options: serde_json::Value,
    /// GPU texture (initially 1×1 black, updated at runtime)
    pub texture: wgpu::Texture,
    /// Texture view for shader binding
    pub view: wgpu::TextureView,
}

/// An effect in the deck's effect chain (ISF filter)
pub struct Effect {
    /// Stable UUID for this effect (8-char hex)
    pub uuid: String,
    /// Cached "fx_{uuid}" prefix for modulation key lookups (avoids per-frame format!)
    pub param_prefix: String,
    pub shader: ISFShader,
    pub pipeline: UnifiedPipeline,
    pub enabled: bool,
    pub params: ShaderParams,
    pub pass_buffers: HashMap<String, PassBuffer>,
    pub passes: Vec<ISFPass>,
    pub target_format: wgpu::TextureFormat,
    /// GPU textures loaded from ISF IMPORTED images (sorted by name for deterministic binding)
    pub imported_textures: Vec<(String, wgpu::Texture, wgpu::TextureView)>,
    /// Preprocessor textures from PREPROCESSORS declarations (placeholder until analyzer provides data)
    pub preprocessor_textures: Vec<PreprocessorSlot>,
    /// Phase accumulators for smooth speed transitions
    pub phase_accumulators: [f32; 4],
    /// Phase input config from shader metadata
    pub phase_inputs_config: Option<Vec<crate::isf::PhaseInput>>,
}

// Effect impl is in effect.rs

/// Multi-pass buffer for ISF PASSES array
/// Uses ping-pong buffers for persistent passes to allow read/write in same frame
pub struct PassBuffer {
    /// Buffer name (from ISF PASSES TARGET field)
    pub name: String,
    /// Primary texture (read source for persistent buffers)
    pub texture_a: wgpu::Texture,
    /// Primary texture view
    pub view_a: wgpu::TextureView,
    /// Secondary texture (write target for persistent buffers) - only for persistent
    pub texture_b: Option<wgpu::Texture>,
    /// Secondary texture view
    pub view_b: Option<wgpu::TextureView>,
    /// Whether this buffer persists across frames
    pub persistent: bool,
    /// Current read index (0 = read from A, 1 = read from B)
    pub read_idx: usize,
}

/// A Deck is an independent render unit that outputs a texture
pub struct Deck {
    /// Stable UUID for this deck (8-char hex, persists across moves/saves)
    uuid: String,

    /// Cached "deck_{uuid}" prefix for modulation key lookups (avoids per-frame format!)
    param_prefix: String,

    /// Name of this deck's source
    source_name: String,

    /// Original file path used to create this deck (for persistence).
    /// Shader path, video path, or image path. None for solid color / camera.
    source_path: Option<String>,

    /// Source type and pipeline (shader, video, or image)
    source: DeckSource,

    /// Generator shader parameters (if source is a shader)
    pub generator_params: ShaderParams,

    /// Render target texture (primary)
    pub texture: wgpu::Texture,

    /// Texture view
    pub texture_view: wgpu::TextureView,

    /// Secondary texture for ping-pong rendering in effect chain
    texture_b: wgpu::Texture,
    texture_b_view: wgpu::TextureView,

    /// Effect chain (ISF filters applied to generator output)
    pub effects: Vec<Effect>,

    /// Deck opacity (0.0 - 1.0)
    pub opacity: f32,

    /// Accumulated render time for TIME uniform (advances by fixed dt each render).
    /// Decoupled from wall clock so skipped frames don't cause animation jumps.
    render_time: f32,

    /// Fixed time step per render (1/target_fps). Updated by the channel when
    /// the deck is rendered, so skipped frames simply don't advance render_time.
    render_dt: f32,

    /// Frame counter
    frame_count: u32,

    /// Last wall-clock render instant (for FPS measurement only, not for TIME uniform)
    last_frame_time: Instant,

    /// External source texture view (set each frame for ExternalSource decks)
    pub external_source_view: Option<wgpu::TextureView>,

    /// Smoothed FPS derived from actual render pipeline timing (EMA of 1/time_delta)
    fps_smoothed: f32,

    /// Phase accumulators for smooth speed transitions (generator shader)
    phase_accumulators: [f32; 4],

    /// Phase input config from generator shader metadata
    generator_phase_inputs: Option<Vec<crate::isf::PhaseInput>>,

    /// Per-deck analyzer instances (brightness, beat detection, etc.)
    pub(crate) analyzers: crate::analyzer::DeckAnalyzers,
}

/// Accessors for Deck properties.
/// Constructors are in source.rs, rendering in render.rs.
impl Deck {
    /// Get the stable UUID for this deck
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    /// Get the cached param prefix ("deck_{uuid}")
    pub fn param_prefix(&self) -> &str {
        &self.param_prefix
    }

    /// Set the UUID (used during scene restore to preserve identity)
    pub fn set_uuid(&mut self, uuid: String) {
        self.param_prefix = format!("deck_{}", uuid);
        self.uuid = uuid;
    }

    /// Get the source name (shader name, video filename, etc.)
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Override the display name (e.g. when loading a preset with a custom name).
    pub fn set_source_name(&mut self, name: String) {
        self.source_name = name;
    }

    /// Get the source file path (for persistence). None for solid color / camera.
    pub fn source_path(&self) -> Option<&str> {
        self.source_path.as_deref()
    }

    /// Get the source type as a string for serialization
    pub fn source_type(&self) -> &str {
        match &self.source {
            DeckSource::Shader { .. } => "shader",
            DeckSource::Video { .. } | DeckSource::HapVideo { .. } => "video",
            DeckSource::Image { .. } => "image",
            DeckSource::SolidColor { .. } => "solid_color",
            DeckSource::ExternalSource { kind, .. } => kind.source_type(),
            DeckSource::ComputeShader { .. } => "compute_shader",
        }
    }

    /// Get a read-only snapshot of the video playback state.
    pub fn playback_snapshot(&self) -> Option<PlaybackSnapshot> {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                Some(handle.playback_snapshot())
            }
            _ => None,
        }
    }

    /// Send a command to the video decode thread (no-op for non-video decks).
    fn video_send(&self, cmd: VideoCommand) -> bool {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                handle.send(cmd);
                true
            }
            _ => false,
        }
    }

    /// Toggle play/pause on the video decode thread.
    pub fn video_toggle_play(&self) -> bool {
        if let Some(snap) = self.playback_snapshot() {
            if snap.playing {
                self.video_send(VideoCommand::Pause)
            } else {
                self.video_send(VideoCommand::Play)
            }
        } else {
            false
        }
    }

    /// Set playing state on the video decode thread.
    pub fn video_set_playing(&self, playing: bool) -> bool {
        if playing {
            self.video_send(VideoCommand::Play)
        } else {
            self.video_send(VideoCommand::Pause)
        }
    }

    /// Set playback speed on the video decode thread.
    pub fn video_set_speed(&self, speed: f64) -> bool {
        self.video_send(VideoCommand::SetSpeed(speed))
    }

    /// Set loop mode on the video decode thread.
    pub fn video_set_loop_mode(&self, mode: crate::video::LoopMode) -> bool {
        self.video_send(VideoCommand::SetLoopMode(mode))
    }

    /// Set in-point on the video decode thread.
    pub fn video_set_in_point(&self, secs: f64) -> bool {
        self.video_send(VideoCommand::SetInPoint(secs))
    }

    /// Set out-point on the video decode thread.
    pub fn video_set_out_point(&self, secs: f64) -> bool {
        self.video_send(VideoCommand::SetOutPoint(secs))
    }

    /// Clear in/out points on the video decode thread.
    pub fn video_clear_in_out_points(&self) -> bool {
        self.video_send(VideoCommand::ClearInOutPoints)
    }

    /// Seek the video to a specific position in seconds.
    pub fn video_seek(&self, time_secs: f64) -> bool {
        self.video_send(VideoCommand::Seek(time_secs))
    }

    /// Get the solid color value (if source is a solid color)
    pub fn solid_color(&self) -> Option<[f32; 4]> {
        match &self.source {
            DeckSource::SolidColor { color } => Some([
                color[0] as f32,
                color[1] as f32,
                color[2] as f32,
                color[3] as f32,
            ]),
            _ => None,
        }
    }

    /// Set the solid color value (only applies to SolidColor sources)
    pub fn set_solid_color(&mut self, new_color: [f32; 4]) {
        if let DeckSource::SolidColor { color } = &mut self.source {
            *color = [
                new_color[0] as f64,
                new_color[1] as f64,
                new_color[2] as f64,
                new_color[3] as f64,
            ];
        }
    }

    /// Get the scaling mode (if applicable for this source type)
    pub fn scaling_mode(&self) -> Option<ScalingMode> {
        match &self.source {
            DeckSource::Image { scaling_mode, .. }
            | DeckSource::Video { scaling_mode, .. }
            | DeckSource::HapVideo { scaling_mode, .. }
            | DeckSource::ExternalSource { scaling_mode, .. } => Some(*scaling_mode),
            _ => None,
        }
    }

    /// Set the scaling mode (applies to Image, Video, HapVideo, and ExternalSource sources)
    pub fn set_scaling_mode(&mut self, mode: ScalingMode) {
        match &mut self.source {
            DeckSource::Image { scaling_mode, .. }
            | DeckSource::Video { scaling_mode, .. }
            | DeckSource::HapVideo { scaling_mode, .. }
            | DeckSource::ExternalSource { scaling_mode, .. } => *scaling_mode = mode,
            _ => {}
        }
    }

    /// Get the external source kind (if source is external)
    pub fn external_source_kind(&self) -> Option<ExternalSourceKind> {
        match &self.source {
            DeckSource::ExternalSource { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    /// Get the NDI receiver index (if source is NDI)
    pub fn ndi_receiver_idx(&self) -> Option<usize> {
        match &self.source {
            DeckSource::ExternalSource {
                kind: ExternalSourceKind::Ndi(idx),
                ..
            } => Some(*idx),
            _ => None,
        }
    }

    /// Get the Syphon client index (if source is Syphon)
    pub fn syphon_client_idx(&self) -> Option<usize> {
        match &self.source {
            DeckSource::ExternalSource {
                kind: ExternalSourceKind::Syphon(idx),
                ..
            } => Some(*idx),
            _ => None,
        }
    }

    /// Get the SRT/HLS/DASH/RTMP receiver index (if source is a stream)
    pub fn srt_receiver_idx(&self) -> Option<usize> {
        match &self.source {
            DeckSource::ExternalSource {
                kind: ExternalSourceKind::Srt(idx),
                ..
            }
            | DeckSource::ExternalSource {
                kind: ExternalSourceKind::Hls(idx),
                ..
            }
            | DeckSource::ExternalSource {
                kind: ExternalSourceKind::Dash(idx),
                ..
            }
            | DeckSource::ExternalSource {
                kind: ExternalSourceKind::Rtmp(idx),
                ..
            } => Some(*idx),
            _ => None,
        }
    }

    /// Get the camera ID (if source is a camera)
    pub fn camera_id(&self) -> Option<crate::camera::CameraId> {
        match &self.source {
            DeckSource::ExternalSource {
                kind: ExternalSourceKind::Camera(id),
                ..
            } => Some(*id),
            _ => None,
        }
    }

    /// Get the shader (if source is a shader or compute shader)
    pub fn shader(&self) -> Option<&ISFShader> {
        match &self.source {
            DeckSource::Shader { shader, .. } | DeckSource::ComputeShader { shader, .. } => {
                Some(shader)
            }
            _ => None,
        }
    }

    /// Set the fixed time step used for the TIME uniform.
    /// Called by the channel to keep render_dt in sync with the target FPS.
    pub fn set_render_dt(&mut self, dt: f32) {
        self.render_dt = dt;
    }

    /// Get the smoothed FPS derived from actual render pipeline timing
    pub fn fps(&self) -> f32 {
        self.fps_smoothed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_short_uuid_format() {
        let id = generate_short_uuid();
        assert_eq!(id.len(), 8, "UUID should be 8 chars");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "UUID should be hex: {id}"
        );
    }

    #[test]
    fn generate_short_uuid_unique() {
        let ids: Vec<String> = (0..100).map(|_| generate_short_uuid()).collect();
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), 100, "100 UUIDs should all be unique");
    }

    #[test]
    fn scaling_mode_default_is_fill() {
        assert_eq!(ScalingMode::default(), ScalingMode::Fill);
    }

    #[test]
    fn stretch_returns_identity() {
        let (scale, offset) = ScalingMode::Stretch.compute_uv_transform(800, 600, 1920, 1080);
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn stretch_same_aspect() {
        let (scale, offset) = ScalingMode::Stretch.compute_uv_transform(1920, 1080, 1920, 1080);
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn fill_same_aspect_is_identity() {
        let (scale, offset) = ScalingMode::Fill.compute_uv_transform(1920, 1080, 960, 540);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!((offset[0]).abs() < 1e-5);
        assert!((offset[1]).abs() < 1e-5);
    }

    #[test]
    fn fill_wide_source_crops_horizontal() {
        // Source 2:1, target 1:1 → crop left/right
        let (scale, offset) = ScalingMode::Fill.compute_uv_transform(200, 100, 100, 100);
        assert!(
            (scale[0] - 0.5).abs() < 1e-5,
            "scale_x should be 0.5, got {}",
            scale[0]
        );
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!(
            (offset[0] - 0.25).abs() < 1e-5,
            "offset_x should center crop"
        );
        assert!((offset[1]).abs() < 1e-5);
    }

    #[test]
    fn fill_tall_source_crops_vertical() {
        // Source 1:2, target 1:1 → crop top/bottom
        let (scale, offset) = ScalingMode::Fill.compute_uv_transform(100, 200, 100, 100);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!(
            (scale[1] - 0.5).abs() < 1e-5,
            "scale_y should be 0.5, got {}",
            scale[1]
        );
        assert!((offset[0]).abs() < 1e-5);
        assert!(
            (offset[1] - 0.25).abs() < 1e-5,
            "offset_y should center crop"
        );
    }

    #[test]
    fn fit_same_aspect_is_identity() {
        let (scale, offset) = ScalingMode::Fit.compute_uv_transform(1920, 1080, 960, 540);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!((offset[0]).abs() < 1e-5);
        assert!((offset[1]).abs() < 1e-5);
    }

    #[test]
    fn fit_wide_source_letterboxes() {
        // Source 2:1, target 1:1 → letterbox top/bottom
        let (scale, _offset) = ScalingMode::Fit.compute_uv_transform(200, 100, 100, 100);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 2.0).abs() < 1e-5, "scale_y={}", scale[1]);
    }

    #[test]
    fn fit_tall_source_pillarboxes() {
        // Source 1:2, target 1:1 → pillarbox left/right
        let (scale, _offset) = ScalingMode::Fit.compute_uv_transform(100, 200, 100, 100);
        assert!((scale[0] - 2.0).abs() < 1e-5, "scale_x={}", scale[0]);
        assert!((scale[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn center_smaller_source() {
        // Source 100x100 in target 200x200 → scale 2.0, offset -0.5
        let (scale, offset) = ScalingMode::Center.compute_uv_transform(100, 100, 200, 200);
        assert!((scale[0] - 2.0).abs() < 1e-5);
        assert!((scale[1] - 2.0).abs() < 1e-5);
        assert!((offset[0] - -0.5).abs() < 1e-5);
        assert!((offset[1] - -0.5).abs() < 1e-5);
    }

    #[test]
    fn center_larger_source() {
        // Source 400x400 in target 200x200 → scale 0.5, offset 0.25
        let (scale, offset) = ScalingMode::Center.compute_uv_transform(400, 400, 200, 200);
        assert!((scale[0] - 0.5).abs() < 1e-5);
        assert!((scale[1] - 0.5).abs() < 1e-5);
        assert!((offset[0] - 0.25).abs() < 1e-5);
        assert!((offset[1] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn center_same_size_is_identity() {
        let (scale, offset) = ScalingMode::Center.compute_uv_transform(1920, 1080, 1920, 1080);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!((offset[0]).abs() < 1e-5);
        assert!((offset[1]).abs() < 1e-5);
    }
}
