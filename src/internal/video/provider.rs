//! Video files as a deck source: HAP on the GPU-native path, anything else
//! through ffmpeg. See /spec/deck-sources.md § 2.

use super::modulation as vm;
use super::staging::VideoStagingBuffers;
use super::{
    DeckTransportSync, HapTextureFormat, LoopMode, PlaybackSnapshot, TransportSyncMode,
    VideoChaseBroadcast, VideoCommand, VideoDecodeHandle, VideoPlayer, hap::HapPlayer,
};
use crate::renderer::{GpuContext, HapConvertPipeline};
use crate::source::{
    AlphaPolicy, ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance,
    DeckSourceProvider, LibraryCreate, LibrarySection, ScaledBlit, ScalingMode, SourceConfig,
    SourceControl, SourceEnv, SourceFrame, SourceLoader, SourceQuery, WidgetHint, choice_index,
    choice_value, decode_config, encode_config, expect_norm, scaling_mode_spec,
};
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "Video";

/// File extensions the video source opens.
pub const EXTENSIONS: &[&str] = &["mov", "mp4", "avi", "mkv", "webm", "gif"];

const LOOP_MODES: [LoopMode; 4] = [
    LoopMode::Loop,
    LoopMode::PingPong,
    LoopMode::OneShot,
    LoopMode::HoldLast,
];
const SYNC_MODES: [TransportSyncMode; 3] = [
    TransportSyncMode::Auto,
    TransportSyncMode::Always,
    TransportSyncMode::Never,
];

/// Playback controls are deck built-ins on the router (`deck/<uuid>/video/…`),
/// modulatable where a continuous or discrete value makes sense. In and out
/// points are not: they are the reference a position offset is scaled against,
/// so modulating them would feed back. See /spec/video-playback-modulation.md.
static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        ControlSpec::toggle("play", "Play")
            .routed(vm::PLAY)
            .modulatable()
            .in_widget(WidgetHint::Transport),
        ControlSpec::float("position", "Position", 0.0, 1.0)
            .routed(vm::POSITION)
            .modulatable()
            .in_widget(WidgetHint::Transport),
        ControlSpec::float("speed", "Speed", vm::SPEED_MIN as f32, vm::SPEED_MAX as f32)
            .unit("x")
            .routed(vm::SPEED)
            .modulatable()
            .in_widget(WidgetHint::Transport),
        ControlSpec::choice(
            "loop_mode",
            "Loop",
            &["Loop", "Ping-Pong", "One Shot", "Hold Last"],
        )
        .routed(vm::LOOP_MODE)
        .modulatable()
        .in_widget(WidgetHint::Transport),
        ControlSpec::float("in_point", "In", 0.0, 1.0)
            .routed("video/in_point")
            .in_widget(WidgetHint::Transport),
        ControlSpec::float("out_point", "Out", 0.0, 1.0)
            .routed("video/out_point")
            .in_widget(WidgetHint::Transport),
        ControlSpec::action("clear", "Clear In/Out")
            .routed("video/clear")
            .in_widget(WidgetHint::Transport),
        ControlSpec::choice("chase", "Chase", &["Auto", "Always", "Never"])
            .in_widget(WidgetHint::Transport),
        ControlSpec::number("chase_offset", "Offset", "s", 0.01).in_widget(WidgetHint::Transport),
        ControlSpec::number("chase_delay", "Delay", "f", 1.0).in_widget(WidgetHint::Transport),
        scaling_mode_spec(),
    ]
});

fn default_speed() -> f64 {
    1.0
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    path: String,
    #[serde(default)]
    loop_mode: LoopMode,
    #[serde(default = "default_speed")]
    speed: f64,
    /// Seconds. 0 = start.
    #[serde(default)]
    in_point: f64,
    /// Seconds. 0 = end of file.
    #[serde(default)]
    out_point: f64,
    #[serde(default)]
    scaling_mode: ScalingMode,
    /// Mapping onto the show transport. Default Auto: chase while the
    /// transport is running. See /spec/timecode.md § Consumer 2.
    #[serde(default)]
    transport_sync: DeckTransportSync,
}

/// Map a normalized value to a playback speed multiplier.
pub fn speed_from_norm(value: f32) -> f64 {
    vm::SPEED_MIN + f64::from(value.clamp(0.0, 1.0)) * (vm::SPEED_MAX - vm::SPEED_MIN)
}

/// Inverse of [`speed_from_norm`].
pub fn speed_to_norm(speed: f64) -> f32 {
    ((speed - vm::SPEED_MIN) / (vm::SPEED_MAX - vm::SPEED_MIN)).clamp(0.0, 1.0) as f32
}

/// A normalized value scaled to seconds against a clip's duration.
pub fn secs_from_norm(value: f32, duration: f64) -> f64 {
    f64::from(value.clamp(0.0, 1.0)) * duration.max(0.0)
}

/// Inverse of [`secs_from_norm`]. A clip of unknown length has no meaningful
/// normalized position, so it reports the start rather than dividing by zero.
pub fn secs_to_norm(secs: f64, duration: f64) -> f32 {
    if duration <= 0.0 {
        return 0.0;
    }
    (secs / duration).clamp(0.0, 1.0) as f32
}

/// Video files.
pub struct VideoProvider;

impl DeckSourceProvider for VideoProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Video"
    }

    fn icon(&self) -> &'static str {
        "🎬"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn library(&self, _query: &SourceQuery) -> LibrarySection {
        LibrarySection {
            create: Some(LibraryCreate::File {
                field: "path".into(),
                extensions: EXTENSIONS.iter().map(|e| (*e).to_string()).collect(),
                label: "Load video files as deck sources".into(),
            }),
            ..LibrarySection::default()
        }
    }

    fn loader(&self, config: &SourceConfig, _query: &SourceQuery) -> Option<Result<SourceLoader>> {
        Some(decode_config::<Config>(config).and_then(|config| {
            anyhow::ensure!(
                Path::new(&config.path).is_file(),
                "Video file not found: {}",
                config.path
            );
            let loader: SourceLoader = Box::new(move |gpu, _width, _height| {
                let mut video = Video::open(gpu, &config.path)?;
                video.apply(&config);
                video.blit.scaling_mode = config.scaling_mode;
                Ok(Box::new(video) as Box<dyn DeckSourceInstance>)
            });
            Ok(loader)
        }))
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let loader = self
            .loader(config, &env.query())
            .context("video source has a loader")??;
        loader(env.gpu, env.width, env.height)
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("path").cloned().unwrap_or_default()
    }
}

/// How the decoded frames reach the GPU.
#[allow(
    clippy::large_enum_variant,
    reason = "one per video deck, built once and never moved in bulk"
)]
enum Upload {
    /// ffmpeg CPU decode to RGBA, blitted.
    Rgba {
        view: wgpu::TextureView,
        texture: wgpu::Texture,
        staging: VideoStagingBuffers,
    },
    /// HAP: compressed `BCn` blocks uploaded as-is and converted on the GPU.
    Hap {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        alpha_texture: Option<wgpu::Texture>,
        alpha_view: Option<wgpu::TextureView>,
        dummy_alpha_view: wgpu::TextureView,
        convert: HapConvertPipeline,
        format: HapTextureFormat,
        staging: VideoStagingBuffers,
        alpha_staging: Option<VideoStagingBuffers>,
    },
}

/// One video deck.
pub struct Video {
    path: String,
    handle: VideoDecodeHandle,
    upload: Upload,
    /// The RGBA path's blit, and the scaling mode and clip size for both paths.
    blit: ScaledBlit,
}

fn bc_texture(
    gpu: &GpuContext,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

impl Video {
    /// Open a clip. Detects HAP and takes the GPU-native path when the device
    /// supports compressed textures.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened as a HAP or generic video stream,
    /// or a pipeline cannot be created.
    pub fn open(gpu: &GpuContext, path: &str) -> Result<Self> {
        let has_bc = gpu
            .device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let hap_format = if has_bc {
            super::detect_hap_codec(path).ok().flatten()
        } else {
            None
        };

        let (handle, upload, size) = if let Some(format) = hap_format {
            let player = HapPlayer::new(path, format)?;
            let (w, h) = (player.width(), player.height());
            let (texture, view) = bc_texture(gpu, "HAP Video Texture", w, h, format.wgpu_format());
            let with_alpha = matches!(format, HapTextureFormat::Bc3YCoCg);
            let (alpha_texture, alpha_view) = if with_alpha {
                let (t, v) = bc_texture(
                    gpu,
                    "HAP Alpha Texture",
                    w,
                    h,
                    wgpu::TextureFormat::Bc4RUnorm,
                );
                (Some(t), Some(v))
            } else {
                (None, None)
            };
            let (dummy, dummy_alpha_view) =
                bc_texture(gpu, "HAP Dummy Alpha", 1, 1, wgpu::TextureFormat::R8Unorm);
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &dummy,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[255u8],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(1),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let convert = HapConvertPipeline::new(&gpu.device, gpu.compositing_format)?;
            log::info!("Using HAP GPU path for '{path}' ({format:?})");

            let blocks_x = w.div_ceil(4);
            let blocks_y = h.div_ceil(4);
            let staging = VideoStagingBuffers::new(
                &gpu.device,
                blocks_x * format.block_bytes(),
                blocks_y,
                "HAP Color",
            );
            let alpha_staging = with_alpha.then(|| {
                VideoStagingBuffers::new(
                    &gpu.device,
                    blocks_x * HapTextureFormat::Bc4.block_bytes(),
                    blocks_y,
                    "HAP Alpha",
                )
            });
            (
                VideoDecodeHandle::spawn_hap(player),
                Upload::Hap {
                    texture,
                    view,
                    alpha_texture,
                    alpha_view,
                    dummy_alpha_view,
                    convert,
                    format,
                    staging,
                    alpha_staging,
                },
                (w, h),
            )
        } else {
            let player = VideoPlayer::new(path)?;
            let (w, h) = (player.width(), player.height());
            let (texture, view) = bc_texture(
                gpu,
                "Video Frame Texture",
                w,
                h,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            );
            let staging = VideoStagingBuffers::new(&gpu.device, w * 4, h, "Video");
            (
                VideoDecodeHandle::spawn_video(player),
                Upload::Rgba {
                    view,
                    texture,
                    staging,
                },
                (w, h),
            )
        };

        Ok(Self {
            path: path.to_string(),
            handle,
            upload,
            blit: ScaledBlit::new(gpu, AlphaPolicy::Verbatim, "Video Blit Pass", size)?,
        })
    }

    /// Take a saved config's playback settings.
    fn apply(&self, config: &Config) {
        self.handle
            .send(VideoCommand::SetLoopMode(config.loop_mode));
        self.handle.send(VideoCommand::SetSpeed(config.speed));
        self.handle.send(VideoCommand::SetInPoint(config.in_point));
        self.handle
            .send(VideoCommand::SetOutPoint(config.out_point));
        self.handle.set_transport_sync(config.transport_sync);
    }

    /// A config for a deck of the clip at `path`.
    pub fn config_for(path: &str) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                path: path.to_string(),
                loop_mode: LoopMode::default(),
                speed: 1.0,
                in_point: 0.0,
                out_point: 0.0,
                scaling_mode: ScalingMode::default(),
                transport_sync: DeckTransportSync::default(),
            },
        )
    }

    /// The decoder's current state.
    pub fn playback(&self) -> PlaybackSnapshot {
        self.handle.playback_snapshot()
    }

    /// Let modulation drive playback.
    ///
    /// Speed and playhead go to the decode thread as levels, because they are
    /// continuous and only the newest value matters. Play and loop mode are
    /// discrete, so they are written only when the option the modulator points
    /// at differs from the one in force, which keeps a settled modulator from
    /// generating any cross-thread traffic at all.
    fn apply_modulation(&self, ctx: &mut SourceControl) {
        if !ctx.modulation.has_modulation_for_any() {
            return;
        }
        let snap = self.handle.playback_snapshot();
        // The servo owns the timeline while chasing, so neither rate nor
        // playhead is a modulator's to hold there. See
        // /spec/video-playback-modulation.md § Authority.
        let chasing = self
            .handle
            .transport_sync()
            .mode
            .is_chasing(ctx.transport_running());
        let speed = if chasing {
            None
        } else {
            ctx.resolve(vm::SPEED)
                .map(|r| vm::effective_speed(snap.speed, &r))
        };
        let position = if chasing {
            vm::PositionTarget::Free
        } else {
            ctx.resolve(vm::POSITION)
                .map_or(vm::PositionTarget::Free, |r| {
                    vm::position_target(&r, snap.in_point, snap.effective_out(), snap.duration)
                })
        };
        // A sleeping clip is frozen rather than played on silently, so nothing
        // may drive it: the same show position has to look the same on the
        // second run. See /spec/deck-residency.md.
        self.handle.publish_modulation(if ctx.awake {
            vm::PlaybackModulation { speed, position }
        } else {
            vm::PlaybackModulation::default()
        });
        if ctx.awake {
            if let Some(r) = ctx.resolve(vm::PLAY) {
                let want = vm::play_gate(&r, snap.playing);
                if want != snap.playing {
                    self.set_playing(want);
                }
            }
            if let Some(r) = ctx.resolve(vm::LOOP_MODE) {
                let next = LoopMode::from_value(crate::source::discrete_value(&r));
                if next != snap.loop_mode {
                    self.handle.send(VideoCommand::SetLoopMode(next));
                }
            }
        }
    }

    fn set_playing(&self, playing: bool) {
        self.handle.send(if playing {
            VideoCommand::Play
        } else {
            VideoCommand::Pause
        });
    }

    fn set_sync(&self, f: impl FnOnce(&mut DeckTransportSync)) {
        let mut sync = self.handle.transport_sync();
        f(&mut sync);
        self.handle.set_transport_sync(sync);
    }
}

impl DeckSourceInstance for Video {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        Path::new(&self.path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("video")
            .to_string()
    }

    fn config(&self) -> SourceConfig {
        let pb = self.handle.playback_snapshot();
        encode_config(
            SOURCE_TYPE,
            &Config {
                path: self.path.clone(),
                loop_mode: pb.loop_mode,
                speed: pb.speed,
                in_point: pb.in_point,
                out_point: pb.out_point,
                scaling_mode: self.blit.scaling_mode,
                transport_sync: self.handle.transport_sync(),
            },
        )
    }

    fn render(&mut self, frame: &mut SourceFrame) -> Result<()> {
        match &self.upload {
            Upload::Rgba { view, .. } => self.blit.draw(frame, view),
            Upload::Hap {
                view,
                alpha_view,
                dummy_alpha_view,
                convert,
                format,
                ..
            } => {
                let (sw, sh) = self.blit.source_size;
                let (uv_scale, uv_offset) =
                    self.blit
                        .scaling_mode
                        .compute_uv_transform(sw, sh, frame.width, frame.height);
                let has_alpha = self.handle.is_dual_plane && alpha_view.is_some();
                convert.set_params_with_uv(
                    &frame.gpu.queue,
                    1.0,
                    format.needs_ycocg_convert(),
                    has_alpha,
                    uv_scale,
                    uv_offset,
                );
                let alpha = alpha_view.as_ref().unwrap_or(dummy_alpha_view);
                let bind_group = convert.create_bind_group(&frame.gpu.device, view, alpha);
                let mut encoder =
                    frame
                        .gpu
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("HAP Convert Encoder"),
                        });
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("HAP Convert Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: frame.target,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    convert.draw(&mut pass, &bind_group);
                }
                frame.cmd_buffers.push(encoder.finish());
            }
        }
        Ok(())
    }

    fn patch(&mut self, config: &SourceConfig) {
        if let Ok(config) = config.decode::<Config>() {
            self.apply(&config);
            self.blit.scaling_mode = config.scaling_mode;
        }
    }

    /// Runs every frame: modulation, then the transport a chasing clip
    /// servos against, then residency and the decode rate bound.
    fn control(&mut self, ctx: &mut SourceControl) {
        self.blit.control(ctx);
        self.apply_modulation(ctx);
        let chase = ctx
            .transport
            .map_or_else(VideoChaseBroadcast::default, |t| VideoChaseBroadcast {
                position: t.position,
                running: t.running,
                fps: t.fps,
            });
        self.handle
            .publish_chase(chase, ctx.transport.is_some_and(|t| t.discontinuity));
        self.handle.set_suspended(!ctx.awake);
        if ctx.awake {
            self.handle.set_output_fps(ctx.target_fps);
        }
    }

    /// Upload the newest decoded frame through the double-buffered staging
    /// pair, which avoids the per-frame allocation `queue.write_texture` makes.
    fn upload(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(frame) = self.handle.take_frame() else {
            return;
        };
        let (w, h) = (self.handle.width, self.handle.height);
        match &mut self.upload {
            Upload::Rgba {
                texture, staging, ..
            } => {
                staging.upload(&frame.color_data, texture, w, h, encoder);
            }
            Upload::Hap {
                texture,
                alpha_texture,
                staging,
                alpha_staging,
                ..
            } => {
                staging.upload(&frame.color_data, texture, w, h, encoder);
                if let (Some(alpha), Some(_), Some(alpha_texture), Some(alpha_staging)) = (
                    frame.alpha_data.as_ref(),
                    frame.alpha_format,
                    alpha_texture.as_ref(),
                    alpha_staging.as_mut(),
                ) {
                    alpha_staging.upload(alpha, alpha_texture, w, h, encoder);
                }
            }
        }
        self.handle.recycle(frame);
    }

    fn after_submit(&mut self) {
        match &mut self.upload {
            Upload::Rgba { staging, .. } => staging.request_remap(),
            Upload::Hap {
                staging,
                alpha_staging,
                ..
            } => {
                staging.request_remap();
                if let Some(alpha) = alpha_staging {
                    alpha.request_remap();
                }
            }
        }
    }

    fn schema(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        let pb = || self.handle.playback_snapshot();
        let sync = || self.handle.transport_sync();
        Some(match name {
            "play" => ControlValue::Bool(pb().playing),
            "position" => {
                let pb = pb();
                ControlValue::Float(secs_to_norm(pb.position, pb.duration))
            }
            "speed" => ControlValue::Float(speed_to_norm(pb().speed)),
            "loop_mode" => {
                let mode = pb().loop_mode;
                let index = LOOP_MODES.iter().position(|m| *m == mode).unwrap_or(0);
                ControlValue::Float(choice_value(index, LOOP_MODES.len()))
            }
            "in_point" => {
                let pb = pb();
                ControlValue::Float(secs_to_norm(pb.in_point, pb.duration))
            }
            "out_point" => {
                let pb = pb();
                ControlValue::Float(secs_to_norm(pb.effective_out(), pb.duration))
            }
            "chase" => {
                let mode = sync().mode;
                let index = SYNC_MODES.iter().position(|m| *m == mode).unwrap_or(0);
                ControlValue::Float(choice_value(index, SYNC_MODES.len()))
            }
            "chase_offset" => ControlValue::Float(sync().offset as f32),
            "chase_delay" => ControlValue::Float(sync().delay_frames as f32),
            _ => return self.blit.param(name),
        })
    }

    fn set_param(&mut self, name: &str, value: &ControlValue) -> Result<(), ControlError> {
        match name {
            "play" => self.set_playing(expect_norm(name, value)? > 0.5),
            "position" => {
                let pb = self.handle.playback_snapshot();
                self.handle.send(VideoCommand::Seek(secs_from_norm(
                    expect_norm(name, value)?,
                    pb.duration,
                )));
            }
            "speed" => self
                .handle
                .send(VideoCommand::SetSpeed(speed_from_norm(expect_norm(
                    name, value,
                )?))),
            "loop_mode" => {
                let index = choice_index(expect_norm(name, value)?, LOOP_MODES.len());
                self.handle
                    .send(VideoCommand::SetLoopMode(LOOP_MODES[index]));
            }
            "in_point" => {
                let pb = self.handle.playback_snapshot();
                self.handle.send(VideoCommand::SetInPoint(secs_from_norm(
                    expect_norm(name, value)?,
                    pb.duration,
                )));
            }
            "out_point" => {
                let pb = self.handle.playback_snapshot();
                self.handle.send(VideoCommand::SetOutPoint(secs_from_norm(
                    expect_norm(name, value)?,
                    pb.duration,
                )));
            }
            "chase" => {
                let index = choice_index(expect_norm(name, value)?, SYNC_MODES.len());
                self.set_sync(|s| s.mode = SYNC_MODES[index]);
            }
            "chase_offset" => {
                let v = value
                    .as_f32()
                    .ok_or_else(|| ControlError::Invalid("'chase_offset' takes seconds".into()))?;
                self.set_sync(|s| s.offset = f64::from(v));
            }
            "chase_delay" => {
                let v = value
                    .as_f32()
                    .ok_or_else(|| ControlError::Invalid("'chase_delay' takes frames".into()))?;
                self.set_sync(|s| s.delay_frames = v.round() as i32);
            }
            _ => {
                return self
                    .blit
                    .set_param(name, value)
                    .unwrap_or_else(|| Err(ControlError::Unknown(name.to_string())));
            }
        }
        Ok(())
    }

    fn trigger(&mut self, action: &str) -> Result<(), ControlError> {
        match action {
            "clear" => {
                self.handle.send(VideoCommand::ClearInOutPoints);
                Ok(())
            }
            _ => Err(ControlError::Unknown(action.to_string())),
        }
    }

    fn status(&self) -> ControlStatus {
        let pb = self.handle.playback_snapshot();
        let sync = self.handle.transport_sync();
        let mut status = crate::source::status_from_params(self);
        status
            .display
            .insert("speed".into(), format!("{:.2}x", pb.speed));
        let info = &mut status.info;
        info.insert("playing".into(), pb.playing.into());
        info.insert("position".into(), pb.position.into());
        info.insert("duration".into(), pb.duration.into());
        info.insert("speed".into(), pb.speed.into());
        info.insert("effective_speed".into(), pb.effective_speed.into());
        info.insert("position_offset".into(), pb.position_offset.into());
        info.insert("in_point".into(), pb.in_point.into());
        info.insert("out_point".into(), pb.out_point.into());
        info.insert("frame_rate".into(), pb.frame_rate.into());
        info.insert(
            "loop_mode".into(),
            serde_json::to_value(pb.loop_mode).unwrap_or_default(),
        );
        info.insert(
            "transport_sync".into(),
            serde_json::to_value(sync).unwrap_or_default(),
        );
        status
    }

    fn warning(&self) -> Option<String> {
        self.handle
            .playback_snapshot()
            .pingpong_cache_truncated
            .then(|| {
                "reverse playback truncated (cache full). Transcode to HAP for full-length \
                 reverse."
                    .to_string()
            })
    }

    fn reached_end(&self) -> Option<bool> {
        Some(self.handle.playback_snapshot().reached_end)
    }

    fn is_suspended(&self) -> bool {
        self.handle.is_suspended()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_and_position_round_trip_through_their_scales() {
        for speed in [0.1, 1.0, 2.5, 4.0] {
            assert!((speed_from_norm(speed_to_norm(speed)) - speed).abs() < 1e-5);
        }
        assert!((secs_from_norm(secs_to_norm(12.5, 50.0), 50.0) - 12.5).abs() < 1e-4);
    }

    #[test]
    fn a_clip_of_unknown_length_normalizes_to_the_start() {
        assert_eq!(secs_to_norm(10.0, 0.0), 0.0);
    }

    #[test]
    fn a_saved_video_config_reads_back_unchanged() {
        let json = serde_json::json!({
            "type": "Video",
            "path": "/a.mov",
            "loop_mode": "PingPong",
            "speed": 1.5,
            "in_point": 1.0,
            "out_point": 4.0,
            "scaling_mode": "Fit",
            "transport_sync": {"mode": "Never", "offset": 2.0, "delay_frames": -1}
        });
        let config: SourceConfig = serde_json::from_value(json.clone()).unwrap();
        let decoded: Config = decode_config(&config).unwrap();
        assert_eq!(
            serde_json::to_value(encode_config(SOURCE_TYPE, &decoded)).unwrap(),
            json
        );
    }
}
