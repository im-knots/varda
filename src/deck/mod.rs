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

/// Scaling mode for non-shader sources (images, video)
#[derive(Debug, Clone, Copy, PartialEq)]
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
}

/// An effect in the deck's effect chain (ISF filter)
pub struct Effect {
    pub shader: ISFShader,
    pub pipeline: UnifiedPipeline,
    pub enabled: bool,
    pub params: ShaderParams,
    pub pass_buffers: HashMap<String, PassBuffer>,
    pub passes: Vec<ISFPass>,
    pub target_format: wgpu::TextureFormat,
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
}

/// Accessors for Deck properties.
/// Constructors are in source.rs, rendering in render.rs.
impl Deck {
    /// Get the source name (shader name, video filename, etc.)
    pub fn source_name(&self) -> &str {
        &self.source_name
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

    /// Get the scaling mode (if applicable for this source type)
    pub fn scaling_mode(&self) -> Option<ScalingMode> {
        match &self.source {
            DeckSource::Image { scaling_mode, .. } => Some(*scaling_mode),
            DeckSource::Camera { scaling_mode, .. } => Some(*scaling_mode),
            _ => None,
        }
    }

    /// Set the scaling mode (applies to Image and Camera sources)
    pub fn set_scaling_mode(&mut self, mode: ScalingMode) {
        match &mut self.source {
            DeckSource::Image { scaling_mode, .. } => *scaling_mode = mode,
            DeckSource::Camera { scaling_mode, .. } => *scaling_mode = mode,
            _ => {}
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
}
