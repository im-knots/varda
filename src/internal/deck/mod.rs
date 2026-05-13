mod effect;
mod source;
mod render;

pub use render::get_current_date;

use crate::isf::{ISFShader, ISFPass};
use crate::params::ShaderParams;
use crate::renderer::{UnifiedPipeline, BlitPipeline, HapConvertPipeline};
use crate::video::{VideoPlayer, HapTextureFormat, hap::HapPlayer};
use std::collections::HashMap;
use std::time::Instant;

/// Generate a short 8-character hex UUID for entity identity.
pub fn generate_short_uuid() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

/// Scaling mode for non-shader sources (images, video)
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum ScalingMode {
    /// Scale to fill the entire target, cropping edges if aspect ratio differs
    Fill,
    /// Scale to fit within the target, letterboxing if aspect ratio differs
    Fit,
    /// Stretch to exactly match target dimensions (may distort)
    Stretch,
    /// No scaling, center at native resolution
    Center,
}

impl Default for ScalingMode {
    fn default() -> Self {
        ScalingMode::Fill
    }
}

impl ScalingMode {
    /// Compute UV scale and offset for blitting source into target
    /// Returns (uv_scale, uv_offset) to transform target UVs to source UVs
    pub fn compute_uv_transform(
        &self,
        source_w: u32, source_h: u32,
        target_w: u32, target_h: u32,
    ) -> ([f32; 2], [f32; 2]) {
        let src_aspect = source_w as f32 / source_h as f32;
        let tgt_aspect = target_w as f32 / target_h as f32;

        match self {
            ScalingMode::Stretch => {
                ([1.0, 1.0], [0.0, 0.0])
            }
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
    },
    /// Video file playback (ffmpeg CPU decode → RGBA)
    Video {
        player: VideoPlayer,
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        blit_pipeline: BlitPipeline,
    },
    /// HAP video playback (GPU-native BCn compressed textures)
    HapVideo {
        player: HapPlayer,
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        alpha_texture: Option<wgpu::Texture>,
        alpha_texture_view: Option<wgpu::TextureView>,
        dummy_alpha_view: wgpu::TextureView,
        convert_pipeline: HapConvertPipeline,
        blit_pipeline: BlitPipeline,
        hap_format: HapTextureFormat,
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
    SolidColor {
        color: [f64; 4],
    },
    /// Live camera feed (reads shared texture from CameraManager)
    Camera {
        camera_id: crate::camera::CameraId,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
    /// NDI network video input (reads shared texture from NdiManager)
    Ndi {
        /// Index into NdiManager's receiver list
        receiver_idx: usize,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
    /// Syphon inter-app video input (reads shared texture from SyphonManager, macOS only)
    Syphon {
        /// Index into SyphonManager's client list
        client_idx: usize,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
    /// SRT network video input (reads shared texture from StreamManager)
    Srt {
        /// Index into StreamManager's receiver list
        receiver_idx: usize,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
    /// HLS stream input (reads shared texture from StreamManager)
    Hls {
        /// Index into StreamManager's receiver list
        receiver_idx: usize,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
    /// DASH stream input (reads shared texture from StreamManager)
    Dash {
        /// Index into StreamManager's receiver list
        receiver_idx: usize,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
}

/// An effect in the deck's effect chain (ISF filter)
pub struct Effect {
    /// Stable UUID for this effect (8-char hex)
    pub uuid: String,
    pub shader: ISFShader,
    pub pipeline: UnifiedPipeline,
    pub enabled: bool,
    pub params: ShaderParams,
    pub pass_buffers: HashMap<String, PassBuffer>,
    pub passes: Vec<ISFPass>,
    pub target_format: wgpu::TextureFormat,
    /// GPU textures loaded from ISF IMPORTED images (sorted by name for deterministic binding)
    pub imported_textures: Vec<(String, wgpu::Texture, wgpu::TextureView)>,
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

    /// Start time for TIME uniform
    start_time: Instant,

    /// Frame counter
    frame_count: u32,

    /// Last frame time
    last_frame_time: Instant,

    /// Camera source texture view (set each frame for Camera decks, cloned from CameraManager)
    pub camera_source_view: Option<wgpu::TextureView>,

    /// NDI source texture view (set each frame for NDI decks, cloned from NdiManager)
    pub ndi_source_view: Option<wgpu::TextureView>,

    /// Syphon source texture view (set each frame for Syphon decks, cloned from SyphonManager)
    pub syphon_source_view: Option<wgpu::TextureView>,

    /// SRT source texture view (set each frame for SRT decks, cloned from StreamManager)
    pub srt_source_view: Option<wgpu::TextureView>,

    /// Smoothed FPS derived from actual render pipeline timing (EMA of 1/time_delta)
    fps_smoothed: f32,

    /// Phase accumulators for smooth speed transitions (generator shader)
    phase_accumulators: [f32; 4],

    /// Phase input config from generator shader metadata
    generator_phase_inputs: Option<Vec<crate::isf::PhaseInput>>,
}

/// Accessors for Deck properties.
/// Constructors are in source.rs, rendering in render.rs.
impl Deck {
    /// Get the stable UUID for this deck
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    /// Set the UUID (used during scene restore to preserve identity)
    pub fn set_uuid(&mut self, uuid: String) {
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
            DeckSource::Camera { .. } => "camera",
            DeckSource::Ndi { .. } => "ndi",
            DeckSource::Syphon { .. } => "syphon",
            DeckSource::Srt { .. } => "srt",
            DeckSource::Hls { .. } => "hls",
            DeckSource::Dash { .. } => "dash",
        }
    }

    /// Get a reference to the video playback state, if this is a video deck.
    pub fn playback_state(&self) -> Option<&crate::video::PlaybackState> {
        match &self.source {
            DeckSource::Video { player, .. } => Some(&player.playback),
            DeckSource::HapVideo { player, .. } => Some(&player.playback),
            _ => None,
        }
    }

    /// Get a mutable reference to the video playback state, if this is a video deck.
    pub fn playback_state_mut(&mut self) -> Option<&mut crate::video::PlaybackState> {
        match &mut self.source {
            DeckSource::Video { player, .. } => Some(&mut player.playback),
            DeckSource::HapVideo { player, .. } => Some(&mut player.playback),
            _ => None,
        }
    }

    /// Seek the video to a specific position in seconds (resets cache for ping-pong).
    pub fn video_seek(&mut self, time_secs: f64) -> anyhow::Result<()> {
        match &mut self.source {
            DeckSource::Video { player, .. } => player.seek_and_reset(time_secs),
            DeckSource::HapVideo { player, .. } => player.seek(time_secs),
            _ => Ok(()),
        }
    }

    /// Get the solid color value (if source is a solid color)
    pub fn solid_color(&self) -> Option<[f32; 4]> {
        match &self.source {
            DeckSource::SolidColor { color } => Some([color[0] as f32, color[1] as f32, color[2] as f32, color[3] as f32]),
            _ => None,
        }
    }

    /// Set the solid color value (only applies to SolidColor sources)
    pub fn set_solid_color(&mut self, new_color: [f32; 4]) {
        if let DeckSource::SolidColor { color } = &mut self.source {
            *color = [new_color[0] as f64, new_color[1] as f64, new_color[2] as f64, new_color[3] as f64];
        }
    }

    /// Get the scaling mode (if applicable for this source type)
    pub fn scaling_mode(&self) -> Option<ScalingMode> {
        match &self.source {
            DeckSource::Image { scaling_mode, .. }
            | DeckSource::Camera { scaling_mode, .. }
            | DeckSource::Ndi { scaling_mode, .. }
            | DeckSource::Syphon { scaling_mode, .. }
            | DeckSource::Srt { scaling_mode, .. }
            | DeckSource::Hls { scaling_mode, .. }
            | DeckSource::Dash { scaling_mode, .. } => Some(*scaling_mode),
            _ => None,
        }
    }

    /// Set the scaling mode (applies to Image, Camera, NDI, and Syphon sources)
    pub fn set_scaling_mode(&mut self, mode: ScalingMode) {
        match &mut self.source {
            DeckSource::Image { scaling_mode, .. }
            | DeckSource::Camera { scaling_mode, .. }
            | DeckSource::Ndi { scaling_mode, .. }
            | DeckSource::Syphon { scaling_mode, .. }
            | DeckSource::Srt { scaling_mode, .. }
            | DeckSource::Hls { scaling_mode, .. }
            | DeckSource::Dash { scaling_mode, .. } => *scaling_mode = mode,
            _ => {}
        }
    }

    /// Get the NDI receiver index (if source is NDI)
    pub fn ndi_receiver_idx(&self) -> Option<usize> {
        match &self.source {
            DeckSource::Ndi { receiver_idx, .. } => Some(*receiver_idx),
            _ => None,
        }
    }

    /// Get the Syphon client index (if source is Syphon)
    pub fn syphon_client_idx(&self) -> Option<usize> {
        match &self.source {
            DeckSource::Syphon { client_idx, .. } => Some(*client_idx),
            _ => None,
        }
    }

    /// Get the SRT/HLS/DASH receiver index (if source is a stream)
    pub fn srt_receiver_idx(&self) -> Option<usize> {
        match &self.source {
            DeckSource::Srt { receiver_idx, .. }
            | DeckSource::Hls { receiver_idx, .. }
            | DeckSource::Dash { receiver_idx, .. } => Some(*receiver_idx),
            _ => None,
        }
    }

    /// Get the camera ID (if source is a camera)
    pub fn camera_id(&self) -> Option<crate::camera::CameraId> {
        match &self.source {
            DeckSource::Camera { camera_id, .. } => Some(*camera_id),
            _ => None,
        }
    }

    /// Get the shader (if source is a shader)
    pub fn shader(&self) -> Option<&ISFShader> {
        match &self.source {
            DeckSource::Shader { shader, .. } => Some(shader),
            _ => None,
        }
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
        assert!((scale[0] - 0.5).abs() < 1e-5, "scale_x should be 0.5, got {}", scale[0]);
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!((offset[0] - 0.25).abs() < 1e-5, "offset_x should center crop");
        assert!((offset[1]).abs() < 1e-5);
    }

    #[test]
    fn fill_tall_source_crops_vertical() {
        // Source 1:2, target 1:1 → crop top/bottom
        let (scale, offset) = ScalingMode::Fill.compute_uv_transform(100, 200, 100, 100);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 0.5).abs() < 1e-5, "scale_y should be 0.5, got {}", scale[1]);
        assert!((offset[0]).abs() < 1e-5);
        assert!((offset[1] - 0.25).abs() < 1e-5, "offset_y should center crop");
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
