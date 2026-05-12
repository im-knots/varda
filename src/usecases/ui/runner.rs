//! UIRunner — windowed delivery layer for the Varda engine.
//!
//! Owns the window, egui state, blit pipeline, texture registrations, and WindowSurface.
//! The engine (`VardaApp`) is owned here and driven each frame.
//! For headless operation (HTTP API, CLI), this module is simply not used.

use crate::app::{AppConfig, VardaApp};
use crate::app::history::HistoryManager;
use crate::app::render::{DeckLoadResult, FileDialogKind, FileDialogResult};
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
    // ── Session config (CLI flags + workspace defaults) ──────────────
    config: AppConfig,

    // ── Window / egui state (delivery layer) ────────────────────────
    window: Option<&'static Window>,
    window_surface: Option<WindowSurface>,
    blit_pipeline: Option<BlitPipeline>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    deck_preview_textures: std::collections::HashMap<(usize, usize), egui::TextureId>,
    channel_preview_textures: std::collections::HashMap<usize, egui::TextureId>,
    output_preview_textures: std::collections::HashMap<usize, egui::TextureId>,
    main_output_texture: Option<egui::TextureId>,
    main_window_id: Option<WindowId>,

    // ── UI-consumer-owned layout/selection state ─────────────────────
    layout: super::UILayoutState,

    // ── File dialog channel (async, non-blocking) ─────────────────────
    file_dialog_tx: std::sync::mpsc::Sender<FileDialogResult>,
    file_dialog_rx: std::sync::mpsc::Receiver<FileDialogResult>,

    // ── Background deck loading channel (async, non-blocking) ────────
    deck_load_tx: std::sync::mpsc::Sender<DeckLoadResult>,
    deck_load_rx: std::sync::mpsc::Receiver<DeckLoadResult>,
    /// Number of deck loads currently in-flight on background threads
    pending_deck_loads: std::sync::Arc<std::sync::atomic::AtomicUsize>,

    // ── Engine (created after GPU init in resumed()) ─────────────────
    varda: Option<VardaApp>,

    // ── Undo/redo history ─────────────────────────────────────────────
    history: HistoryManager,

    // ── Performance: gate publish_state to reduce snapshot overhead ──
    publish_counter: u32,

    // ── HTTP API server (background thread) ──────────────────────────
    api_handle: Option<crate::usecases::api::runner::ApiServerHandle>,

    // ── Headless render timing ──────────────────────────────────────
    last_headless_frame: Option<std::time::Instant>,

    // ── Signal-driven shutdown (SIGINT/SIGTERM) ─────────────────────
    shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl UIRunner {
    pub fn new(config: AppConfig) -> Self {
        let (file_dialog_tx, file_dialog_rx) = std::sync::mpsc::channel();
        let (deck_load_tx, deck_load_rx) = std::sync::mpsc::channel();
        Self {
            config,
            window: None,
            window_surface: None,
            blit_pipeline: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            deck_preview_textures: std::collections::HashMap::new(),
            channel_preview_textures: std::collections::HashMap::new(),
            output_preview_textures: std::collections::HashMap::new(),
            main_output_texture: None,
            main_window_id: None,
            layout: super::UILayoutState::default(),
            file_dialog_tx,
            file_dialog_rx,
            deck_load_tx,
            deck_load_rx,
            pending_deck_loads: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            varda: None,
            history: HistoryManager::new(),
            publish_counter: 0,
            api_handle: None,
            last_headless_frame: None,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Run the UI event loop. Blocks until the window is closed.
    pub fn run(mut self) -> anyhow::Result<()> {
        // Install Ctrl-C handler for graceful shutdown (especially useful in headless)
        let flag = self.shutdown_flag.clone();
        let _ = ctrlc::set_handler(move || {
            log::info!("Received interrupt signal, shutting down...");
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut self)
            .map_err(|e| anyhow::anyhow!("Event loop error: {:?}", e))?;
        Ok(())
    }
}

impl ApplicationHandler for UIRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Guard re-entry (resumed can be called multiple times on some platforms)
        if self.varda.is_some() { return; }

        let gpu = if self.config.headless {
            // Headless: no main window, no egui — GPU without window surface
            log::info!("Headless mode: skipping main window creation");
            match GpuContext::new_headless() {
                Ok(gpu) => gpu,
                Err(e) => { log::error!("Failed to create headless GPU context: {}", e); event_loop.exit(); return; }
            }
        } else {
            // Windowed: create main UI window + egui
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
            gpu
        };

        // Create engine now that GPU is ready
        let mut varda = match VardaApp::new(gpu, &self.config) {
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
        self.history.clear();

        // Start HTTP API server on background thread
        if self.api_handle.is_none() {
            self.api_handle = crate::usecases::api::runner::start(
                self.config.api_port,
                varda.command_sender(),
                varda.state_reader(),
            );
        }

        self.varda = Some(varda);

        // Register GPU textures with egui for previews (windowed only)
        if !self.config.headless {
            self.register_preview_textures();
        }
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
                    if let Some(api) = self.api_handle.take() {
                        api.shutdown();
                    }
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.config.headless { return; }

        // Headless FPS throttle: sleep to maintain target FPS
        let frame_budget = std::time::Duration::from_secs_f64(1.0 / self.config.target_fps as f64);
        if let Some(last) = self.last_headless_frame {
            let elapsed = last.elapsed();
            if elapsed < frame_budget {
                std::thread::sleep(frame_budget - elapsed);
            }
        }
        self.last_headless_frame = Some(std::time::Instant::now());

        // Drive the engine (same as windowed render but without UI/egui)
        self.render_headless(event_loop);
    }
}


impl UIRunner {
    /// Register GPU textures with egui for deck/channel/output previews and main output.
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
            // Channel composite preview
            let ch_tid = egui_renderer.register_native_texture(
                &context.device, &ch.composite_view, wgpu::FilterMode::Linear,
            );
            self.channel_preview_textures.insert(ch_idx, ch_tid);
        }
        self.main_output_texture = Some(egui_renderer.register_native_texture(
            &context.device, &mixer.composite_view(), wgpu::FilterMode::Linear,
        ));
        // Output preview textures — resolve source view for live preview
        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
            let view = Self::output_preview_view(output, mixer);
            let tid = egui_renderer.register_native_texture(
                &context.device, view, wgpu::FilterMode::Linear,
            );
            self.output_preview_textures.insert(out_idx, tid);
        }
    }

    /// Resolve the texture view to use for an output preview.
    /// Windowed outputs use the mixer composite; headless outputs resolve their source.
    fn output_preview_view<'a>(
        output: &'a crate::renderer::context::UnifiedOutput,
        mixer: &'a crate::mixer::Mixer,
    ) -> &'a wgpu::TextureView {
        use crate::renderer::context::{UnifiedOutput, OutputSource};
        match output {
            UnifiedOutput::Window(_) => mixer.composite_view(),
            UnifiedOutput::Headless(h) => match &h.source {
                OutputSource::Master => mixer.composite_view(),
                OutputSource::Channel(idx) => mixer.channels().get(*idx)
                    .map(|c| &c.composite_view)
                    .unwrap_or_else(|| mixer.composite_view()),
                OutputSource::Deck(ch, dk) => mixer.channels().get(*ch)
                    .and_then(|c| c.decks.get(*dk))
                    .map(|s| &s.deck.texture_view)
                    .unwrap_or_else(|| mixer.composite_view()),
                OutputSource::Channels(indices) => {
                    let mut sorted = indices.clone();
                    sorted.sort();
                    sorted.dedup();
                    mixer.get_sub_mix_view(&sorted)
                        .unwrap_or_else(|| mixer.composite_view())
                }
            },
        }
    }

    /// Re-register GPU textures when deck/channel/output layout changes.
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
            if !self.channel_preview_textures.contains_key(&ch_idx) {
                let tid = egui_renderer.register_native_texture(
                    &context.device, &ch.composite_view, wgpu::FilterMode::Linear,
                );
                self.channel_preview_textures.insert(ch_idx, tid);
            }
        }
        // Register any new output preview textures
        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
            if !self.output_preview_textures.contains_key(&out_idx) {
                let view = Self::output_preview_view(output, mixer);
                let tid = egui_renderer.register_native_texture(
                    &context.device, view, wgpu::FilterMode::Linear,
                );
                self.output_preview_textures.insert(out_idx, tid);
            }
        }
    }

    /// Headless render loop — engine processing without UI/egui.
    fn render_headless(&mut self, event_loop: &ActiveEventLoop) {
        let Some(varda) = self.varda.as_mut() else { return; };

        // Check for shutdown request (from API or SIGINT/SIGTERM)
        if varda.shutdown_requested || self.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
            log::info!("Shutdown requested, saving workspace and exiting...");
            varda.save_workspace(&self.layout);
            if let Some(api) = self.api_handle.take() {
                api.shutdown();
            }
            event_loop.exit();
            return;
        }

        varda.update_frame_timing();
        varda.update_notifications();
        varda.process_commands();
        varda.process_inputs();

        // Create pending output windows (API-driven in headless)
        varda.create_pending_outputs(event_loop);
        varda.refresh_monitors(event_loop);

        // GPU render (mixer compositing)
        varda.render_mixer_frame();

        // Render output windows + publish state
        varda.render_outputs();
        self.publish_counter += 1;
        if self.publish_counter % 10 == 0 {
            varda.publish_state();
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
        let mut ui_data = varda_ref
            .collect_ui_data(&self.layout, &self.deck_preview_textures, &self.channel_preview_textures, &self.output_preview_textures, self.main_output_texture);
        ui_data.can_undo = self.history.can_undo();
        ui_data.can_redo = self.history.can_redo();
        ui_data.pending_deck_loads = self.pending_deck_loads.load(std::sync::atomic::Ordering::Relaxed);

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

            // ── Undo/redo: snapshot before undoable mutations ──
            if ui_actions.has_undoable_action() {
                let snapshot = crate::persistence::snapshot_scene(
                    varda.mixer_ref(), varda.render_width(), varda.render_height(),
                );
                self.history.push(snapshot);
            }

            // Intercept shader_to_add: resolve and route to background loading
            if let Some((ch_idx, gen_idx)) = ui_actions.shader_to_add.take() {
                if let Some(shader) = varda.resolve_generator(gen_idx) {
                    let context = varda.gpu_context();
                    VardaApp::spawn_deck_loads(
                        &self.deck_load_tx,
                        context,
                        &self.pending_deck_loads,
                        varda.render_width(),
                        varda.render_height(),
                        Vec::new(),
                        Vec::new(),
                        vec![(ch_idx, shader)],
                    );
                }
            }

            let removed_ch = varda.apply_engine_actions(&mut ui_actions, egui_renderer, &mut self.deck_preview_textures);

            // ── Drain MIDI-triggered global actions ──
            if std::mem::take(&mut varda.midi_pending_undo) {
                ui_actions.undo_requested = true;
            }
            if std::mem::take(&mut varda.midi_pending_redo) {
                ui_actions.redo_requested = true;
            }
            if std::mem::take(&mut varda.midi_pending_save) {
                ui_actions.save_requested = true;
            }

            // ── Undo/redo: diff-apply from history ──
            if ui_actions.undo_requested || ui_actions.redo_requested {
                let current = crate::persistence::snapshot_scene(
                    varda.mixer_ref(), varda.render_width(), varda.render_height(),
                );
                let target = if ui_actions.undo_requested {
                    self.history.undo(current)
                } else {
                    self.history.redo(current)
                };
                if let Some(config) = target {
                    let rw = varda.render_width();
                    let rh = varda.render_height();
                    let (warnings, structural_changed) = varda.apply_scene_diff(&config, rw, rh);
                    for w in &warnings {
                        log::warn!("Undo/redo restore warning: {}", w);
                    }
                    let label = if ui_actions.undo_requested { "↩ Undo" } else { "↪ Redo" };
                    varda.notify_info(label);

                    if structural_changed {
                        // Structural change: re-register all deck + channel preview textures
                        self.deck_preview_textures.clear();
                        self.channel_preview_textures.clear();
                        let context = varda.gpu_context();
                        let mixer = varda.mixer_ref();
                        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
                            for (deck_idx, slot) in ch.decks.iter().enumerate() {
                                let tex_id = egui_renderer.register_native_texture(
                                    &context.device, &slot.deck.texture_view,
                                    wgpu::FilterMode::Linear,
                                );
                                self.deck_preview_textures.insert((ch_idx, deck_idx), tex_id);
                            }
                            let ch_tid = egui_renderer.register_native_texture(
                                &context.device, &ch.composite_view,
                                wgpu::FilterMode::Linear,
                            );
                            self.channel_preview_textures.insert(ch_idx, ch_tid);
                        }
                        if let Some(main_id) = self.main_output_texture {
                            egui_renderer.update_egui_texture_from_wgpu_texture(
                                &context.device, &varda.mixer_ref().composite_view(),
                                wgpu::FilterMode::Linear, main_id,
                            );
                        }
                        // Re-register output preview textures
                        self.output_preview_textures.clear();
                        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
                            let view = Self::output_preview_view(output, mixer);
                            let tid = egui_renderer.register_native_texture(
                                &context.device, view,
                                wgpu::FilterMode::Linear,
                            );
                            self.output_preview_textures.insert(out_idx, tid);
                        }
                    }
                }
            }

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
                    if let Some(&ch_tid) = self.channel_preview_textures.get(&ch_idx) {
                        egui_renderer.update_egui_texture_from_wgpu_texture(
                            &context.device, &ch.composite_view,
                            wgpu::FilterMode::Linear, ch_tid,
                        );
                    }
                }
                if let Some(main_id) = self.main_output_texture {
                    egui_renderer.update_egui_texture_from_wgpu_texture(
                        &context.device, &mixer.composite_view(),
                        wgpu::FilterMode::Linear, main_id,
                    );
                }
                // Update output preview textures after resolution change
                for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
                    if let Some(&tid) = self.output_preview_textures.get(&out_idx) {
                        let view = Self::output_preview_view(output, mixer);
                        egui_renderer.update_egui_texture_from_wgpu_texture(
                            &context.device, view,
                            wgpu::FilterMode::Linear, tid,
                        );
                    }
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

            // Spawn file dialogs on background threads (non-blocking)
            if let Some(ch_idx) = ui_actions.open_image_dialog_for_channel.take() {
                VardaApp::open_file_dialog(&self.file_dialog_tx, FileDialogKind::Image, ch_idx);
            }
            if let Some(ch_idx) = ui_actions.open_video_dialog_for_channel.take() {
                VardaApp::open_file_dialog(&self.file_dialog_tx, FileDialogKind::Video, ch_idx);
            }

            // Poll completed file dialog results → spawn background deck loads
            while let Ok(result) = self.file_dialog_rx.try_recv() {
                let mut images = Vec::new();
                let mut videos = Vec::new();
                for path in result.paths {
                    match result.kind {
                        FileDialogKind::Image => images.push((result.ch_idx, path)),
                        FileDialogKind::Video => videos.push((result.ch_idx, path)),
                    }
                }
                if !images.is_empty() || !videos.is_empty() {
                    let context = varda.gpu_context();
                    VardaApp::spawn_deck_loads(
                        &self.deck_load_tx,
                        context,
                        &self.pending_deck_loads,
                        varda.render_width(),
                        varda.render_height(),
                        images,
                        videos,
                        Vec::new(),
                    );
                }
            }

            // Poll completed background deck loads (non-blocking)
            while let Ok(result) = self.deck_load_rx.try_recv() {
                match result.deck {
                    Ok(deck) => {
                        let ch_idx = result.ch_idx;
                        if let Some(ch) = varda.mixer_mut().channel_mut(ch_idx) {
                            let idx = ch.add_deck(deck);
                            log::info!("Background load complete: deck {} to channel {}: {}", idx, ch_idx, result.name);
                        }
                        // Re-borrow for texture registration (separate from mixer borrow)
                        if let Some(ch) = varda.mixer_ref().channels().get(ch_idx) {
                            let idx = ch.decks.len() - 1;
                            let texture_id = egui_renderer.register_native_texture(
                                &varda.gpu_context().device,
                                &ch.decks[idx].deck.texture_view,
                                wgpu::FilterMode::Linear,
                            );
                            self.deck_preview_textures.insert((ch_idx, idx), texture_id);
                        }
                    }
                    Err(e) => log::error!("Background deck load failed for '{}': {}", result.name, e),
                }
            }
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
            self.publish_counter += 1;
            if self.publish_counter % 10 == 0 {
                varda.publish_state();
            }
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