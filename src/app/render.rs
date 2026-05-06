//! GPU rendering — mixer render, output windows, frame timing.

use super::VardaApp;
use crate::mixer::Mixer;
use crate::renderer::context::{OutputSource, SurfaceRenderInfo};
use crate::surface::ContentMapping;

impl VardaApp {
    /// Update frame timing (FPS measurement). Call once per frame before any work.
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
    }

    /// Render the mixer frame: update cameras, collect audio, render mixer.
    /// This performs all GPU work that doesn't need the surface texture.
    pub fn render_mixer_frame(&mut self) {
        // Update camera frames
        self.camera_manager.update(&self.context.queue);
        for channel in &mut self.mixer.channels {
            for slot in &mut channel.decks {
                if let Some(cam_id) = slot.deck.camera_id() {
                    slot.deck.camera_source_view = self.camera_manager
                        .texture_view(cam_id)
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

        let primary_audio = self.audio_manager.get_primary_data().clone();
        if let Err(e) = self.mixer.render(&self.context, &primary_audio, &audio_values) {
            log::error!("Failed to render mixer: {}", e);
        }
    }

    /// Render content to all output windows using the surface layout.
    pub fn render_output_windows(&mut self) {
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

        for output in &self.output_windows {
            if output.calibration_mode && !self.calibration_textures.is_empty() && self.surface_manager.surfaces.is_empty() {
                output.render(context, &self.calibration_textures[0].1);
            } else if self.surface_manager.surfaces.is_empty() {
                output.render(context, &mixer.composite_view);
            } else if !output.surface_assignments.is_empty() {
                let render_infos: Vec<SurfaceRenderInfo<'_>> = output.surface_assignments.iter()
                    .enumerate()
                    .filter(|(_, a)| a.enabled)
                    .filter_map(|(ai, assignment)| {
                        let surface = self.surface_manager.surfaces.get(assignment.surface_idx)?;
                        let bb = surface.bounding_box();
                        let content_view = if output.calibration_mode && !self.calibration_textures.is_empty() {
                            &self.calibration_textures[ai % self.calibration_textures.len()].1
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
                let render_infos: Vec<SurfaceRenderInfo<'_>> = self.surface_manager.surfaces.iter()
                    .enumerate()
                    .filter_map(|(si, surface)| {
                        let bb = surface.bounding_box();
                        let content_view = if output.calibration_mode && !self.calibration_textures.is_empty() {
                            &self.calibration_textures[si % self.calibration_textures.len()].1
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
    }


    fn resolve_source<'a>(mixer: &'a Mixer, source: &OutputSource) -> Option<&'a wgpu::TextureView> {
        match source {
            OutputSource::Master => Some(&mixer.composite_view),
            OutputSource::Channel(ch_idx) => {
                mixer.channels.get(*ch_idx).map(|ch| &ch.composite_view)
            }
            OutputSource::Channels(indices) => {
                let mut sorted = indices.clone();
                sorted.sort();
                sorted.dedup();
                mixer.get_sub_mix_view(&sorted)
            }
            OutputSource::Deck(ch_idx, deck_idx) => {
                mixer.channels.get(*ch_idx)
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

    /// Handle file dialog actions (deferred from UI for macOS Finder focus).
    pub fn handle_file_dialogs(
        &mut self,
        ui_actions: &mut crate::usecases::ui::UIActions,
        egui_renderer: &mut egui_wgpu::Renderer,
        deck_preview_textures: &mut std::collections::HashMap<(usize, usize), egui::TextureId>,
    ) {
        if let Some(ch_idx) = ui_actions.open_image_dialog_for_channel {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "tiff", "tga", "webp"])
                .pick_file()
            {
                ui_actions.image_to_add = Some((ch_idx, path));
                self.apply_deck_and_effect_actions(ui_actions, egui_renderer, deck_preview_textures);
            }
        }
        if let Some(ch_idx) = ui_actions.open_video_dialog_for_channel {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Video", &["mov", "mp4", "avi", "mkv", "webm"])
                .pick_file()
            {
                ui_actions.video_to_add = Some((ch_idx, path));
                self.apply_deck_and_effect_actions(ui_actions, egui_renderer, deck_preview_textures);
            }
        }
    }
}