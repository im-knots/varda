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
                let name = path.file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("image")
                    .to_string();
                let deck = Deck::new_from_image(&ctx, &path, w, h);
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
                let name = path.file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("video")
                    .to_string();
                let deck = Deck::new_from_video(&ctx, &path, w, h);
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
                let name = shader.name();
                let deck = Deck::new(&ctx, shader, w, h);
                let _ = tx.send(DeckLoadResult { ch_idx, deck, name });
                counter.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }

    /// Update frame timing (FPS measurement) and system stats. Call once per frame before any work.
    pub fn update_frame_timing(&mut self) {
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_frame_instant).as_secs_f32();
        self.last_frame_instant = now;
        if dt > 0.0 {
            let instant_fps = 1.0 / dt;
            self.fps_history.push(instant_fps);
            if self.fps_history.len() > 60 {
                self.fps_history.remove(0);
            }
            self.fps_smoothed = self.fps_history.iter().sum::<f32>() / self.fps_history.len() as f32;
        }
        self.system_monitor.update();
    }

    /// Render the mixer frame: update cameras, NDI, Syphon, collect audio, render mixer.
    /// This performs all GPU work that doesn't need the surface texture.
    pub fn render_mixer_frame(&mut self) {
        // Compute effective channel opacities to determine which cameras are needed
        let channel_count = self.mixer.channel_count();
        let crossfader = self.mixer.crossfader();
        let effective_opacities: Vec<f32> = if channel_count == 2 {
            let channels = self.mixer.channels();
            vec![
                (1.0 - crossfader) * channels[0].opacity,
                crossfader * channels[1].opacity,
            ]
        } else {
            self.mixer.channels().iter().map(|ch| ch.opacity).collect()
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
        self.camera_manager.update_selective(&self.context.queue, &needed_camera_ids);

        // Update NDI receiver frames
        self.ndi_manager.update(&self.context.device, &self.context.queue);

        // Update Syphon client frames
        #[cfg(target_os = "macos")]
        self.syphon_manager.update(&self.context.queue);

        // Update stream receiver frames
        self.stream_manager.update(&self.context.queue);

        for channel in self.mixer.channels_mut() {
            for slot in &mut channel.decks {
                if let Some(cam_id) = slot.deck.camera_id() {
                    slot.deck.camera_source_view = self.camera_manager
                        .texture_view(cam_id)
                        .cloned();
                }
                // Update NDI deck texture views
                if let Some(ndi_idx) = slot.deck.ndi_receiver_idx() {
                    slot.deck.ndi_source_view = self.ndi_manager
                        .texture_view(ndi_idx)
                        .cloned();
                }
                // Update Syphon deck texture views
                #[cfg(target_os = "macos")]
                if let Some(syph_idx) = slot.deck.syphon_client_idx() {
                    slot.deck.syphon_source_view = self.syphon_manager
                        .texture_view(syph_idx)
                        .cloned();
                }
                // Update stream deck texture views
                if let Some(srt_idx) = slot.deck.srt_receiver_idx() {
                    slot.deck.srt_source_view = self.stream_manager
                        .texture_view(srt_idx)
                        .cloned();
                }
            }
        }

        // Collect audio values for modulation
        let audio_values = {
            let mut av = crate::modulation::AudioValues::default();
            for id in self.audio_manager.active_source_ids() {
                if let Some(data) = self.audio_manager.get_data(id) {
                    av.sources.insert(id, crate::modulation::AudioSourceValues {
                        fft: data.fft.clone(),
                        level: data.level,
                        sample_rate: data.sample_rate,
                    });
                }
            }
            av
        };

        let mut primary_audio = self.audio_manager.get_primary_data().clone();

        // Override audio BPM/beat with clock-resolved values (MIDI > OSC > Audio)
        let clock = self.clock_manager.state();
        if clock.active {
            primary_audio.bpm = Some(clock.bpm);
            primary_audio.time_since_beat = clock.beat_phase * (60.0 / clock.bpm);
        }

        if let Err(e) = self.mixer.render(&self.context, &primary_audio, &audio_values) {
            log::error!("Failed to render mixer: {}", e);
        }
    }

    /// Render content to all outputs (windowed + headless) using the surface layout.
    pub fn render_outputs(&mut self) {
        let context = &self.context;

        // Prepare sub-mixes for any Channels(...) sources
        {
            let mut sub_mix_sources: Vec<Vec<usize>> = Vec::new();
            for surface in &self.surface_manager.surfaces {
                if let OutputSource::Channels(indices) = &surface.source {
                    let mut sorted = indices.clone();
                    sorted.sort();
                    sorted.dedup();
                    if !sub_mix_sources.contains(&sorted) {
                        sub_mix_sources.push(sorted);
                    }
                }
            }
            self.mixer.prepare_sub_mixes(&sub_mix_sources, context);
        }

        let mixer = &self.mixer;

        for output in &self.outputs {
            match output {
                crate::renderer::context::UnifiedOutput::Window(output) => {
                    Self::render_window_output(output, context, mixer, &self.surface_manager, &self.calibration_textures);
                }
                crate::renderer::context::UnifiedOutput::Headless(_) => {
                    // Headless rendering handled separately (needs &mut for subprocess)
                }
            }
        }

        // Render headless outputs (needs &mut self for subprocess feeding)
        Self::render_headless_outputs_inner(
            &mut self.outputs, context, mixer,
            &self.surface_manager,
            &mut self.ndi_manager,
            #[cfg(target_os = "macos")]
            &mut self.syphon_manager,
        );
    }

    fn render_window_output(
        output: &crate::renderer::context::OutputWindow,
        context: &crate::renderer::context::GpuContext,
        mixer: &crate::mixer::Mixer,
        surface_manager: &crate::surface::SurfaceManager,
        calibration_textures: &[(wgpu::Texture, wgpu::TextureView)],
    ) {
        if output.calibration_mode && !calibration_textures.is_empty() && surface_manager.surfaces.is_empty() {
            output.render(context, &calibration_textures[0].1);
        } else if surface_manager.surfaces.is_empty() {
            output.render(context, mixer.composite_view());
        } else if !output.surface_assignments.is_empty() {
            let render_infos: Vec<SurfaceRenderInfo<'_>> = output.surface_assignments.iter()
                .enumerate()
                .filter(|(_, a)| a.enabled)
                .filter_map(|(ai, assignment)| {
                    let (_, surface) = surface_manager.find_by_uuid(&assignment.surface_uuid)?;
                    let bb = surface.bounding_box();
                    let content_view = if output.calibration_mode && !calibration_textures.is_empty() {
                        &calibration_textures[ai % calibration_textures.len()].1
                    } else {
                        Self::resolve_source(mixer, &surface.source)?
                    };
                    let (uv_scale, uv_offset) = if output.calibration_mode {
                        ([1.0, 1.0], [0.0, 0.0])
                    } else {
                        Self::compute_uv(surface.content_mapping, &bb)
                    };
                    Some(SurfaceRenderInfo {
                        content_view, vertices: &surface.vertices,
                        bounding_box: [bb.x, bb.y, bb.width, bb.height],
                        uv_scale, uv_offset,
                        warp_corners: Some(assignment.warp_corners),
                    })
                })
                .collect();
            output.render_surfaces(context, &render_infos);
        } else {
            let render_infos: Vec<SurfaceRenderInfo<'_>> = surface_manager.surfaces.iter()
                .enumerate()
                .filter_map(|(si, surface)| {
                    let bb = surface.bounding_box();
                    let content_view = if output.calibration_mode && !calibration_textures.is_empty() {
                        &calibration_textures[si % calibration_textures.len()].1
                    } else {
                        Self::resolve_source(mixer, &surface.source)?
                    };
                    let (uv_scale, uv_offset) = if output.calibration_mode {
                        ([1.0, 1.0], [0.0, 0.0])
                    } else {
                        Self::compute_uv(surface.content_mapping, &bb)
                    };
                    Some(SurfaceRenderInfo {
                        content_view, vertices: &surface.vertices,
                        bounding_box: [bb.x, bb.y, bb.width, bb.height],
                        uv_scale, uv_offset, warp_corners: None,
                    })
                })
                .collect();
            output.render_surfaces(context, &render_infos);
        }
        output.window.request_redraw();
    }


    fn resolve_source<'a>(mixer: &'a Mixer, source: &OutputSource) -> Option<&'a wgpu::TextureView> {
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
            OutputSource::Deck(ch_idx, deck_idx) => {
                mixer.channels().get(*ch_idx)
                    .and_then(|ch| ch.decks.get(*deck_idx))
                    .map(|slot| &slot.deck.texture_view)
            }
        }
    }

    fn compute_uv(mapping: ContentMapping, bb: &crate::surface::BoundingBox) -> ([f32; 2], [f32; 2]) {
        match mapping {
            ContentMapping::Fill => ([1.0, 1.0], [0.0, 0.0]),
            ContentMapping::Mapped => ([bb.width, bb.height], [bb.x, bb.y]),
        }
    }

    /// Refresh monitors from the event loop.
    pub fn refresh_monitors(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.cached_monitors = event_loop.available_monitors()
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
        #[cfg(target_os = "macos")]
        syphon_manager: &mut crate::syphon::SyphonManager,
    ) {
        for output in outputs.iter_mut() {
            let h = match output {
                crate::renderer::context::UnifiedOutput::Headless(h) if h.active => h,
                _ => continue,
            };

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Headless Output Encoder"),
            });

            if !h.surface_assignments.is_empty() {
                // Surface-routed rendering: render assigned surfaces with warp
                let prepared: Vec<_> = h.surface_assignments.iter()
                    .filter(|a| a.enabled)
                    .filter_map(|assignment| {
                        let (_, surface) = surface_manager.find_by_uuid(&assignment.surface_uuid)?;
                        let bb = surface.bounding_box();
                        let content_view = Self::resolve_source(mixer, &surface.source)?;
                        let (uv_scale, uv_offset) = Self::compute_uv(surface.content_mapping, &bb);
                        let homography = {
                            let src_corners = [
                                [bb.x, bb.y],
                                [bb.x + bb.width, bb.y],
                                [bb.x + bb.width, bb.y + bb.height],
                                [bb.x, bb.y + bb.height],
                            ];
                            crate::renderer::warp::compute_forward_homography(&src_corners, &assignment.warp_corners)
                        };
                        let bind_group = h.polygon_pipeline.create_bind_group(
                            &context.device, content_view,
                            uv_scale, uv_offset, Some(&homography),
                        );
                        let (vb, num_tris) = crate::renderer::blit::PolygonBlitPipeline::triangulate(
                            &context.device, &surface.vertices,
                            bb.x, bb.y, bb.width, bb.height,
                        );
                        Some((bind_group, vb, num_tris))
                    })
                    .collect();

                {
                    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Headless Surface Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &h.texture_view,
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
                    });
                    for (bind_group, vb, num_tris) in &prepared {
                        if *num_tris > 0 {
                            h.polygon_pipeline.render_polygon(
                                &context.device, &mut rp, bind_group, vb, *num_tris,
                            );
                        }
                    }
                }
            } else {
                // Fallback: simple blit from source
                let source_view = match Self::resolve_source(mixer, &h.source) {
                    Some(view) => view,
                    None => continue,
                };
                let bind_group = h.blit_pipeline.create_bind_group(&context.device, source_view);
                {
                    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Headless Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &h.texture_view,
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
                    });
                    h.blit_pipeline.render(&mut rp, &bind_group);
                }
            }

            // Enqueue readback copy from the now-rendered texture
            h.readback.begin_readback(&mut encoder, &h.texture);
            context.queue.submit(std::iter::once(encoder.finish()));

            // Deliver previous frame's readback data to target
            if let Some(frame_data) = h.readback.try_read(&context.device) {
                match &mut h.target {
                    crate::renderer::context::OutputTarget::Recording { .. } => {
                        if let Some(sub) = &mut h.subprocess {
                            if !sub.feed_frame(&frame_data) {
                                log::error!("Subprocess write failed for '{}', stopping", h.name);
                                if let Some(mut sub) = h.subprocess.take() {
                                    sub.stop();
                                }
                                h.active = false;
                            }
                        }
                    }
                    crate::renderer::context::OutputTarget::SrtStream { ref url, ref codec } => {
                        if let Some(sub) = &mut h.subprocess {
                            if !sub.feed_frame(&frame_data) {
                                // SRT client disconnected — auto-restart the listener
                                log::info!("SRT client disconnected on '{}', restarting listener", h.name);
                                if let Some(mut sub) = h.subprocess.take() {
                                    sub.stop();
                                }
                                match crate::renderer::FfmpegSubprocess::spawn_srt(
                                    url, codec, h.width, h.height, 30,
                                ) {
                                    Ok(new_sub) => {
                                        h.subprocess = Some(new_sub);
                                        // h.active stays true — ready for next client
                                    }
                                    Err(e) => {
                                        log::error!("Failed to restart SRT listener: {}", e);
                                        h.active = false;
                                    }
                                }
                            }
                        }
                    }
                    crate::renderer::context::OutputTarget::HlsStream { .. } |
                    crate::renderer::context::OutputTarget::DashStream { .. } => {
                        if let Some(sub) = &mut h.subprocess {
                            if !sub.feed_frame(&frame_data) {
                                log::error!("Stream subprocess write failed for '{}', stopping", h.name);
                                if let Some(mut sub) = h.subprocess.take() {
                                    sub.stop();
                                }
                                h.active = false;
                            }
                        }
                    }
                    crate::renderer::context::OutputTarget::NdiSend { ref sender_name } => {
                        ndi_manager.send_frame(sender_name, &frame_data, h.width, h.height);
                    }
                    #[cfg(target_os = "macos")]
                    crate::renderer::context::OutputTarget::SyphonServer { .. } => {
                        syphon_manager.publish_frame(&frame_data, h.width, h.height);
                    }
                    #[cfg(not(target_os = "macos"))]
                    crate::renderer::context::OutputTarget::SyphonServer { .. } => {
                        log::warn!("Syphon output not supported on this platform");
                    }
                    _ => {} // Windowed/Display targets don't appear on headless outputs
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
                FileDialogKind::Image => rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "tiff", "tga", "webp"]),
                FileDialogKind::Video => rfd::FileDialog::new()
                    .add_filter("Video", &["mov", "mp4", "avi", "mkv", "webm", "gif"]),
            };
            if let Some(paths) = dialog.pick_files() {
                if !paths.is_empty() {
                    let _ = tx.send(FileDialogResult { kind, ch_idx, paths });
                }
            }
        });
    }
}