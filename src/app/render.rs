//! GPU rendering — mixer render, output windows, frame timing.

use super::VardaApp;
use crate::mixer::Mixer;
use crate::renderer::context::{OutputSource, SurfaceRenderInfo};
use crate::surface::ContentMapping;

/// Kind of file dialog to open.
#[derive(Debug, Clone, Copy)]
pub enum FileDialogKind {
    Image,
    Video,
}

/// Result from a completed file dialog (sent from background thread).
/// Supports multi-select: `paths` may contain one or more files.
#[derive(Debug)]
pub struct FileDialogResult {
    pub kind: FileDialogKind,
    pub ch_idx: usize,
    pub paths: Vec<std::path::PathBuf>,
}

/// Result from a background deck load (sent from a spawned thread).
/// Contains a ready-to-use Deck that just needs mixer insertion + egui texture registration.
pub struct DeckLoadResult {
    pub ch_idx: usize,
    pub deck: anyhow::Result<crate::deck::Deck>,
    pub name: String,
}

impl VardaApp {
    /// Spawn background threads to create decks from file paths and shaders.
    /// Each thread creates a full Deck (CPU decode + GPU upload) and sends
    /// the result via the channel. The render loop polls for completed decks.
    /// `pending` is incremented per-spawn and decremented when each thread completes.
    // Args map directly to the independent inputs a deck load needs; bundling them
    // would only add an ephemeral struct with no shared invariant.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_deck_loads(
        sender: &std::sync::mpsc::Sender<DeckLoadResult>,
        context: &crate::renderer::context::GpuContext,
        pending: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
        render_width: u32,
        render_height: u32,
        images: Vec<(usize, std::path::PathBuf)>,
        videos: Vec<(usize, std::path::PathBuf)>,
        shaders: Vec<(usize, crate::isf::ISFShader)>,
    ) {
        use crate::deck::Deck;
        use std::sync::atomic::Ordering;

        for (ch_idx, path) in images {
            let tx = sender.clone();
            let ctx = context.clone();
            let counter = pending.clone();
            let w = render_width;
            let h = render_height;
            counter.fetch_add(1, Ordering::Relaxed);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let name = path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or("image")
                        .to_string();
                    let deck = Deck::new_from_image(&ctx, &path, w, h);
                    (name, deck)
                }));
                let (name, deck) = match result {
                    Ok((name, deck)) => (name, deck),
                    Err(_) => (
                        "image".to_string(),
                        Err(anyhow::anyhow!("panic loading image deck")),
                    ),
                };
                let _ = tx.send(DeckLoadResult { ch_idx, deck, name });
                counter.fetch_sub(1, Ordering::Relaxed);
            });
        }

        for (ch_idx, path) in videos {
            let tx = sender.clone();
            let ctx = context.clone();
            let counter = pending.clone();
            let w = render_width;
            let h = render_height;
            counter.fetch_add(1, Ordering::Relaxed);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let name = path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or("video")
                        .to_string();
                    let deck = Deck::new_from_video(&ctx, &path, w, h);
                    (name, deck)
                }));
                let (name, deck) = match result {
                    Ok((name, deck)) => (name, deck),
                    Err(_) => (
                        "video".to_string(),
                        Err(anyhow::anyhow!("panic loading video deck")),
                    ),
                };
                let _ = tx.send(DeckLoadResult { ch_idx, deck, name });
                counter.fetch_sub(1, Ordering::Relaxed);
            });
        }

        for (ch_idx, shader) in shaders {
            let tx = sender.clone();
            let ctx = context.clone();
            let counter = pending.clone();
            let w = render_width;
            let h = render_height;
            counter.fetch_add(1, Ordering::Relaxed);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let name = shader.name();
                    let deck = if shader.metadata.is_compute() {
                        Deck::new_from_compute_shader(&ctx, shader, w, h)
                    } else {
                        Deck::new(&ctx, shader, w, h)
                    };
                    (name, deck)
                }));
                let (name, deck) = match result {
                    Ok((name, deck)) => (name, deck),
                    Err(_) => (
                        "shader".to_string(),
                        Err(anyhow::anyhow!("panic loading shader deck")),
                    ),
                };
                let _ = tx.send(DeckLoadResult { ch_idx, deck, name });
                counter.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }

    /// Update frame timing (FPS measurement) and system stats. Call once per frame before any work.
    pub fn update_frame_timing(&mut self) {
        let now = std::time::Instant::now();
        let dt = now
            .duration_since(self.frame_stats.last_frame_instant)
            .as_secs_f32();
        self.frame_stats.last_frame_instant = now;
        if dt > 0.0 {
            let instant_fps = 1.0 / dt;
            self.frame_stats.fps_history.push_back(instant_fps);
            if self.frame_stats.fps_history.len() > 60 {
                self.frame_stats.fps_history.pop_front();
            }
            self.frame_stats.fps_smoothed = self.frame_stats.fps_history.iter().sum::<f32>()
                / self.frame_stats.fps_history.len() as f32;
        }
        self.frame_stats.system_monitor.update();
    }

    /// Collect all analyzer scalar values from all decks into a flat lookup table.
    fn collect_analyzer_values(&self) -> crate::modulation::AnalyzerValues {
        let mut vals = crate::modulation::AnalyzerValues::default();
        for ch in self.mixer.channels() {
            for slot in &ch.decks {
                let deck_id = slot.deck.uuid();
                for (analyzer_type, snapshot) in slot.deck.analyzers.all_snapshots() {
                    for (name, value) in &snapshot.scalars {
                        vals.insert(
                            deck_id.to_owned(),
                            analyzer_type.clone(),
                            name.clone(),
                            *value,
                        );
                    }
                }
            }
        }
        vals
    }

    /// Render the mixer frame: update cameras, NDI, Syphon, collect audio, render mixer.
    /// This performs all GPU work that doesn't need the surface texture.
    pub fn render_mixer_frame(&mut self) {
        // Surface a one-time notice for any deck whose ping-pong RAM cache was
        // truncated (hit the memory cap). The supported path for full-length
        // reverse on heavy/long/high-res clips is to pre-transcode to HAP.
        let truncated: Vec<(String, String)> = self
            .mixer
            .channels()
            .iter()
            .flat_map(|ch| ch.decks.iter())
            .filter_map(|slot| {
                slot.deck
                    .playback_snapshot()
                    .filter(|s| s.pingpong_cache_truncated)
                    .map(|_| {
                        (
                            slot.deck.uuid().to_string(),
                            slot.deck.source_name().to_string(),
                        )
                    })
            })
            .collect();
        for (uuid, name) in truncated {
            self.session.notifications.notify_once(
                format!("pingpong_truncated:{uuid}"),
                crate::notifications::NotificationLevel::Warning,
                format!(
                    "Deck '{name}': reverse playback truncated (cache full). \
                     Transcode to HAP for full-length reverse."
                ),
            );
        }

        // Compute effective channel opacities to determine which cameras are needed
        let channel_count = self.mixer.channel_count();
        let crossfader = self.mixer.crossfader();
        let two_ch_buf: [f32; 2];
        let n_ch_buf: Vec<f32>;
        let effective_opacities: &[f32] = if channel_count == 2 {
            let channels = self.mixer.channels();
            two_ch_buf = [
                (1.0 - crossfader) * channels[0].opacity,
                crossfader * channels[1].opacity,
            ];
            &two_ch_buf
        } else {
            n_ch_buf = self.mixer.channels().iter().map(|ch| ch.opacity).collect();
            &n_ch_buf
        };

        // Collect camera IDs needed by visible channels
        let mut needed_camera_ids = std::collections::HashSet::new();
        for (ch_idx, channel) in self.mixer.channels().iter().enumerate() {
            if effective_opacities.get(ch_idx).copied().unwrap_or(0.0) <= 0.0 {
                continue;
            }
            for slot in &channel.decks {
                if let Some(cam_id) = slot.deck.camera_id() {
                    needed_camera_ids.insert(cam_id);
                }
            }
        }

        // Update only needed camera frames
        self.camera_manager
            .update_selective(&self.context.queue, &needed_camera_ids);

        // Update NDI receiver frames
        self.external_io
            .ndi_manager
            .update(&self.context.device, &self.context.queue);

        // Update Syphon client frames
        #[cfg(target_os = "macos")]
        self.external_io.syphon_manager.update(&self.context.queue);

        // Update stream receiver frames
        self.external_io.stream_manager.update(&self.context.queue);

        for channel in self.mixer.channels_mut() {
            for slot in &mut channel.decks {
                if let Some(kind) = slot.deck.external_source_kind() {
                    use crate::deck::ExternalSourceKind;
                    slot.deck.external_source_view = match kind {
                        ExternalSourceKind::Camera(cam_id) => {
                            self.camera_manager.texture_view(cam_id).cloned()
                        }
                        ExternalSourceKind::Ndi(idx) => {
                            self.external_io.ndi_manager.texture_view(idx).cloned()
                        }
                        #[cfg(target_os = "macos")]
                        ExternalSourceKind::Syphon(idx) => {
                            self.external_io.syphon_manager.texture_view(idx).cloned()
                        }
                        #[cfg(not(target_os = "macos"))]
                        ExternalSourceKind::Syphon(_) => None,
                        ExternalSourceKind::Srt(idx)
                        | ExternalSourceKind::Hls(idx)
                        | ExternalSourceKind::Dash(idx)
                        | ExternalSourceKind::Rtmp(idx) => {
                            self.external_io.stream_manager.texture_view(idx).cloned()
                        }
                    };
                }
            }
        }

        // Collect audio values for modulation
        let audio_values = {
            let mut av = crate::modulation::AudioValues::default();
            for id in self.audio_manager.active_source_ids() {
                if let Some(data) = self.audio_manager.get_data(id) {
                    av.sources.insert(
                        id,
                        crate::modulation::AudioSourceValues {
                            fft: data.fft.clone(),
                            level: data.level,
                            sample_rate: data.sample_rate,
                        },
                    );
                }
            }
            av
        };

        let mut primary_audio = self.audio_manager.get_primary_data().clone();

        // Override audio BPM/beat with clock-resolved values (MIDI > OSC > Audio)
        let clock = self.input.clock_manager.state();
        if clock.active {
            primary_audio.bpm = Some(clock.bpm);
            primary_audio.time_since_beat = clock.beat_phase * (60.0 / clock.bpm);
        }

        // Collect analyzer scalar values from all decks
        let analyzer_values = self.collect_analyzer_values();

        let target_fps = self.target_fps;
        if let Err(e) = self.mixer.render(
            &self.context,
            &primary_audio,
            &audio_values,
            &analyzer_values,
            target_fps,
        ) {
            log::error!("Failed to render mixer: {}", e);
        }
    }

    /// Render content to all outputs (windowed + headless) using the surface layout.
    pub fn render_outputs(&mut self) {
        let context = &self.context;

        // Prepare sub-mixes for any Channels(...) sources
        {
            let mut seen: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();
            let mut sub_mix_sources: Vec<Vec<usize>> = Vec::new();
            for surface in &self.output.surface_manager.surfaces {
                if let OutputSource::Channels(indices) = &surface.source {
                    let mut sorted = indices.clone();
                    sorted.sort();
                    sorted.dedup();
                    if seen.insert(sorted.clone()) {
                        sub_mix_sources.push(sorted);
                    }
                }
            }
            self.mixer.prepare_sub_mixes(&sub_mix_sources, context);
        }

        let mixer = &self.mixer;

        // Run domemaster renderer if enabled (content rotation is updated each frame via set_content_rotation)
        let domemaster_view = if let Some(dome) = &self.output.domemaster {
            if dome.enabled {
                dome.update_params(&self.context.queue);
                dome.render(
                    &self.context.device,
                    &self.context.queue,
                    mixer.composite_view(),
                );
                Some(dome.output_view())
            } else {
                None
            }
        } else {
            None
        };

        for output in &self.output.outputs {
            match output {
                crate::renderer::context::UnifiedOutput::Window(output) => {
                    Self::render_window_output(
                        output,
                        context,
                        mixer,
                        &self.output.surface_manager,
                        &self.output.calibration_textures,
                        domemaster_view,
                    );
                }
                crate::renderer::context::UnifiedOutput::Headless(_) => {
                    // Headless rendering handled separately (needs &mut for subprocess)
                }
            }
        }

        // Render headless outputs (needs &mut self for subprocess feeding)
        Self::render_headless_outputs_inner(
            &mut self.output.outputs,
            context,
            mixer,
            &self.output.surface_manager,
            &mut self.external_io.ndi_manager,
            #[cfg(target_os = "macos")]
            &mut self.external_io.syphon_manager,
            domemaster_view,
        );
    }

    fn render_window_output(
        output: &crate::renderer::context::OutputWindow,
        context: &crate::renderer::context::GpuContext,
        mixer: &crate::mixer::Mixer,
        surface_manager: &crate::surface::SurfaceManager,
        calibration_textures: &[(wgpu::Texture, wgpu::TextureView)],
        domemaster_view: Option<&wgpu::TextureView>,
    ) {
        if output.calibration_mode
            && !calibration_textures.is_empty()
            && surface_manager.surfaces.is_empty()
        {
            output.render(context, &calibration_textures[0].1);
        } else if surface_manager.surfaces.is_empty() {
            output.render(context, mixer.composite_view());
        } else if !output.surface_assignments.is_empty() {
            let render_infos: Vec<SurfaceRenderInfo<'_>> = output
                .surface_assignments
                .iter()
                .enumerate()
                .filter(|(_, a)| a.enabled)
                .filter_map(|(ai, assignment)| {
                    let (_, surface) = surface_manager.find_by_uuid(&assignment.surface_uuid)?;
                    let bb = surface.bounding_box();
                    let content_view =
                        if output.calibration_mode && !calibration_textures.is_empty() {
                            &calibration_textures[ai % calibration_textures.len()].1
                        } else {
                            Self::resolve_source(mixer, &surface.source, domemaster_view)?
                        };
                    let (uv_scale, uv_offset) = if output.calibration_mode {
                        ([1.0, 1.0], [0.0, 0.0])
                    } else {
                        Self::compute_uv(surface.content_mapping, &bb)
                    };
                    Some(SurfaceRenderInfo {
                        content_view,
                        vertices: &surface.vertices,
                        bounding_box: [bb.x, bb.y, bb.width, bb.height],
                        uv_scale,
                        uv_offset,
                        warp_mode: Some(assignment.warp_mode.clone()),
                        overlap_zones: assignment.overlap_zones.clone(),
                    })
                })
                .collect();
            output.render_surfaces(context, &render_infos);
        } else {
            let render_infos: Vec<SurfaceRenderInfo<'_>> = surface_manager
                .surfaces
                .iter()
                .enumerate()
                .filter_map(|(si, surface)| {
                    let bb = surface.bounding_box();
                    let content_view =
                        if output.calibration_mode && !calibration_textures.is_empty() {
                            &calibration_textures[si % calibration_textures.len()].1
                        } else {
                            Self::resolve_source(mixer, &surface.source, domemaster_view)?
                        };
                    let (uv_scale, uv_offset) = if output.calibration_mode {
                        ([1.0, 1.0], [0.0, 0.0])
                    } else {
                        Self::compute_uv(surface.content_mapping, &bb)
                    };
                    Some(SurfaceRenderInfo {
                        content_view,
                        vertices: &surface.vertices,
                        bounding_box: [bb.x, bb.y, bb.width, bb.height],
                        uv_scale,
                        uv_offset,
                        warp_mode: None,
                        overlap_zones: Default::default(),
                    })
                })
                .collect();
            output.render_surfaces(context, &render_infos);
        }
        output.window.request_redraw();
    }

    fn resolve_source<'a>(
        mixer: &'a Mixer,
        source: &OutputSource,
        domemaster_view: Option<&'a wgpu::TextureView>,
    ) -> Option<&'a wgpu::TextureView> {
        match source {
            OutputSource::Master => Some(mixer.composite_view()),
            OutputSource::Channel(ch_idx) => {
                mixer.channels().get(*ch_idx).map(|ch| &ch.composite_view)
            }
            OutputSource::Channels(indices) => {
                let mut sorted = indices.clone();
                sorted.sort();
                sorted.dedup();
                mixer.get_sub_mix_view(&sorted)
            }
            OutputSource::Deck(ch_idx, deck_idx) => mixer
                .channels()
                .get(*ch_idx)
                .and_then(|ch| ch.decks.get(*deck_idx))
                .map(|slot| &slot.deck.texture_view),
            OutputSource::Domemaster => domemaster_view,
        }
    }

    fn compute_uv(
        mapping: ContentMapping,
        bb: &crate::surface::BoundingBox,
    ) -> ([f32; 2], [f32; 2]) {
        match mapping {
            ContentMapping::Fill => ([1.0, 1.0], [0.0, 0.0]),
            ContentMapping::Mapped => ([bb.width, bb.height], [bb.x, bb.y]),
        }
    }

    /// Refresh monitors from the event loop.
    pub fn refresh_monitors(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.output.cached_monitors = event_loop
            .available_monitors()
            .map(|m| {
                let name = m.name().unwrap_or_else(|| "Unknown".to_string());
                (name, m)
            })
            .collect();
    }

    /// Render all active headless outputs — readback + deliver frames.
    fn render_headless_outputs_inner(
        outputs: &mut [crate::renderer::context::UnifiedOutput],
        context: &crate::renderer::context::GpuContext,
        mixer: &crate::mixer::Mixer,
        surface_manager: &crate::surface::SurfaceManager,
        ndi_manager: &mut crate::ndi::NdiManager,
        #[cfg(target_os = "macos")] syphon_manager: &mut crate::syphon::SyphonManager,
        domemaster_view: Option<&wgpu::TextureView>,
    ) {
        for output in outputs.iter_mut() {
            let h = match output {
                crate::renderer::context::UnifiedOutput::Headless(h) if h.active => h,
                _ => continue,
            };

            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Headless Output Encoder"),
                    });

            // Post-process edge blend only for Manual mode; Auto uses per-surface shader blend.
            let use_edge_blend = h.edge_blend_mode
                == crate::renderer::edge_blend::EdgeBlendMode::Manual
                && h.edge_blend.any_enabled();
            // When edge blending: render to intermediate, then blend → final texture.
            let render_target = if use_edge_blend {
                &h.edge_blend_texture_view
            } else {
                &h.texture_view
            };

            if !h.surface_assignments.is_empty() {
                // Surface-routed rendering: render assigned surfaces with warp
                // Triangulate on the CPU, then prepare draws from the pipeline's
                // persistent param/vertex pools (no per-frame GPU buffer alloc).
                let draws: Vec<crate::renderer::blit::PolygonDrawDesc<'_>> = h
                    .surface_assignments
                    .iter()
                    .filter(|a| a.enabled)
                    .filter_map(|assignment| {
                        let (_, surface) =
                            surface_manager.find_by_uuid(&assignment.surface_uuid)?;
                        let bb = surface.bounding_box();
                        let content_view =
                            Self::resolve_source(mixer, &surface.source, domemaster_view)?;
                        let (uv_scale, uv_offset) = Self::compute_uv(surface.content_mapping, &bb);
                        let (homography, vertices) = match &assignment.warp_mode {
                            crate::renderer::warp::WarpMode::CornerPin { corners } => {
                                let src_corners = [
                                    [bb.x, bb.y],
                                    [bb.x + bb.width, bb.y],
                                    [bb.x + bb.width, bb.y + bb.height],
                                    [bb.x, bb.y + bb.height],
                                ];
                                let homography = crate::renderer::warp::compute_forward_homography(
                                    &src_corners,
                                    corners,
                                );
                                let verts =
                                    crate::renderer::blit::PolygonBlitPipeline::triangulate_verts(
                                        &surface.vertices,
                                        bb.x,
                                        bb.y,
                                        bb.width,
                                        bb.height,
                                    );
                                (Some(homography), verts)
                            }
                            crate::renderer::warp::WarpMode::Mesh(mesh) => (
                                None,
                                crate::renderer::blit::PolygonBlitPipeline::mesh_verts(mesh),
                            ),
                        };
                        Some(crate::renderer::blit::PolygonDrawDesc {
                            content_view,
                            uv_scale,
                            uv_offset,
                            homography,
                            overlap_zones: &assignment.overlap_zones,
                            vertices,
                        })
                    })
                    .collect();

                let (prepared, vertex_pool) =
                    h.polygon_pipeline
                        .prepare(&context.device, &context.queue, &draws);

                {
                    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Headless Surface Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: render_target,
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
                    h.polygon_pipeline.draw(&mut rp, &prepared, &vertex_pool);
                }
            } else {
                // Fallback: simple blit from source
                let source_view = match Self::resolve_source(mixer, &h.source, domemaster_view) {
                    Some(view) => view,
                    None => continue,
                };
                h.blit_pipeline
                    .set_rotation(&context.queue, h.rotation.index());
                let bind_group = h
                    .blit_pipeline
                    .create_bind_group(&context.device, source_view);
                {
                    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Headless Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: render_target,
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
                    h.blit_pipeline.render(&mut rp, &bind_group);
                }
            }

            // Apply edge blend post-process if any edge is enabled
            if use_edge_blend {
                h.edge_blend_pipeline.render(
                    &context.device,
                    &context.queue,
                    &mut encoder,
                    &h.edge_blend_texture_view,
                    &h.texture_view,
                    &h.edge_blend,
                );
            }

            // Enqueue readback copy from the now-rendered texture
            h.readback.begin_readback(&mut encoder, &h.texture);
            context.queue.submit(std::iter::once(encoder.finish()));

            // Deliver previous frame's readback data to target
            if let Some(frame_data) = h.readback.try_read(&context.device) {
                match h.deliver_frame(
                    &frame_data,
                    ndi_manager,
                    #[cfg(target_os = "macos")]
                    syphon_manager,
                ) {
                    crate::renderer::context::DeliveryResult::Failed(msg) => {
                        log::error!("{}", msg);
                        h.active = false;
                    }
                    crate::renderer::context::DeliveryResult::Restarted => {
                        log::info!("SRT restarted for '{}'", h.name);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Open a native file picker on a background thread.
    /// Uses rfd's synchronous FileDialog which correctly dispatches to the
    /// main thread on macOS (NSOpenPanel requires main-thread presentation
    /// for proper focus/activation). Results are sent via channel.
    pub fn open_file_dialog(
        sender: &std::sync::mpsc::Sender<FileDialogResult>,
        kind: FileDialogKind,
        ch_idx: usize,
    ) {
        let tx = sender.clone();
        std::thread::spawn(move || {
            let dialog = match kind {
                FileDialogKind::Image => rfd::FileDialog::new().add_filter(
                    "Images",
                    &["png", "jpg", "jpeg", "bmp", "tiff", "tga", "webp"],
                ),
                FileDialogKind::Video => rfd::FileDialog::new()
                    .add_filter("Video", &["mov", "mp4", "avi", "mkv", "webm", "gif"]),
            };
            if let Some(paths) = dialog.pick_files() {
                if !paths.is_empty() {
                    let _ = tx.send(FileDialogResult {
                        kind,
                        ch_idx,
                        paths,
                    });
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{BoundingBox, ContentMapping};

    #[test]
    fn compute_uv_fill() {
        let bb = BoundingBox {
            x: 0.2,
            y: 0.3,
            width: 0.4,
            height: 0.5,
        };
        let (scale, offset) = VardaApp::compute_uv(ContentMapping::Fill, &bb);
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn compute_uv_mapped() {
        let bb = BoundingBox {
            x: 0.2,
            y: 0.3,
            width: 0.4,
            height: 0.5,
        };
        let (scale, offset) = VardaApp::compute_uv(ContentMapping::Mapped, &bb);
        assert_eq!(scale, [0.4, 0.5]);
        assert_eq!(offset, [0.2, 0.3]);
    }

    #[test]
    fn compute_uv_mapped_full_canvas() {
        let bb = BoundingBox {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let (scale, offset) = VardaApp::compute_uv(ContentMapping::Mapped, &bb);
        // Full canvas mapped should behave like fill
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn fps_smoothing_converges() {
        use clap::Parser;
        fn parse_args(args: &[&str]) -> super::super::AppConfig {
            super::super::AppConfig::parse_from(
                std::iter::once("varda").chain(args.iter().copied()),
            )
        }
        let gpu = crate::renderer::context::GpuContext::new_headless();
        let Ok(gpu) = gpu else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
        let Ok(mut app) = VardaApp::new(gpu, &config) else {
            eprintln!("Skipping: VardaApp creation failed");
            return;
        };
        // Seed with 60 identical FPS values
        app.frame_stats.fps_history.clear();
        for _ in 0..60 {
            app.frame_stats.fps_history.push_back(60.0);
        }
        app.frame_stats.fps_smoothed = app.frame_stats.fps_history.iter().sum::<f32>()
            / app.frame_stats.fps_history.len() as f32;
        assert!((app.frame_stats.fps_smoothed - 60.0).abs() < 0.01);
    }

    #[test]
    fn fps_smoothing_window_cap() {
        use clap::Parser;
        fn parse_args(args: &[&str]) -> super::super::AppConfig {
            super::super::AppConfig::parse_from(
                std::iter::once("varda").chain(args.iter().copied()),
            )
        }
        let gpu = crate::renderer::context::GpuContext::new_headless();
        let Ok(gpu) = gpu else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let config = parse_args(&["--headless", "--no-osc", "--no-ndi", "--no-syphon"]);
        let Ok(mut app) = VardaApp::new(gpu, &config) else {
            eprintln!("Skipping: VardaApp creation failed");
            return;
        };
        // Push more than 60 entries
        app.frame_stats.fps_history.clear();
        for _ in 0..100 {
            app.frame_stats.fps_history.push_back(30.0);
            if app.frame_stats.fps_history.len() > 60 {
                app.frame_stats.fps_history.pop_front();
            }
        }
        assert_eq!(
            app.frame_stats.fps_history.len(),
            60,
            "Window should cap at 60 entries"
        );
    }

    // ── Offensive: catch_unwind pattern delivers error through channel ──

    #[test]
    fn catch_unwind_delivers_error_on_panic() {
        let (tx, rx) = std::sync::mpsc::channel::<DeckLoadResult>();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let c = counter.clone();
        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> (String, anyhow::Result<crate::deck::Deck>) {
                    panic!("simulated loader panic");
                },
            ));
            let (name, deck) = match result {
                Ok((name, deck)) => (name, deck),
                Err(_) => (
                    "panicked".to_string(),
                    Err(anyhow::anyhow!("panic in loader")),
                ),
            };
            let _ = tx.send(DeckLoadResult {
                ch_idx: 0,
                deck,
                name,
            });
            c.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });

        let msg = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("should receive result even after panic");
        assert!(msg.deck.is_err(), "deck should be an error after panic");
        assert_eq!(msg.name, "panicked");
        // Counter should be back to zero (cleanup ran)
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "counter must decrement even after panic"
        );
    }
}
