//! UIRunner — windowed delivery layer for the Varda engine.
//!
//! Owns the window, egui state, blit pipeline, texture registrations, and WindowSurface.
//! The engine (`VardaApp`) is owned here and driven each frame.
//! For headless operation (HTTP API, CLI), this module is simply not used.

use crate::app::VardaApp;
use crate::renderer::blit::BlitPipeline;
use crate::renderer::context::{GpuContext, WindowSurface};
use crate::usecases::ui;

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

    // ── UI-consumer-owned layout/selection state ─────────────────────
    layout: super::UILayoutState,

    // ── Engine (created after GPU init in resumed()) ─────────────────
    varda: Option<VardaApp>,
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
            layout: super::UILayoutState::default(),
            varda: None,
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

        self.blit_pipeline = BlitPipeline::new(&gpu.device, win_surface.surface_config.format).ok();
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(), egui::ViewportId::ROOT, window_static,
            Some(window_static.scale_factor() as f32), None, Some(2 * 1024),
        ));
        self.egui_renderer = Some(egui_wgpu::Renderer::new(
            &gpu.device, win_surface.surface_config.format, egui_wgpu::RendererOptions::default(),
        ));
        self.window_surface = Some(win_surface);

        // Create engine now that GPU is ready — mixer, audio textures,
        // calibration textures all initialized immediately.
        let mut varda = match VardaApp::new(gpu) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Failed to initialize engine: {}", e);
                event_loop.exit();
                return;
            }
        };
        log::info!("Varda initialized: {} shaders", varda.shader_count());

        // Load workspace (may replace default mixer with saved scene)
        if let Some(loaded_layout) = varda.load_workspace() {
            self.layout = loaded_layout;
        }

        self.varda = Some(varda);

        // Register GPU textures with egui for previews
        self.register_preview_textures();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let Some(varda) = self.varda.as_mut() else { return; };
        if self.main_window_id == Some(window_id) {
            if let (Some(window), Some(egui_state)) = (self.window, &mut self.egui_state) {
                if egui_state.on_window_event(window, &event).consumed { return; }
            }
            match event {
                WindowEvent::CloseRequested => {
                    log::info!("Close requested, saving workspace and exiting...");
                    varda.save_workspace(&self.layout);
                    event_loop.exit();
                }
                WindowEvent::Resized(new_size) => {
                    let device = &varda.gpu_context().device;
                    if let Some(ws) = &mut self.window_surface {
                        ws.resize(device, new_size);
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
                    if let Some(name) = varda.close_output_window_by_id(window_id) {
                        log::info!("Output window '{}' closed", name);
                    }
                }
                WindowEvent::Resized(new_size) => {
                    varda.resize_output_window_by_id(window_id, new_size);
                }
                _ => {}
            }
        }
    }
}


impl UIRunner {
    /// Register GPU textures with egui for deck previews and main output.
    fn register_preview_textures(&mut self) {
        let Some(varda) = &self.varda else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else { return };
        let context = varda.gpu_context();
        let mixer = varda.mixer_ref();

        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
            for (deck_idx, slot) in ch.decks.iter().enumerate() {
                let tid = egui_renderer.register_native_texture(
                    &context.device, &slot.deck.texture_view, wgpu::FilterMode::Linear,
                );
                self.deck_preview_textures.insert((ch_idx, deck_idx), tid);
            }
        }
        self.main_output_texture = Some(egui_renderer.register_native_texture(
            &context.device, &mixer.composite_view(), wgpu::FilterMode::Linear,
        ));
    }

    /// Re-register GPU textures when deck layout changes.
    fn refresh_textures(&mut self) {
        let Some(varda) = &self.varda else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else { return };
        let context = varda.gpu_context();
        let mixer = varda.mixer_ref();

        if self.main_output_texture.is_none() {
            self.main_output_texture = Some(egui_renderer.register_native_texture(
                &context.device, &mixer.composite_view(), wgpu::FilterMode::Linear,
            ));
        }
        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
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
        // 1. Frame timing + notifications + inputs
        {
            let Some(varda) = self.varda.as_mut() else { return; };
            varda.update_frame_timing();
            varda.update_notifications();
            varda.process_commands();
            varda.process_inputs();
        }

        // 2. Sync egui texture registrations
        self.refresh_textures();

        let Some(window) = self.window else { return };

        // 3. Create pending output windows + refresh monitors
        {
            let Some(varda) = self.varda.as_mut() else { return; };
            varda.create_pending_outputs(event_loop);
            varda.refresh_monitors(event_loop);
        }

        // 4. Collect UI data snapshot (engine → UI, with UI-owned layout state)
        let Some(varda_ref) = self.varda.as_ref() else { return; };
        let ui_data = varda_ref
            .collect_ui_data(&self.layout, &self.deck_preview_textures, self.main_output_texture);

        // 5. Run egui frame
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

        // 6. Apply all UI actions
        // 6a. UI-consumer-owned selection/layout state
        self.layout.apply_selections(&ui_actions);

        // 6b. Engine actions (delegated to VardaApp)
        {
            let Some(varda) = self.varda.as_mut() else { return; };
            let Some(egui_renderer) = self.egui_renderer.as_mut() else { return; };
            let removed_ch = varda.apply_engine_actions(&mut ui_actions, egui_renderer, &mut self.deck_preview_textures);
            varda.apply_ui_actions(&ui_actions);
            varda.apply_output_actions(&ui_actions);
            varda.apply_surface_actions(&ui_actions, self.layout.stage_editor_grid_size);
            varda.apply_device_actions(&ui_actions);
            // Recording/SRT now handled per-output in apply_output_actions()
            varda.apply_clock_actions(&ui_actions);
            let resolution_changed = varda.apply_resolution_change(&ui_actions);
            varda.update_controller_leds();

            // After resolution change, all GPU textures were recreated —
            // re-register them with egui so previews point to the new views.
            if resolution_changed {
                let context = varda.gpu_context();
                let mixer = varda.mixer_ref();
                for (ch_idx, ch) in mixer.channels().iter().enumerate() {
                    for (deck_idx, slot) in ch.decks.iter().enumerate() {
                        let key = (ch_idx, deck_idx);
                        if let Some(&tex_id) = self.deck_preview_textures.get(&key) {
                            egui_renderer.update_egui_texture_from_wgpu_texture(
                                &context.device, &slot.deck.texture_view,
                                wgpu::FilterMode::Linear, tex_id,
                            );
                        }
                    }
                }
                if let Some(main_id) = self.main_output_texture {
                    egui_renderer.update_egui_texture_from_wgpu_texture(
                        &context.device, &mixer.composite_view(),
                        wgpu::FilterMode::Linear, main_id,
                    );
                }
            }

            // Fix up selection state after channel removal
            if let Some(ch_idx) = removed_ch {
                self.layout.fixup_channel_removal(ch_idx);
            }

            if ui_actions.save_requested {
                varda.save_workspace(&self.layout);
                varda.notify_info("💾 Workspace saved");
            }

            varda.handle_file_dialogs(&mut ui_actions, egui_renderer, &mut self.deck_preview_textures);
        }

        // 7. GPU: render mixer + blit + egui overlay + present
        {
            let Some(varda) = self.varda.as_mut() else { return; };
            varda.render_mixer_frame();
        }
        self.submit_frame(window, full_output.shapes, full_output.pixels_per_point, full_output.textures_delta);

        // 8. Render output windows + publish state
        {
            let Some(varda) = self.varda.as_mut() else { return; };
            varda.render_outputs();
            varda.publish_state();
        }
    }

    /// Blit mixer output to screen, overlay egui, and present.
    fn submit_frame(
        &mut self,
        window: &Window,
        shapes: Vec<egui::epaint::ClippedShape>,
        pixels_per_point: f32,
        textures_delta: egui::TexturesDelta,
    ) {
        let Some(varda) = &self.varda else { return };
        let context = varda.gpu_context();
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

        let bind_group = if let Some(blit) = &self.blit_pipeline {
            let mixer = varda.mixer_ref();
            Some(blit.create_bind_group(&context.device, &mixer.composite_view()))
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