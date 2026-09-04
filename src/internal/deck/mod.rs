mod effect;
mod render;
mod source;
pub mod svg;

pub use render::get_current_date;

use crate::isf::{ISFPass, ISFShader};
use crate::params::ShaderParams;
use crate::renderer::{BlitPipeline, ComputePipeline, HapConvertPipeline, UnifiedPipeline};
use crate::video::{HapTextureFormat, PlaybackSnapshot, VideoCommand, VideoDecodeHandle};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Returns (`uv_scale`, `uv_offset`) to transform target UVs to source UVs
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
    /// Bytes per row padded to `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` (256).
    padded_bpr: u32,
    /// Unpadded bytes per row (actual source data stride).
    unpadded_bpr: u32,
    /// Number of rows (height for RGBA, `blocks_y` for compressed).
    rows: u32,
    /// Tracks which buffers need `map_async` after the next `queue.submit()`.
    needs_remap: [bool; 2],
}

impl VideoStagingBuffers {
    /// Create a new double-buffered staging pair.
    /// Buffers start unmapped — call `request_remap()` after the first
    /// `queue.submit()` to begin the mapping lifecycle.
    pub fn new(device: &wgpu::Device, unpadded_bpr: u32, rows: u32, label: &str) -> Self {
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bpr = (unpadded_bpr + align - 1) & !(align - 1);
        let buffer_size = u64::from(padded_bpr) * u64::from(rows);

        let make_buf = |idx: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{label} Staging {idx}")),
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
    ///
    /// # Panics
    ///
    /// Panics if a staging slot marked as mapped no longer exposes its mapped range.
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
            let mut view = buf
                .slice(..)
                .get_mapped_range_mut()
                .expect("upload staging buffer must remain mapped");
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
    /// HAP video playback (GPU-native `BCn`, background decode thread)
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
        /// Vector artwork this texture was rendered from, kept so a resolution
        /// change can re-render it at the new size rather than magnifying
        /// pixels. `None` for raster images, which have nothing to re-render.
        /// Boxed to keep the enum's other variants from paying for the tree.
        svg: Option<Box<usvg::Tree>>,
    },
    /// Solid color fill
    SolidColor { color: [f64; 4] },
    /// External live source (camera, NDI, Syphon, SRT, HLS, DASH, RTMP)
    ExternalSource {
        kind: ExternalSourceKind,
        /// REPLACE blit — writes the source's straight RGBA verbatim. Used when
        /// the deck is flagged `transparent` (preserves source alpha).
        blit_pipeline: BlitPipeline,
        /// `ALPHA_BLENDING` blit over an opaque black clear — flattens the source
        /// to opaque. Used by default (unflagged), so an HTML source with alpha<1
        /// composites over black instead of punching transparent holes.
        blit_pipeline_over_black: BlitPipeline,
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

/// Discriminant for external source types sharing the same `DeckSource` layout.
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
    /// Depth sensor (Kinect/LIDAR). Unlike other external sources, this is
    /// reprojected as a point cloud into the deck texture rather than blitted.
    /// See spec/depth-sensors.md.
    DepthSensor(crate::depth::DepthSensorId),
    /// OS display or application window. See spec/screen-capture.md.
    ScreenCapture(crate::screen_capture::CaptureId),
    /// Varda's own master program or a channel composite, from the previous
    /// frame. The source it points at lives in `Deck::tap`, because a channel
    /// UUID is not `Copy`. See spec/program-tap.md.
    Tap,
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
            Self::DepthSensor(_) => "depth_sensor",
            Self::ScreenCapture(_) => "screen_capture",
            Self::Tap => "tap",
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
            Self::DepthSensor(_) => "Depth Sensor",
            Self::ScreenCapture(_) => "Screen Capture",
            Self::Tap => "Tap",
        }
    }

    /// Depth-sensor id if this is a depth source.
    pub fn depth_sensor_id(&self) -> Option<crate::depth::DepthSensorId> {
        match self {
            Self::DepthSensor(id) => Some(*id),
            _ => None,
        }
    }

    /// Screen-capture id if this is a capture source.
    pub fn screen_capture_id(&self) -> Option<crate::screen_capture::CaptureId> {
        match self {
            Self::ScreenCapture(id) => Some(*id),
            _ => None,
        }
    }
}

/// Live binding for a screen-capture deck.
///
/// The `CaptureId` is a runtime handle and is never persisted; `identity` is the
/// handle-free name a scene stores and rebinds by. See spec/screen-capture.md.
#[derive(Debug, Clone)]
pub struct ScreenCaptureState {
    pub capture_id: crate::screen_capture::CaptureId,
    pub identity: crate::screen_capture::backend::TargetIdentity,
    pub config: crate::screen_capture::backend::CaptureConfig,
    /// Set when the router or UI edits `config`; the render loop pushes the new
    /// config to the manager and clears it. The deck layer never reaches up into
    /// a device, so the change travels down rather than sideways.
    pub config_dirty: bool,
}

/// What a tap deck reads. See spec/program-tap.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapSource {
    /// The master program, tapped before tonemap and LUT so the whole feedback
    /// path stays in linear light and tonemaps exactly once.
    MasterProgram,
    /// A channel composite, addressed by the channel's stable UUID so a tap
    /// survives reordering. See spec/entity-identity.md.
    Channel(String),
}

impl TapSource {
    /// Display name. Channels are resolved through the `(uuid, name)` list the
    /// caller holds; an unresolvable UUID falls back to the UUID itself, and
    /// whether it is genuinely missing is reported separately so a stale label
    /// never masquerades as an error.
    pub fn label(&self, channels: &[(String, String)]) -> String {
        match self {
            Self::MasterProgram => "Master Program".to_string(),
            Self::Channel(uuid) => channels
                .iter()
                .find(|(u, _)| u == uuid)
                .map_or_else(|| format!("Channel {uuid}"), |(_, n)| n.clone()),
        }
    }
}

/// Live binding for a tap deck. There is no runtime handle to hold: the tap
/// resolves to a mixer-owned texture view every frame, so an unresolvable
/// source simply renders black until the channel comes back.
#[derive(Debug, Clone)]
pub struct TapState {
    pub source: TapSource,
}

/// Live state for a deck's `depth_sensor` shader preprocessor.
///
/// The sensor reference is ref-counted on `DepthSensorManager`; it is acquired
/// before the deck is constructed and released when the deck is removed, so any
/// number of preprocessor decks and point-cloud decks can share one device.
pub struct DepthPreprocessState {
    /// The acquired sensor. Held so deck teardown can release the reference.
    pub sensor_id: crate::depth::DepthSensorId,
    /// Device name, captured at acquisition. Persistence matches sensors by name
    /// (ids are not stable across replugs), and snapshotting a scene must not
    /// need the device manager.
    pub sensor_name: String,
    /// Router-exposed params (`deck/<uuid>/depth_prepro/*`).
    pub params: crate::depth::preprocess::DepthPreprocessParams,
    /// The conversion pipeline and its owned output textures.
    pub pipeline: crate::depth::preprocess::DepthPreprocessPipeline,
    /// Whether any consuming shader declared the `rgb` output. When false the
    /// colour pass is skipped entirely.
    pub wants_rgb: bool,
    /// Last sensor frame generation processed, so a 30 Hz sensor does not drive
    /// 60 Hz of redundant passes.
    pub last_generation: Option<u64>,
    /// Set while the sensor reports disconnected, so the warning fires once.
    pub warned_disconnected: bool,
    /// This frame's sensor inputs, pushed by the app render loop (which owns the
    /// manager). `None` before the first tick or while the sensor is gone.
    pub input: Option<DepthPreprocessInput>,
}

/// Per-frame sensor inputs handed to a deck's depth preprocessor by the app layer.
pub struct DepthPreprocessInput {
    /// Shared `R16Uint` depth texture owned by `DepthSensorManager`.
    pub depth_view: wgpu::TextureView,
    /// Shared colour texture owned by `DepthSensorManager`.
    pub rgb_view: wgpu::TextureView,
    /// Manager upload counter, used to skip redundant passes.
    pub generation: u64,
    /// Measured seconds between the last two sensor frames.
    pub frame_dt: f32,
    /// Whether the sensor is currently producing frames.
    pub connected: bool,
}

/// A preprocessor texture slot — holds a GPU texture that gets updated with analyzer output.
pub struct PreprocessorSlot {
    /// Name prefix for shader uniforms (e.g. "depth" → `depth_depth_map`)
    pub name: String,
    /// Analyzer type this preprocessor needs (e.g. "`depth_estimate`")
    pub analyzer_type: String,
    /// Options to pass when starting the analyzer
    pub options: serde_json::Value,
    /// Analyzer value name to live shader parameter name.
    pub param_bindings: HashMap<String, String>,
    /// Analyzer value name to phase-accumulator index.
    pub phase_bindings: HashMap<String, usize>,
    /// GPU texture (initially 1×1 black, updated at runtime)
    pub texture: wgpu::Texture,
    /// Texture view for shader binding
    pub view: wgpu::TextureView,
    /// Format the shader declared for this slot, fixed for the slot's lifetime
    /// because the pipeline layout's filterability is derived from it.
    pub format: wgpu::TextureFormat,
    /// Last non-zero analyzer texture generation uploaded to this slot.
    pub last_uploaded_generation: Option<u64>,
}

/// The texture format a `PREPROCESSORS` entry's `FORMAT` string names.
///
/// `rgba32float` binds non-filterable and carries four raw floats per texel
/// for `texelFetch`-only data payloads; an unknown or absent declaration is
/// the filterable byte-packed default.
pub(crate) fn preprocessor_texture_format(declared: &str) -> wgpu::TextureFormat {
    crate::analyzer::traits::texture_format_from_str(declared)
        .unwrap_or(wgpu::TextureFormat::Rgba8Unorm)
}

/// An effect in the deck's effect chain (ISF filter)
pub struct Effect {
    /// Stable UUID for this effect (8-char hex).
    ///
    /// Private, with [`Effect::set_uuid`] as the only way to change it, because
    /// `param_prefix` is derived from it and the two must never disagree. Scene
    /// restore used to assign the field directly, leaving the prefix pointing at
    /// the throwaway UUID minted by `Effect::new`; modulation assignments are
    /// keyed on the prefix, so every effect modulation silently stopped applying
    /// after a reload while the UI still showed it attached.
    uuid: String,
    /// Cached "fx_{uuid}" prefix for modulation key lookups (avoids per-frame format!)
    param_prefix: String,
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

    /// When true, the deck's base texture preserves source alpha (transparent
    /// letterbox + transparent HTML regions). When false (default), the source is
    /// composited over opaque black, reproducing the historical opaque behavior.
    /// See /spec/html-source.md §2 (Option A).
    transparent: bool,

    /// Accumulated render time for TIME uniform (advances by fixed dt each render).
    /// Decoupled from wall clock so skipped frames don't cause animation jumps.
    render_time: f32,

    /// Fixed time step per render (`1/target_fps`). Updated by the channel when
    /// the deck is rendered, so skipped frames simply don't advance `render_time`.
    render_dt: f32,

    /// Frame counter
    frame_count: u32,

    /// Last wall-clock render instant (for FPS measurement only, not for TIME uniform)
    last_frame_time: Instant,

    /// External source texture view (set each frame for `ExternalSource` decks).
    /// For `DepthSensor` decks this holds the `R16Uint` depth view.
    pub external_source_view: Option<wgpu::TextureView>,

    /// Depth-sensor RGB view (set each frame for `DepthSensor` decks).
    pub depth_rgb_view: Option<wgpu::TextureView>,

    /// Depth-sensor intrinsics (set each frame for `DepthSensor` decks).
    pub depth_intrinsics: Option<crate::depth::backend::DepthIntrinsics>,

    /// Native depth-sensor resolution `(w, h)` (set each frame).
    pub depth_source_size: Option<(u32, u32)>,

    /// Point-cloud reprojection params for `DepthSensor` decks (router-driven).
    pub point_cloud_params: crate::depth::point_cloud::PointCloudParams,

    /// Lazily-built point-cloud pipeline for `DepthSensor` decks.
    point_cloud_pipeline: Option<crate::depth::point_cloud::PointCloudPipeline>,

    /// Depth-sensor shader preprocessor, present when this deck's shader (or one
    /// of its effects) declared a `depth_sensor` PREPROCESSOR and the device was
    /// successfully acquired. See spec/depth-sensor-preprocessor.md.
    pub depth_prepro: Option<DepthPreprocessState>,

    /// Screen-capture binding, present only on `ScreenCapture` decks. Held on
    /// the deck rather than looked up in the manager so `snapshot_scene` can
    /// serialize the target and settings from the mixer alone.
    /// See spec/screen-capture.md § Configuration and Persistence.
    pub screen_capture: Option<ScreenCaptureState>,

    /// Tap binding, present only on `Tap` decks. See spec/program-tap.md.
    pub tap: Option<TapState>,

    /// Smoothed FPS derived from actual render pipeline timing (EMA of `1/time_delta`)
    fps_smoothed: f32,

    /// Phase accumulators for smooth speed transitions (generator shader)
    phase_accumulators: [f32; 4],

    /// Phase input config from generator shader metadata
    generator_phase_inputs: Option<Vec<crate::isf::PhaseInput>>,

    /// Per-deck analyzer instances (brightness, beat detection, etc.)
    pub(crate) analyzers: crate::analyzer::DeckAnalyzers,

    /// Set when this deck raised a GPU error, which quarantines it: it stops
    /// rendering and holds its last good frame instead of aborting the process.
    /// Cleared by [`Deck::clear_gpu_error`] when the shader is reloaded.
    /// See spec/error-handling.md § Shader Errors.
    gpu_error: Option<String>,
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
        self.param_prefix = format!("deck_{uuid}");
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

    /// Bound this deck's decode rate to what the renderer can present
    /// (0 = uncapped). No-op for non-video decks.
    pub fn set_video_output_fps(&self, fps: u32) {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                handle.set_output_fps(fps);
            }
            _ => {}
        }
    }

    /// Stop or resume this deck's decoding, leaving its play/pause state alone.
    /// No-op for non-video decks. See /spec/deck-residency.md.
    pub fn set_video_suspended(&self, suspended: bool) {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                handle.set_suspended(suspended);
            }
            _ => {}
        }
    }

    /// Whether this deck's decoding is currently suspended. False for anything
    /// that is not a video deck, which never suspends.
    pub fn video_is_suspended(&self) -> bool {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                handle.is_suspended()
            }
            _ => false,
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

    /// Map this clip onto the show transport. No-op for non-video decks.
    pub fn video_set_transport_sync(&self, sync: crate::video::DeckTransportSync) -> bool {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                handle.set_transport_sync(sync);
                true
            }
            _ => false,
        }
    }

    pub fn video_transport_sync(&self) -> Option<crate::video::DeckTransportSync> {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                Some(handle.transport_sync())
            }
            _ => None,
        }
    }

    /// Publish this frame's transport so a chasing clip can servo.
    pub fn publish_video_chase(
        &self,
        sample: crate::video::VideoChaseBroadcast,
        discontinuity: bool,
    ) {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                handle.publish_chase(sample, discontinuity);
            }
            _ => {}
        }
    }

    /// Publish this frame's resolved playback modulation so the decode thread
    /// can act on it. No-op for non-video decks.
    /// See /spec/video-playback-modulation.md.
    pub fn publish_video_modulation(&self, value: crate::video::PlaybackModulation) {
        match &self.source {
            DeckSource::Video { handle, .. } | DeckSource::HapVideo { handle, .. } => {
                handle.publish_modulation(value);
            }
            _ => {}
        }
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

    /// Set the solid color value (only applies to `SolidColor` sources)
    pub fn set_solid_color(&mut self, new_color: [f32; 4]) {
        if let DeckSource::SolidColor { color } = &mut self.source {
            *color = [
                f64::from(new_color[0]),
                f64::from(new_color[1]),
                f64::from(new_color[2]),
                f64::from(new_color[3]),
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

    /// Set the scaling mode (applies to Image, Video, `HapVideo`, and `ExternalSource` sources)
    pub fn set_scaling_mode(&mut self, mode: ScalingMode) {
        match &mut self.source {
            DeckSource::Image { scaling_mode, .. }
            | DeckSource::Video { scaling_mode, .. }
            | DeckSource::HapVideo { scaling_mode, .. }
            | DeckSource::ExternalSource { scaling_mode, .. } => *scaling_mode = mode,
            _ => {}
        }
    }

    /// Set a screen-capture parameter from a normalized value (0.0–1.0).
    /// Returns `false` if this deck is not a screen-capture source or `name` is
    /// not a capture parameter. See spec/screen-capture.md § Parameters.
    pub fn set_capture_param(&mut self, name: &str, value: f32) -> bool {
        use crate::screen_capture::backend::{MAX_CAPTURE_RATE, MIN_CAPTURE_RATE};
        let Some(state) = &mut self.screen_capture else {
            return false;
        };
        let v = value.clamp(0.0, 1.0);
        match name {
            "rate" => {
                state.config.rate = MIN_CAPTURE_RATE + v * (MAX_CAPTURE_RATE - MIN_CAPTURE_RATE);
            }
            "crop_x" => state.config.crop.x = v,
            "crop_y" => state.config.crop.y = v,
            "crop_w" => state.config.crop.w = v,
            "crop_h" => state.config.crop.h = v,
            // Bucketed so a MIDI fader can drive it like every other toggle.
            "cursor" => state.config.show_cursor = v > 0.5,
            "exclude_varda" => state.config.exclude_varda = v > 0.5,
            _ => return false,
        }
        state.config = state.config.clone().sanitized();
        state.config_dirty = true;
        true
    }

    /// Set a depth point-cloud parameter from a normalized value (0.0–1.0).
    /// Returns `false` if this deck is not a depth-sensor source. Continuous
    /// params map linearly to their range; `color_mode` buckets into 3 modes.
    /// See spec/depth-sensors.md.
    pub fn set_depth_param(&mut self, name: &str, value: f32) -> bool {
        if !matches!(
            self.external_source_kind(),
            Some(ExternalSourceKind::DepthSensor(_))
        ) {
            return false;
        }
        self.point_cloud_params.set_normalized_param(name, value)
    }

    /// Normalized (`0..1`) value of a point-cloud parameter, for snapshots.
    /// `None` when this deck is not a depth-sensor source, so a consumer cannot
    /// render faders for a deck that has no point cloud.
    pub fn depth_param(&self, name: &str) -> Option<f32> {
        if !matches!(
            self.external_source_kind(),
            Some(ExternalSourceKind::DepthSensor(_))
        ) {
            return None;
        }
        self.point_cloud_params.normalized_param(name)
    }

    /// Set a depth-preprocessor parameter from a normalized value (0.0–1.0).
    /// Returns `false` if this deck has no `depth_sensor` preprocessor.
    /// See spec/depth-sensor-preprocessor.md.
    pub fn set_depth_prepro_param(&mut self, name: &str, value: f32) -> bool {
        self.depth_prepro
            .as_mut()
            .is_some_and(|s| s.params.set_normalized_param(name, value))
    }

    /// Normalized (`0..1`) value of a depth-preprocessor parameter, for snapshots.
    pub fn depth_prepro_param(&self, name: &str) -> Option<f32> {
        self.depth_prepro
            .as_ref()
            .and_then(|s| s.params.normalized_param(name))
    }

    /// Attach an acquired depth sensor's preprocessor to this deck.
    ///
    /// Rebinds every `depth_sensor` preprocessor slot — on the source shader and
    /// on every effect — to the pipeline's owned output textures. The clones are
    /// `Arc`-backed handles to the same GPU resources, so this is a rebind, not a
    /// copy. Called by the app layer after `open_depth_sensor` succeeds, because
    /// device managers live above `internal::deck`.
    pub fn attach_depth_preprocessor(
        &mut self,
        sensor_id: crate::depth::DepthSensorId,
        sensor_name: String,
        pipeline: crate::depth::preprocess::DepthPreprocessPipeline,
        params: crate::depth::preprocess::DepthPreprocessParams,
    ) {
        self.depth_prepro = Some(DepthPreprocessState {
            sensor_id,
            sensor_name,
            params,
            pipeline,
            wants_rgb: false,
            last_generation: None,
            warned_disconnected: false,
            input: None,
        });
        self.rebind_depth_preprocessor_slots();
    }

    /// Point every `depth_sensor` preprocessor slot at the attached pipeline's
    /// outputs, and recompute whether the colour pass is needed.
    ///
    /// Idempotent, and must be re-run after adding an effect that declares the
    /// preprocessor to a deck that already has one attached.
    pub fn rebind_depth_preprocessor_slots(&mut self) {
        use crate::depth::preprocess::{Output, PREPROCESSOR_TYPE};

        // Take the state so the pipeline can be read while `self.source` and
        // `self.effects` are mutably borrowed.
        let Some(mut state) = self.depth_prepro.take() else {
            return;
        };
        let mut wants_rgb = false;
        let mut rebind = |slots: &mut Vec<PreprocessorSlot>| {
            for slot in slots {
                if slot.analyzer_type != PREPROCESSOR_TYPE {
                    continue;
                }
                let Some(output) = Output::from_name(&slot.name) else {
                    log::warn!(
                        "Shader declared unknown depth_sensor output '{}'; leaving it blank",
                        slot.name
                    );
                    continue;
                };
                if output == Output::Rgb {
                    wants_rgb = true;
                }
                if let Some((texture, view)) = state.pipeline.output(output) {
                    slot.texture = texture;
                    slot.view = view;
                }
            }
        };

        if let DeckSource::Shader {
            preprocessor_textures,
            ..
        } = &mut self.source
        {
            rebind(preprocessor_textures);
        }
        for effect in &mut self.effects {
            rebind(&mut effect.preprocessor_textures);
        }

        state.wants_rgb = wants_rgb;
        self.depth_prepro = Some(state);
    }

    /// Whether any slot on this deck still consumes the `depth_sensor`
    /// preprocessor. Used after removing an effect to decide whether the sensor
    /// reference is still needed.
    fn wants_depth_preprocessor(&self) -> bool {
        let ty = crate::depth::preprocess::PREPROCESSOR_TYPE;
        let source_wants = matches!(
            &self.source,
            DeckSource::Shader { preprocessor_textures, .. }
                if preprocessor_textures.iter().any(|s| s.analyzer_type == ty)
        );
        source_wants
            || self.effects.iter().any(|e| {
                e.preprocessor_textures
                    .iter()
                    .any(|s| s.analyzer_type == ty)
            })
    }

    /// Drop the depth preprocessor if nothing on this deck consumes it any more,
    /// returning the sensor ID the caller must release on the manager.
    ///
    /// Called after removing an effect: if that effect was the only consumer,
    /// holding the device open would keep a capture thread and three GPU passes
    /// alive for nothing.
    pub fn detach_depth_preprocessor_if_unused(&mut self) -> Option<crate::depth::DepthSensorId> {
        if self.depth_prepro.is_none() || self.wants_depth_preprocessor() {
            return None;
        }
        self.depth_prepro.take().map(|s| s.sensor_id)
    }

    /// Sensor IDs this deck holds ref-counted references to, for release on
    /// removal. Covers both point-cloud depth sources and shader preprocessors.
    pub fn held_depth_sensors(&self) -> Vec<crate::depth::DepthSensorId> {
        let mut ids = Vec::new();
        if let Some(id) = self
            .external_source_kind()
            .and_then(|k| k.depth_sensor_id())
        {
            ids.push(id);
        }
        if let Some(state) = &self.depth_prepro {
            ids.push(state.sensor_id);
        }
        ids
    }

    /// The GPU error that quarantined this deck, if any.
    pub fn gpu_error(&self) -> Option<&str> {
        self.gpu_error.as_deref()
    }

    /// Lift the quarantine and let the deck render again.
    ///
    /// Called on shader hot-reload: the author has just changed the source, so
    /// the thing that failed may no longer exist. Without this a single bad save
    /// would black out the deck until the app restarts.
    pub fn clear_gpu_error(&mut self) {
        if self.gpu_error.take().is_some() {
            log::info!("Deck '{}': GPU quarantine lifted", self.uuid);
        }
    }

    /// Whether this deck preserves source alpha (transparent compositing).
    pub fn transparent(&self) -> bool {
        self.transparent
    }

    /// Set whether this deck preserves source alpha (transparent compositing).
    pub fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
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

    /// Get the screen-capture ID (if source is a screen capture)
    pub fn screen_capture_id(&self) -> Option<crate::screen_capture::CaptureId> {
        match &self.source {
            DeckSource::ExternalSource {
                kind: ExternalSourceKind::ScreenCapture(id),
                ..
            } => Some(*id),
            _ => None,
        }
    }

    /// Push the live source dimensions of an external source down from its
    /// manager. Sources whose resolution can change mid-session (a screen
    /// capture being cropped, a stream reconnecting at a new size) need this or
    /// the blit keeps letterboxing to a stale aspect ratio.
    pub fn set_external_source_size(&mut self, width: u32, height: u32) {
        if let DeckSource::ExternalSource {
            source_width,
            source_height,
            ..
        } = &mut self.source
        {
            *source_width = width;
            *source_height = height;
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
    /// Called by the channel to keep `render_dt` in sync with the target FPS.
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
