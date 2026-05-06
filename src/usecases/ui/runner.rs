//! UIRunner — windowed delivery layer for the Varda engine.
//!
//! Owns the window, egui state, blit pipeline, texture registrations, and WindowSurface.
//! The engine (`VardaApp`) is owned here and driven each frame.
//! For headless operation (HTTP API, CLI), this module is simply not used.

use crate::app::VardaApp;
use crate::renderer::context::{GpuContext, WindowSurface};
use crate::usecases::ui;
use crate::*;

use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

pub struct UIRunner {
    // ── Window / egui state (delivery layer) ────────────────────────
    window: Option<&'static Window>,
    window_surface: Option<WindowSurface>,
    blit_pipeline: Option<BlitPipeline>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    deck_preview_textures: std::collections::HashMap<(usize, usize), egui::TextureId>,
    main_output_texture: Option<egui::TextureId>,
    main_window_id: Option<WindowId>,

    // ── Engine (owns all subsystems) ────────────────────────────────
    varda: VardaApp,
}

impl UIRunner {
    pub fn new() -> Self {
        Self {
            window: None,
            window_surface: None,
            blit_pipeline: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            deck_preview_textures: std::collections::HashMap::new(),
            main_output_texture: None,
            main_window_id: None,
            varda: VardaApp::new(),
        }
    }

    /// Run the UI event loop. Blocks until the window is closed.
    pub fn run(mut self) -> anyhow::Result<()> {
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut self)
            .map_err(|e| anyhow::anyhow!("Event loop error: {:?}", e))?;
        Ok(())
    }
}

impl ApplicationHandler for UIRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        let window_attrs = Window::default_attributes()
            .with_title("Varda VJ Software")
            .with_inner_size(winit::dpi::LogicalSize::new(1920, 1080));

        let window_static: &'static Window = match event_loop.create_window(window_attrs) {
            Ok(w) => { log::info!("Window created"); Box::leak(Box::new(w)) }
            Err(e) => { log::error!("Failed to create window: {}", e); event_loop.exit(); return; }
        };
        self.main_window_id = Some(window_static.id());
        self.window = Some(window_static);

        let (gpu, win_surface) = match pollster::block_on(GpuContext::new_for_window(window_static)) {
            Ok(pair) => pair,
            Err(e) => { log::error!("Failed to create render context: {}", e); event_loop.exit(); return; }
        };
        log::info!("Varda initialized: {}x{}, {} shaders", win_surface.size.width, win_surface.size.height, self.varda.registry.count());

        self.blit_pipeline = BlitPipeline::new(&gpu.device, win_surface.surface_config.format).ok();
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(), egui::ViewportId::ROOT, window_static,
            Some(window_static.scale_factor() as f32), None, Some(2 * 1024),
        ));
        self.egui_renderer = Some(egui_wgpu::Renderer::new(
            &gpu.device, win_surface.surface_config.format, egui_wgpu::RendererOptions::default(),
        ));
        self.varda.audio_textures = Some(AudioTextures::new(&gpu.device));
        self.varda.calibration_textures = crate::renderer::context::create_calibration_textures(&gpu.device, &gpu.queue, 8);
        self.window_surface = Some(win_surface);
        self.varda.context = Some(gpu);

        // Load workspace + init default mixer (engine concerns)
        self.varda.load_workspace();
        self.varda.init_default_mixer();

        // Register GPU textures with egui for previews
        self.register_preview_textures();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        if self.main_window_id == Some(window_id) {
            if let (Some(window), Some(egui_state)) = (self.window, &mut self.egui_state) {
                if egui_state.on_window_event(window, &event).consumed { return; }
            }
            match event {
                WindowEvent::CloseRequested => {
                    log::info!("Close requested, saving workspace and exiting...");
                    self.varda.save_workspace();
                    event_loop.exit();
                }
                WindowEvent::Resized(new_size) => {
                    if let (Some(ctx), Some(ws)) = (&self.varda.context, &mut self.window_surface) {
                        ws.resize(&ctx.device, new_size);
                    }
                }
                WindowEvent::RedrawRequested => {
                    self.render(event_loop);
                    if let Some(w) = self.window { w.request_redraw(); }
                }
                _ => {}
            }
        } else {
            match event {
                WindowEvent::CloseRequested => {
                    if let Some(idx) = self.varda.output_windows.iter().position(|o| o.window.id() == window_id) {
                        let name = self.varda.output_windows[idx].name.clone();
                        self.varda.output_windows.remove(idx).destroy();
                        log::info!("Output window '{}' closed", name);
                    }
                }
                WindowEvent::Resized(new_size) => {
                    if let Some(ctx) = &self.varda.context {
                        if let Some(o) = self.varda.output_windows.iter_mut().find(|o| o.window.id() == window_id) {
                            o.resize(&ctx.device, new_size);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}


impl UIRunner {
    /// Register GPU textures with egui for deck previews and main output.
    fn register_preview_textures(&mut self) {
        let Some(context) = &self.varda.context else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else { return };
        let Some(mixer) = &self.varda.mixer else { return };

        for (ch_idx, ch) in mixer.channels.iter().enumerate() {
            for (deck_idx, slot) in ch.decks.iter().enumerate() {
                let tid = egui_renderer.register_native_texture(
                    &context.device, &slot.deck.texture_view, wgpu::FilterMode::Linear,
                );
                self.deck_preview_textures.insert((ch_idx, deck_idx), tid);
            }
        }
        self.main_output_texture = Some(egui_renderer.register_native_texture(
            &context.device, &mixer.composite_view, wgpu::FilterMode::Linear,
        ));
    }

    /// Re-register GPU textures when deck layout changes.
    fn refresh_textures(&mut self) {
        let Some(context) = &self.varda.context else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else { return };
        let Some(mixer) = &self.varda.mixer else { return };

        if self.main_output_texture.is_none() {
            self.main_output_texture = Some(egui_renderer.register_native_texture(
                &context.device, &mixer.composite_view, wgpu::FilterMode::Linear,
            ));
        }
        for (ch_idx, ch) in mixer.channels.iter().enumerate() {
            for (deck_idx, slot) in ch.decks.iter().enumerate() {
                let key = (ch_idx, deck_idx);
                if !self.deck_preview_textures.contains_key(&key) {
                    let tid = egui_renderer.register_native_texture(
                        &context.device, &slot.deck.texture_view, wgpu::FilterMode::Linear,
                    );
                    self.deck_preview_textures.insert(key, tid);
                }
            }
        }
    }

    /// Main render loop — delegates all logic to VardaApp.
    fn render(&mut self, event_loop: &ActiveEventLoop) {
        // 1. Frame timing + notifications
        self.varda.update_frame_timing();
        self.varda.notifications.update();

        // 2. Process cross-thread commands + external inputs
        self.varda.process_commands();
        self.varda.process_inputs();

        // 3. Sync egui texture registrations
        self.refresh_textures();

        let Some(window) = self.window else { return };
        if self.varda.context.is_none() { return; }

        // 4. Create pending output windows + refresh monitors
        self.varda.create_pending_outputs(event_loop);
        self.varda.refresh_monitors(event_loop);

        // 5. Collect UI data snapshot (engine → UI)
        let ui_data = self.varda.collect_ui_data(&self.deck_preview_textures, self.main_output_texture);

        // 6. Run egui frame
        let raw_input = {
            let Some(egui_state) = &mut self.egui_state else { return };
            egui_state.take_egui_input(window)
        };
        let mut ui_actions = ui::UIActions::new();
        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            ui_actions = ui::panels::render_ui(ctx, &ui_data);
        });
        {
            let Some(egui_state) = &mut self.egui_state else { return };
            egui_state.handle_platform_output(window, full_output.platform_output);
        }

        // 7. Apply all UI actions (delegated to VardaApp)
        let egui_renderer = self.egui_renderer.as_mut().unwrap();
        self.varda.apply_engine_actions(&mut ui_actions, egui_renderer, &mut self.deck_preview_textures);
        self.varda.apply_ui_actions(&ui_actions);
        self.varda.apply_output_actions(&ui_actions);
        self.varda.apply_surface_actions(&ui_actions);
        self.varda.apply_device_actions(&ui_actions);
        self.varda.update_controller_leds();

        // 8. Handle save requests
        if ui_actions.save_requested {
            self.varda.save_workspace();
            self.varda.notifications.info("💾 Workspace saved".to_string());
        }

        // 9. Handle deferred file dialogs (macOS Finder focus)
        self.varda.handle_file_dialogs(&mut ui_actions, egui_renderer, &mut self.deck_preview_textures);

        // 10. GPU: render mixer + blit + egui overlay + present
        self.varda.render_mixer_frame();
        self.submit_frame(window, full_output.shapes, full_output.pixels_per_point, full_output.textures_delta);

        // 11. Render output windows (after main present)
        self.varda.render_output_windows();

        // 12. Publish engine state for cross-thread consumers
        self.varda.publish_state();
    }

    /// Blit mixer output to screen, overlay egui, and present.
    fn submit_frame(
        &mut self,
        window: &Window,
        shapes: Vec<egui::epaint::ClippedShape>,
        pixels_per_point: f32,
        textures_delta: egui::TexturesDelta,
    ) {
        let Some(context) = &self.varda.context else { return };
        let Some(win_surface) = &self.window_surface else { return };

        let paint_jobs = self.egui_ctx.tessellate(shapes, pixels_per_point);

        let _ = context.device.poll(wgpu::PollType::Poll);
        let output = match win_surface.surface.get_current_texture() {
            Ok(o) => o,
            Err(e) => { log::error!("Failed to get surface texture: {}", e); return; }
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let Some(egui_renderer) = &mut self.egui_renderer else { return };

        for (id, delta) in &textures_delta.set {
            egui_renderer.update_texture(&context.device, &context.queue, *id, delta);
        }

        let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Screen Encoder"),
        });

        let bind_group = if let (Some(mixer), Some(blit)) = (&self.varda.mixer, &self.blit_pipeline) {
            Some(blit.create_bind_group(&context.device, &mixer.composite_view))
        } else { None };

        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [win_surface.size.width, win_surface.size.height],
            pixels_per_point: window.scale_factor() as f32,
        };

        egui_renderer.update_buffers(&context.device, &context.queue, &mut encoder, &paint_jobs, &screen_desc);

        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view, resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None,
            });
            if let (Some(bg), Some(blit)) = (&bind_group, &self.blit_pipeline) { blit.render(&mut rp, bg); }
            let mut rp_static = rp.forget_lifetime();
            egui_renderer.render(&mut rp_static, &paint_jobs, &screen_desc);
        }

        for id in &textures_delta.free { egui_renderer.free_texture(id); }

        context.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}