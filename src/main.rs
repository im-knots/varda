use varda::*;
use varda::modulation::ModulationSource;
use varda::renderer::context::{OutputWindow, OutputSource, OutputTarget, SurfaceRenderInfo};
use varda::surface::ContentMapping;
use varda::surface::SurfaceManager;
use varda::ui::{self, UIData, AudioUIData, ModSourceUI, ModAssignmentUI, NotificationUI, OutputWindowUI, SurfaceUI, ShaderParamsUI, ChannelUIInfo, DeckUIInfo, collect_params, RENDER_WIDTH, RENDER_HEIGHT};
use varda::renderer::context::SurfaceAssignment;
use varda::ui::notifications::NotificationSystem;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

struct App {
    window: Option<&'static Window>,
    context: Option<RenderContext>,
    registry: ShaderRegistry,
    mixer: Option<Mixer>,
    blit_pipeline: Option<BlitPipeline>,
    // egui state
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    // Deck preview texture IDs for egui ((ch_idx, deck_idx) -> egui TextureId)
    deck_preview_textures: std::collections::HashMap<(usize, usize), egui::TextureId>,
    // Main output preview texture ID
    main_output_texture: Option<egui::TextureId>,
    // Audio state
    audio_input: Option<AudioInput>,
    audio_textures: Option<AudioTextures>,
    audio_data: AudioData,
    // OSC state
    osc_receiver: Option<OscReceiver>,
    // MIDI state
    midi_devices: Option<midi::MidiDeviceManager>,
    midi_mappings: midi::MidiMappingStore,
    apc_mini_mgr: midi::apc_mini::ApcMiniManager,
    // Notification system
    notifications: NotificationSystem,
    // Currently selected deck for bottom bar detail view
    selected_deck: Option<(usize, usize)>,
    // Currently selected channel for bottom bar detail view
    selected_channel: Option<usize>,
    // Whether master output is selected for bottom bar detail view
    selected_master: bool,
    // Output windows for multi-output
    output_windows: Vec<OutputWindow>,
    // Main window ID for event dispatch
    main_window_id: Option<WindowId>,
    // Pending output window creation (deferred to next resumed/event cycle since we need ActiveEventLoop)
    pending_output_creates: Vec<()>,
    // Cached monitor handles (refreshed each frame from event_loop)
    cached_monitors: Vec<(String, winit::monitor::MonitorHandle)>,
    // Surface manager for 2D stage layout
    surface_manager: SurfaceManager,
    // Stage editor state
    stage_editor_open: bool,
    stage_editor_grid_size: f32,
    stage_editor_snap: bool,
    library_panel_open: bool,
    // Calibration card textures (one per color, created on init)
    calibration_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    // Camera capture manager (shared camera sessions)
    camera_manager: varda::camera::CameraManager,
    // Workspace persistence
    workspace: varda::persistence::Workspace,
}

impl App {
    fn new() -> Self {
        let mut registry = ShaderRegistry::new();

        // Add shader library directory
        if let Err(e) = registry.add_library_path("shaders") {
            log::warn!("Failed to add shaders path: {}", e);
        }

        // Scan for shaders
        match registry.scan() {
            Ok(count) => log::info!("Loaded {} shaders", count),
            Err(e) => log::error!("Failed to scan shaders: {}", e),
        }

        // Start watching for shader file changes
        if let Err(e) = registry.start_watching() {
            log::warn!("Failed to start shader hot-reload: {}", e);
        }

        // Initialize audio input
        let audio_input = match AudioInput::new() {
            Ok(input) => {
                log::info!("Audio input initialized");
                Some(input)
            }
            Err(e) => {
                log::warn!("Failed to initialize audio input: {}", e);
                None
            }
        };

        // Try to start OSC receiver on port 9000
        let osc_receiver = match OscReceiver::new(9000) {
            Ok(osc) => {
                log::info!("OSC receiver started on port 9000");
                Some(osc)
            }
            Err(e) => {
                log::warn!("Failed to start OSC receiver: {}", e);
                None
            }
        };

        // Initialize MIDI device manager (handles N devices, input + output)
        let mut apc_mini_mgr = midi::apc_mini::ApcMiniManager::new();
        let midi_devices = match midi::MidiDeviceManager::new() {
            Ok(mgr) => {
                let count = mgr.devices.len();
                log::info!("MIDI initialized: {} device(s)", count);
                apc_mini_mgr.sync_devices(&mgr);
                Some(mgr)
            }
            Err(e) => {
                log::warn!("Failed to initialize MIDI: {}", e);
                None
            }
        };

        Self {
            window: None,
            context: None,
            registry,
            mixer: None,
            blit_pipeline: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            deck_preview_textures: std::collections::HashMap::new(),
            main_output_texture: None,
            audio_input,
            audio_textures: None,
            audio_data: AudioData::default(),
            osc_receiver,
            midi_devices,
            midi_mappings: midi::MidiMappingStore::new(),
            apc_mini_mgr,
            notifications: NotificationSystem::new(),
            selected_deck: None,
            selected_channel: None,
            selected_master: false,
            output_windows: Vec::new(),
            main_window_id: None,
            pending_output_creates: Vec::new(),
            cached_monitors: Vec::new(),
            surface_manager: SurfaceManager::new(),
            stage_editor_open: false,
            stage_editor_grid_size: 0.05,
            stage_editor_snap: true,
            library_panel_open: true,
            calibration_textures: Vec::new(),
            camera_manager: varda::camera::CameraManager::new(),
            workspace: varda::persistence::Workspace::from_cwd()
                .unwrap_or_else(|_| varda::persistence::Workspace::new(std::path::PathBuf::from("."))),
        }
    }
}

impl App {
    /// Save the entire workspace to `.varda/`.
    fn save_workspace(&self) {
        if let Err(e) = self.workspace.ensure_dir() {
            log::error!("Failed to create .varda directory: {}", e);
            return;
        }

        // Save scene (show-specific: channels, decks, effects, modulation)
        if let Some(mixer) = &self.mixer {
            let scene = varda::persistence::snapshot_scene(mixer);
            match scene.save(self.workspace.scene_path()) {
                Ok(()) => log::info!("Saved scene to {}", self.workspace.scene_path().display()),
                Err(e) => log::error!("Failed to save scene: {}", e),
            }
        }

        // Save MIDI mappings
        if let Some(midi) = &self.midi_devices {
            let midi_config = self.midi_mappings.to_config(&midi.devices);
            match midi_config.save(self.workspace.midi_path()) {
                Ok(()) => log::info!("Saved MIDI mappings to {}", self.workspace.midi_path().display()),
                Err(e) => log::error!("Failed to save MIDI config: {}", e),
            }
        }

        // Save stage (venue-specific: surfaces, outputs, editor prefs)
        let stage = varda::persistence::snapshot_stage(
            &self.surface_manager,
            &self.output_windows,
            self.stage_editor_grid_size,
            self.stage_editor_snap,
            self.library_panel_open,
            self.stage_editor_open,
        );
        match stage.save(self.workspace.stage_path()) {
            Ok(()) => log::info!("Saved stage to {}", self.workspace.stage_path().display()),
            Err(e) => log::error!("Failed to save stage: {}", e),
        }
    }

    /// Load workspace from `.varda/` if it exists. Called after RenderContext is ready.
    fn load_workspace(&mut self, context: &RenderContext) {
        if !self.workspace.exists() {
            log::info!("No .varda/ directory found, starting fresh");
            return;
        }

        // Load stage first (venue-specific: surfaces, outputs, editor prefs)
        if self.workspace.has_stage() {
            match varda::persistence::StagePrefs::load(self.workspace.stage_path()) {
                Ok(prefs) => {
                    self.stage_editor_grid_size = prefs.grid_size;
                    self.stage_editor_snap = prefs.snap;
                    self.library_panel_open = prefs.library_panel_open;
                    self.stage_editor_open = prefs.stage_editor_open;
                    self.surface_manager = prefs.surfaces;
                    // Output configs are stored but windows created lazily
                    // (need ActiveEventLoop, handled in create_pending_outputs)
                    for _output_config in &prefs.outputs {
                        self.pending_output_creates.push(());
                    }
                    log::info!("Loaded stage with {} surfaces, {} outputs",
                        self.surface_manager.surfaces.len(),
                        prefs.outputs.len());
                }
                Err(e) => log::warn!("Failed to load stage: {}", e),
            }
        }

        // Load scene (show-specific: channels, decks, effects, modulation)
        if self.workspace.has_scene() {
            match varda::scene::SceneConfig::load(self.workspace.scene_path()) {
                Ok(scene_config) => {
                    match varda::persistence::restore_scene(&scene_config, context, &self.registry) {
                        Ok(result) => {
                            self.mixer = Some(result.mixer);
                            for warn in &result.warnings {
                                self.notifications.warn(warn.clone());
                            }
                            log::info!("Loaded scene with {} channels",
                                scene_config.channels.len());
                        }
                        Err(e) => {
                            log::error!("Failed to restore scene: {}", e);
                            self.notifications.error(format!("Failed to load scene: {}", e));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to load scene file: {}", e);
                    self.notifications.warn(format!("Failed to load scene: {}", e));
                }
            }
        }

        // Load MIDI mappings
        if self.workspace.has_midi() {
            match varda::midi::MidiConfig::load(self.workspace.midi_path()) {
                Ok(midi_config) => {
                    if let Some(midi) = &self.midi_devices {
                        self.midi_mappings.load_from_config(&midi_config, &midi.devices);
                        log::info!("Loaded {} MIDI mappings", midi_config.mappings.len());
                    } else {
                        log::info!("MIDI config found but no MIDI devices connected, mappings deferred");
                    }
                }
                Err(e) => log::warn!("Failed to load MIDI config: {}", e),
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window_attrs = Window::default_attributes()
                .with_title("Varda VJ Software")
                .with_inner_size(winit::dpi::LogicalSize::new(1920, 1080));

            match event_loop.create_window(window_attrs) {
                Ok(window) => {
                    log::info!("Window created");

                    // Leak window for 'static lifetime
                    let window_static: &'static Window = Box::leak(Box::new(window));
                    self.main_window_id = Some(window_static.id());
                    self.window = Some(window_static);

                    // Create render context
                    match pollster::block_on(RenderContext::new(window_static)) {
                        Ok(context) => {
                            log::info!("Varda initialized successfully!");
                            log::info!("Window size: {}x{}", context.size.width, context.size.height);
                            log::info!("Loaded {} shaders", self.registry.count());

                            // Create blit pipeline
                            match BlitPipeline::new(&context.device, context.surface_config.format) {
                                Ok(blit_pipeline) => {
                                    self.blit_pipeline = Some(blit_pipeline);
                                }
                                Err(e) => {
                                    log::error!("Failed to create blit pipeline: {}", e);
                                }
                            }

                            // Try to load workspace from .varda/, or create fresh mixer
                            self.load_workspace(&context);

                            if self.mixer.is_none() {
                                // No saved scene — create default Mixer
                                match Mixer::new(&context, RENDER_WIDTH, RENDER_HEIGHT) {
                                    Ok(mut mixer) => {
                                        // Load first available shader into channel A
                                        let generators = self.registry.generators();
                                        let shader_to_load = generators.iter()
                                            .find(|s| s.metadata.passes.as_ref().map(|p| !p.is_empty()).unwrap_or(false))
                                            .or_else(|| generators.first());

                                        if let Some(shader) = shader_to_load {
                                            log::info!("Loading shader: {} (has_passes: {})",
                                                shader.name(),
                                                shader.metadata.passes.as_ref().map(|p| !p.is_empty()).unwrap_or(false));
                                            match Deck::new(&context, (*shader).clone(), RENDER_WIDTH, RENDER_HEIGHT) {
                                                Ok(deck) => {
                                                    if let Some(ch) = mixer.channel_mut(0) {
                                                        ch.add_deck(deck);
                                                        log::info!("Created deck in channel A with shader: {}", shader.name());
                                                    }
                                                }
                                                Err(e) => {
                                                    log::error!("Failed to create deck: {}", e);
                                                }
                                            }
                                        }
                                        self.mixer = Some(mixer);
                                    }
                                    Err(e) => {
                                        log::error!("Failed to create mixer: {}", e);
                                    }
                                }
                            }

                            // Initialize egui
                            self.egui_state = Some(egui_winit::State::new(
                                self.egui_ctx.clone(),
                                egui::ViewportId::ROOT,
                                window_static,
                                Some(window_static.scale_factor() as f32),
                                None,
                                Some(2 * 1024), // Max texture size
                            ));

                            self.egui_renderer = Some(egui_wgpu::Renderer::new(
                                &context.device,
                                context.surface_config.format,
                                egui_wgpu::RendererOptions::default(),
                            ));

                            log::info!("egui initialized");

                            // Create audio textures
                            self.audio_textures = Some(AudioTextures::new(&context.device));
                            log::info!("Audio textures created");

                            // Create calibration card textures (8 colors for distinct surfaces)
                            self.calibration_textures = varda::renderer::context::create_calibration_textures(
                                &context.device,
                                &context.queue,
                                8,
                            );
                            log::info!("Calibration card textures created ({} colors)", self.calibration_textures.len());

                            // Register deck textures with egui for previews
                            if let (Some(mixer), Some(egui_renderer)) = (&self.mixer, &mut self.egui_renderer) {
                                // Register deck preview textures from all channels
                                for (ch_idx, ch) in mixer.channels.iter().enumerate() {
                                    for (deck_idx, deck_slot) in ch.decks.iter().enumerate() {
                                        let texture_id = egui_renderer.register_native_texture(
                                            &context.device,
                                            &deck_slot.deck.texture_view,
                                            wgpu::FilterMode::Linear,
                                        );
                                        self.deck_preview_textures.insert((ch_idx, deck_idx), texture_id);
                                        log::info!("Registered ch{} deck {} texture for preview", ch_idx, deck_idx);
                                    }
                                }
                                // Register main output texture
                                let main_texture_id = egui_renderer.register_native_texture(
                                    &context.device,
                                    &mixer.composite_view,
                                    wgpu::FilterMode::Linear,
                                );
                                self.main_output_texture = Some(main_texture_id);
                                log::info!("Registered main output texture for preview");
                            }

                            self.context = Some(context);
                        }
                        Err(e) => {
                            log::error!("Failed to create render context: {}", e);
                            event_loop.exit();
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to create window: {}", e);
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let is_main = self.main_window_id == Some(window_id);

        if is_main {
            // Main window: pass events to egui first
            if let (Some(window), Some(egui_state)) = (self.window, &mut self.egui_state) {
                let response = egui_state.on_window_event(window, &event);
                if response.consumed {
                    return;
                }
            }

            match event {
                WindowEvent::CloseRequested => {
                    log::info!("Close requested, saving workspace and exiting...");
                    self.save_workspace();
                    event_loop.exit();
                }
                WindowEvent::Resized(new_size) => {
                    if let Some(context) = &mut self.context {
                        context.resize(new_size);
                        log::info!("Window resized to: {}x{}", new_size.width, new_size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    self.render(event_loop);
                    if let Some(window) = self.window {
                        window.request_redraw();
                    }
                }
                _ => {}
            }
        } else {
            // Output window events
            match event {
                WindowEvent::CloseRequested => {
                    // Find and remove the output window, destroying the OS window
                    if let Some(idx) = self.output_windows.iter().position(|o| o.window.id() == window_id) {
                        let name = self.output_windows[idx].name.clone();
                        let output = self.output_windows.remove(idx);
                        output.destroy();
                        log::info!("Output window '{}' closed", name);
                    }
                }
                WindowEvent::Resized(new_size) => {
                    if let Some(context) = &self.context {
                        if let Some(output) = self.output_windows.iter_mut().find(|o| o.window.id() == window_id) {
                            output.resize(&context.device, new_size);
                        }
                    }
                }
                WindowEvent::RedrawRequested => {
                    // Output windows are rendered in the main render loop
                }
                _ => {}
            }
        }
    }
}

impl App {
    /// Collect all data needed by the UI into a read-only snapshot
    fn collect_ui_data(&self) -> UIData {
        let mut generators: Vec<(String, usize)> = self.registry.generators().iter()
            .enumerate().map(|(i, s)| (s.name(), i)).collect();
        generators.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        let mut filters: Vec<(String, usize)> = self.registry.filters().iter()
            .enumerate().map(|(i, s)| (s.name(), i)).collect();
        filters.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        let shader_count = self.registry.count();

        // Collect per-channel data
        let channels: Vec<ChannelUIInfo> = self.mixer.as_ref()
            .map(|m| m.channels.iter().enumerate().map(|(ch_idx, ch)| {
                let decks = ch.decks.iter().enumerate().map(|(deck_idx, slot)| {
                    let gen_params = ShaderParamsUI {
                        shader_name: slot.deck.source_name().to_string(),
                        params: collect_params(&slot.deck.generator_params),
                    };
                    let effects = slot.deck.effects.iter()
                        .map(|e| {
                            let params = ShaderParamsUI {
                                shader_name: e.shader.name(),
                                params: collect_params(&e.params),
                            };
                            (e.shader.name(), e.enabled, params)
                        })
                        .collect();
                    DeckUIInfo {
                        deck_idx,
                        name: slot.deck.source_name().to_string(),
                        opacity: slot.opacity,
                        blend_mode: slot.blend_mode,
                        solo: slot.solo,
                        mute: slot.mute,
                        scaling_mode: slot.deck.scaling_mode(),
                        generator: gen_params,
                        effects,
                    }
                }).collect();
                let ch_effects = ch.effects.iter()
                    .map(|e| {
                        let params = ShaderParamsUI {
                            shader_name: e.shader.name(),
                            params: collect_params(&e.params),
                        };
                        (e.shader.name(), e.enabled, params)
                    })
                    .collect();
                ChannelUIInfo {
                    ch_idx,
                    name: ch.name.clone(),
                    opacity: ch.opacity,
                    blend_mode: ch.blend_mode,
                    decks,
                    effects: ch_effects,
                }
            }).collect())
            .unwrap_or_default();

        let master_effect_info = self.mixer.as_ref()
            .map(|m| m.master_effects.iter().map(|e| {
                let params = ShaderParamsUI {
                    shader_name: e.shader.name(),
                    params: collect_params(&e.params),
                };
                (e.shader.name(), e.enabled, params)
            }).collect())
            .unwrap_or_default();

        let modulation_sources = self.mixer.as_ref()
            .map(|m| m.modulation.sources.iter().map(|src| {
                match src {
                    ModulationSource::LFO { waveform, frequency, phase, amplitude, bipolar } => {
                        ModSourceUI::LFO { waveform: *waveform, frequency: *frequency, phase: *phase, amplitude: *amplitude, bipolar: *bipolar }
                    }
                    ModulationSource::AudioBand { band, smoothing } => {
                        ModSourceUI::Audio { band: *band, smoothing: *smoothing }
                    }
                    ModulationSource::ADSR { attack, decay, sustain, release, stage, .. } => {
                        ModSourceUI::ADSR { attack: *attack, decay: *decay, sustain: *sustain, release: *release, stage: *stage }
                    }
                    ModulationSource::StepSequencer { steps, rate, interpolation, bipolar } => {
                        ModSourceUI::StepSequencer { steps: steps.clone(), rate: *rate, interpolation: *interpolation, bipolar: *bipolar }
                    }
                }
            }).collect())
            .unwrap_or_default();

        let modulation_current_values = self.mixer.as_ref()
            .map(|m| m.modulation.current_values().to_vec())
            .unwrap_or_default();

        let modulation_assignments = self.mixer.as_ref()
            .map(|m| m.modulation.assignments.iter().map(|(k, v)| {
                (k.clone(), v.iter().map(|pm| ModAssignmentUI {
                    source_idx: pm.source_idx,
                    amount: pm.amount,
                }).collect())
            }).collect())
            .unwrap_or_default();

        let audio = AudioUIData {
            level: self.audio_data.level,
            bass: self.audio_data.bass(),
            mid: self.audio_data.mid(),
            treble: self.audio_data.treble(),
            bpm: self.audio_data.bpm,
            beat_phase: self.audio_data.beat_phase(),
            enabled: self.audio_input.is_some(),
        };

        let notifications = self.notifications.visible().iter().map(|n| NotificationUI {
            level: n.level,
            message: n.message.clone(),
            progress: n.progress(),
        }).collect();

        // Crossfader state
        let (crossfader, auto_crossfade_active, auto_crossfade_progress) = self.mixer.as_ref()
            .map(|m| {
                let active = m.is_crossfading();
                let progress = m.auto_crossfade.as_ref().map_or(0.0, |a| a.progress());
                (m.crossfader, active, progress)
            })
            .unwrap_or((0.0, false, 0.0));

        UIData {
            generators,
            filters,
            shader_count,
            channels,
            master_effect_info,
            modulation_sources,
            modulation_current_values,
            modulation_assignments,
            audio,
            deck_preview_textures: self.deck_preview_textures.clone(),
            main_output_texture: self.main_output_texture,
            notifications,
            crossfader,
            auto_crossfade_active,
            auto_crossfade_progress,
            midi_learn_active: self.midi_mappings.learn_mode,
            midi_learn_target: self.midi_mappings.learn_target.clone(),
            transition_names: self.registry.transitions().iter().map(|s| s.name()).collect(),
            active_transition_name: self.mixer.as_ref()
                .and_then(|m| m.active_transition.as_ref())
                .map(|t| t.name.clone()),
            selected_deck: self.selected_deck,
            selected_channel: self.selected_channel,
            selected_master: self.selected_master,
            output_windows: self.output_windows.iter().map(|o| OutputWindowUI {
                name: o.name.clone(),
                target_label: format!("{}", o.target),
                is_on_display: matches!(o.target, OutputTarget::Display { .. }),
                surface_assignments: o.surface_assignments.iter().map(|a| {
                    let surface_name = self.surface_manager.surfaces.get(a.surface_idx)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| format!("Surface {}", a.surface_idx));
                    ui::SurfaceAssignmentUI {
                        surface_idx: a.surface_idx,
                        surface_name,
                        warp_corners: a.warp_corners,
                        enabled: a.enabled,
                    }
                }).collect(),
                calibration_mode: o.calibration_mode,
            }).collect(),
            surfaces: self.surface_manager.surfaces.iter().map(|s| SurfaceUI {
                name: s.name.clone(),
                vertices: s.vertices.clone(),
                extra_contours: s.extra_contours.clone(),
                source: s.source.clone(),
                content_mapping: s.content_mapping,
                output_type: s.output_type,
                circle_hint: s.circle_hint,
            }).collect(),
            stage_editor_open: self.stage_editor_open,
            library_panel_open: self.library_panel_open,
            stage_editor_grid_size: self.stage_editor_grid_size,
            stage_editor_snap: self.stage_editor_snap,
            available_monitors: self.cached_monitors.iter().enumerate().map(|(i, (name, handle))| {
                let size = handle.size();
                ui::MonitorInfo {
                    name: name.clone(),
                    index: i,
                    width: size.width,
                    height: size.height,
                }
            }).collect(),
            midi_devices: self.midi_devices.as_ref().map(|mgr| {
                mgr.device_list().iter().map(|d| ui::MidiDeviceUI {
                    id: d.id,
                    name: d.name.clone(),
                    enabled: d.enabled,
                    has_output: d.has_output,
                    profile: format!("{:?}", d.profile),
                }).collect()
            }).unwrap_or_default(),
            midi_mappings: {
                let mappings = self.midi_mappings.sorted_mappings();
                mappings.iter().map(|(key, path)| {
                    let dev_name = self.midi_devices.as_ref()
                        .and_then(|mgr| mgr.device(key.device_id()))
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| format!("Device {}", key.device_id()));
                    ui::MidiMappingUI {
                        key: *key,
                        key_display: format!("{}", key),
                        device_name: dev_name,
                        param_path: path.clone(),
                    }
                }).collect()
            },
            cameras: self.camera_manager.devices().iter()
                .map(|d| (d.name.clone(), d.id))
                .collect(),
        }
    }

    /// Process all external inputs: shader hot-reload, audio, OSC, MIDI.
    fn process_inputs(&mut self) {
        // Poll for shader file changes (hot-reload)
        let shader_events = self.registry.poll_changes();
        for event in &shader_events {
            match event {
                ShaderEvent::Changed(path) => {
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
                    self.notifications.info(format!("Shader reloaded: {}", name));
                }
                ShaderEvent::Removed(path) => {
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
                    self.notifications.warn(format!("Shader removed: {}", name));
                }
                ShaderEvent::Error(path, err) => {
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
                    self.notifications.error(format!("Shader error in {}: {}", name, err));
                }
            }
        }

        // Poll for audio data
        if let Some(audio_input) = &self.audio_input {
            if let Some(data) = audio_input.get_latest() {
                self.audio_data = data;
            }
        }

        // Update audio textures
        if let Some(context) = &self.context {
            if let Some(audio_textures) = &self.audio_textures {
                audio_textures.update(&context.queue, &self.audio_data);
            }
        }

        // Process OSC messages (mapped to channel A for now)
        if let Some(osc) = &self.osc_receiver {
            while let Some(ctrl) = osc.try_recv() {
                match ctrl {
                    OscControl::SetOpacity(deck_idx, val) => {
                        if let Some(mixer) = &mut self.mixer {
                            if let Some(ch) = mixer.channel_mut(0) {
                                ch.set_deck_opacity(deck_idx, val);
                            }
                        }
                    }
                    OscControl::SetSolo(deck_idx, enabled) => {
                        if let Some(mixer) = &mut self.mixer {
                            if let Some(ch) = mixer.channel_mut(0) {
                                ch.set_deck_solo(deck_idx, enabled);
                            }
                        }
                    }
                    OscControl::SetMute(deck_idx, enabled) => {
                        if let Some(mixer) = &mut self.mixer {
                            if let Some(ch) = mixer.channel_mut(0) {
                                ch.set_deck_mute(deck_idx, enabled);
                            }
                        }
                    }
                    OscControl::Unknown(addr, args) => {
                        log::debug!("Unknown OSC: {} {:?}", addr, args);
                    }
                    _ => {}
                }
            }
        }

        // Process MIDI messages → apply to mixer via mapping store
        if let Some(midi) = &self.midi_devices {
            while let Some(msg) = midi.try_recv() {
                let key = msg.mapping_key();
                let value = msg.normalized_value();

                // Learn mode: map next MIDI input to the learn target
                if self.midi_mappings.learn_mode {
                    self.midi_mappings.process_learn(key);
                    // Fall through to apply — mapped controls should still move sliders
                }

                // Apply mapped value to mixer (both normal and learn mode)
                if let Some(path) = self.midi_mappings.get(&key).cloned() {
                    if let Some(mixer) = &mut self.mixer {
                        midi::apply_midi_to_param(mixer, &path, value);
                    }
                } else if !self.midi_mappings.learn_mode {
                    log::debug!("Unmapped MIDI: {} value={:.2}", key, value);
                }
            }
        }
    }

    /// Re-register GPU textures with egui so preview IDs stay valid.
    fn refresh_textures(&mut self) {
        let Some(context) = &self.context else { return };
        if let (Some(mixer), Some(egui_renderer)) = (&self.mixer, &mut self.egui_renderer) {
            // Main output texture
            if let Some(old_id) = self.main_output_texture.take() {
                egui_renderer.free_texture(&old_id);
            }
            let main_texture_id = egui_renderer.register_native_texture(
                &context.device,
                &mixer.composite_view,
                wgpu::FilterMode::Linear,
            );
            self.main_output_texture = Some(main_texture_id);

            // Deck preview textures from all channels
            for (ch_idx, ch) in mixer.channels.iter().enumerate() {
                for (deck_idx, deck_slot) in ch.decks.iter().enumerate() {
                    let key = (ch_idx, deck_idx);
                    if let Some(old_id) = self.deck_preview_textures.get(&key) {
                        egui_renderer.free_texture(old_id);
                    }
                    let new_id = egui_renderer.register_native_texture(
                        &context.device,
                        &deck_slot.deck.texture_view,
                        wgpu::FilterMode::Linear,
                    );
                    self.deck_preview_textures.insert(key, new_id);
                }
            }
        }
    }

    /// Apply UI-driven state changes that don't touch the engine (selection, MIDI learn, notifications).
    fn apply_ui_actions(&mut self, ui_actions: &ui::UIActions) {
        // Handle deck selection (clears channel/master selection)
        if let Some(sel) = ui_actions.select_deck {
            self.selected_deck = Some(sel);
            self.selected_channel = None;
            self.selected_master = false;
        }

        // Handle channel selection (clears deck/master selection)
        if let Some(ch) = ui_actions.select_channel {
            self.selected_channel = Some(ch);
            self.selected_deck = None;
            self.selected_master = false;
        }

        // Handle master selection (clears deck/channel selection)
        if ui_actions.select_master {
            self.selected_master = true;
            self.selected_deck = None;
            self.selected_channel = None;
        }

        // Handle MIDI learn actions from UI
        if ui_actions.midi_learn_toggle {
            self.midi_mappings.toggle_learn();
        }
        if let Some(ref path) = ui_actions.midi_learn_select {
            self.midi_mappings.select_learn_target(path.clone());
        }

        // Handle notification dismissals (process in reverse to keep indices valid)
        let mut dismissals = ui_actions.notifications_to_dismiss.clone();
        dismissals.sort_unstable_by(|a, b| b.cmp(a));
        for idx in dismissals {
            self.notifications.dismiss(idx);
        }

        // Handle stage editor toggles
        if ui_actions.toggle_stage_editor {
            self.stage_editor_open = !self.stage_editor_open;
        }
        if let Some(size) = ui_actions.set_grid_size {
            self.stage_editor_grid_size = size;
        }
        if ui_actions.toggle_snap {
            self.stage_editor_snap = !self.stage_editor_snap;
        }
        if ui_actions.toggle_library_panel {
            self.library_panel_open = !self.library_panel_open;
        }

        // Ctrl+S / Cmd+S: save workspace
        if ui_actions.save_requested {
            self.save_workspace();
            self.notifications.info("💾 Workspace saved".to_string());
        }
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        self.notifications.update();
        self.process_inputs();
        self.refresh_textures();

        let Some(window) = self.window else { return };
        if self.context.is_none() { return; }

        // Create any pending output windows
        self.create_pending_outputs(event_loop);

        // Refresh available monitors
        self.cached_monitors = event_loop.available_monitors()
            .map(|m| {
                let name = m.name().unwrap_or_else(|| "Unknown".to_string());
                (name, m)
            })
            .collect();

        // Collect UI data and run egui frame
        let ui_data = self.collect_ui_data();
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

        // Apply all UI actions (engine state + selection/MIDI learn/notifications)
        self.apply_engine_actions(&mut ui_actions);
        self.apply_ui_actions(&ui_actions);
        self.apply_output_actions(&ui_actions);
        self.apply_surface_actions(&ui_actions);

        // Update APC Mini LEDs based on current state
        if let (Some(mgr), Some(mixer)) = (&self.midi_devices, &self.mixer) {
            self.apc_mini_mgr.update_leds(
                mgr,
                &self.midi_mappings,
                mixer,
                self.midi_mappings.learn_mode,
                self.midi_mappings.learn_target.as_deref(),
            );
        }

        // Handle camera rescan
        if ui_actions.camera_rescan {
            self.camera_manager.scan_devices();
        }

        // Handle MIDI device actions from UI
        if ui_actions.midi_rescan {
            if let Some(mgr) = &mut self.midi_devices {
                if let Err(e) = mgr.scan_devices() {
                    log::warn!("MIDI rescan failed: {}", e);
                }
                self.apc_mini_mgr.sync_devices(mgr);
            }
        }
        for (dev_id, enabled) in &ui_actions.midi_device_toggles {
            if let Some(mgr) = &mut self.midi_devices {
                mgr.set_device_enabled(*dev_id, *enabled);
            }
        }
        if ui_actions.midi_clear_mappings {
            self.midi_mappings.clear_all();
        }
        for key in &ui_actions.midi_remove_mapping {
            self.midi_mappings.remove(key);
        }

        // Handle deferred file dialogs (must happen outside egui frame for macOS Finder focus)
        if let Some(ch_idx) = ui_actions.open_image_dialog_for_channel {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "tiff", "tga", "webp"])
                .pick_file()
            {
                ui_actions.image_to_add = Some((ch_idx, path));
                // Re-apply deck actions so the image gets loaded this frame
                if let (Some(context), Some(mixer), Some(egui_renderer)) =
                    (&self.context, &mut self.mixer, &mut self.egui_renderer)
                {
                    ui::state::apply_deck_and_effect_actions(
                        mixer,
                        context,
                        &self.registry,
                        &mut ui_actions,
                        egui_renderer,
                        &mut self.deck_preview_textures,
                    );
                }
            }
        }

        // Handle deferred video file dialog
        if let Some(ch_idx) = ui_actions.open_video_dialog_for_channel {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Video", &["mov", "mp4", "avi", "mkv", "webm"])
                .pick_file()
            {
                ui_actions.video_to_add = Some((ch_idx, path));
                if let (Some(context), Some(mixer), Some(egui_renderer)) =
                    (&self.context, &mut self.mixer, &mut self.egui_renderer)
                {
                    ui::state::apply_deck_and_effect_actions(
                        mixer,
                        context,
                        &self.registry,
                        &mut ui_actions,
                        egui_renderer,
                        &mut self.deck_preview_textures,
                    );
                }
            }
        }

        // Render output windows
        self.render_output_windows();

        // Submit GPU frame
        self.submit_frame(window, full_output.shapes, full_output.pixels_per_point, full_output.textures_delta);
    }

    /// Create pending output windows (deferred from UI actions to here where we have ActiveEventLoop)
    fn create_pending_outputs(&mut self, event_loop: &ActiveEventLoop) {
        let pending: Vec<()> = self.pending_output_creates.drain(..).collect();
        for _ in pending {
            let idx = self.output_windows.len() + 1;
            let name = format!("Output {}", idx);
            let window_attrs = Window::default_attributes()
                .with_title(format!("Varda - {}", name))
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));

            match event_loop.create_window(window_attrs) {
                Ok(window) => {
                    let window_static: &'static Window = Box::leak(Box::new(window));
                    if let Some(context) = &self.context {
                        match OutputWindow::new(context, window_static, name.clone()) {
                            Ok(output) => {
                                log::info!("Created output window '{}'", name);
                                self.output_windows.push(output);
                            }
                            Err(e) => {
                                log::error!("Failed to create output window: {}", e);
                                self.notifications.error(format!("Failed to create output: {}", e));
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to create output window: {}", e);
                    self.notifications.error(format!("Failed to create window: {}", e));
                }
            }
        }
    }

    /// Apply output-related UI actions
    fn apply_output_actions(&mut self, ui_actions: &ui::UIActions) {
        // Process in reverse for removals
        for action in &ui_actions.output_actions {
            match action {
                ui::OutputAction::Create => {
                    self.pending_output_creates.push(());
                }
                ui::OutputAction::Close { idx } => {
                    if *idx < self.output_windows.len() {
                        let name = self.output_windows[*idx].name.clone();
                        let output = self.output_windows.remove(*idx);
                        output.destroy();
                        log::info!("Closed output window '{}'", name);
                    }
                }
                ui::OutputAction::SetTarget { idx, target } => {
                    if let Some(output) = self.output_windows.get_mut(*idx) {
                        let monitor = match target {
                            OutputTarget::Display { monitor_index, .. } => {
                                self.cached_monitors.get(*monitor_index).map(|(_, h)| h.clone())
                            }
                            _ => None,
                        };
                        log::info!("Output '{}' target: {}", output.name, target);
                        output.set_target(target.clone(), monitor);
                    }
                }
                ui::OutputAction::AssignSurface { output_idx, surface_idx } => {
                    if let Some(output) = self.output_windows.get_mut(*output_idx) {
                        // Don't add duplicate assignments
                        if !output.surface_assignments.iter().any(|a| a.surface_idx == *surface_idx) {
                            if let Some(surface) = self.surface_manager.surfaces.get(*surface_idx) {
                                let bb = surface.bounding_box();
                                let assignment = SurfaceAssignment {
                                    surface_idx: *surface_idx,
                                    warp_corners: [
                                        [bb.x, bb.y],
                                        [bb.x + bb.width, bb.y],
                                        [bb.x + bb.width, bb.y + bb.height],
                                        [bb.x, bb.y + bb.height],
                                    ],
                                    enabled: true,
                                };
                                log::info!("Assigned surface '{}' to output '{}'", surface.name, output.name);
                                output.surface_assignments.push(assignment);
                            }
                        }
                    }
                }
                ui::OutputAction::UnassignSurface { output_idx, assignment_idx } => {
                    if let Some(output) = self.output_windows.get_mut(*output_idx) {
                        if *assignment_idx < output.surface_assignments.len() {
                            output.surface_assignments.remove(*assignment_idx);
                            log::info!("Removed surface assignment from output '{}'", output.name);
                        }
                    }
                }
                ui::OutputAction::ToggleCalibration { idx } => {
                    if let Some(output) = self.output_windows.get_mut(*idx) {
                        output.calibration_mode = !output.calibration_mode;
                        log::info!("Output '{}' calibration mode: {}", output.name, output.calibration_mode);
                    }
                }
                ui::OutputAction::SetWarpCorner { output_idx, assignment_idx, corner_idx, position } => {
                    if let Some(output) = self.output_windows.get_mut(*output_idx) {
                        if let Some(assignment) = output.surface_assignments.get_mut(*assignment_idx) {
                            if *corner_idx < 4 {
                                assignment.warp_corners[*corner_idx] = *position;
                            }
                        }
                    }
                }
                ui::OutputAction::ResetWarp { output_idx, assignment_idx } => {
                    if let Some(output) = self.output_windows.get_mut(*output_idx) {
                        if let Some(assignment) = output.surface_assignments.get_mut(*assignment_idx) {
                            if let Some(surface) = self.surface_manager.surfaces.get(assignment.surface_idx) {
                                let bb = surface.bounding_box();
                                assignment.warp_corners = [
                                    [bb.x, bb.y],
                                    [bb.x + bb.width, bb.y],
                                    [bb.x + bb.width, bb.y + bb.height],
                                    [bb.x, bb.y + bb.height],
                                ];
                                log::info!("Reset warp for surface in output '{}'", output.name);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Apply surface actions from UI
    fn apply_surface_actions(&mut self, ui_actions: &ui::UIActions) {
        for action in &ui_actions.surface_actions {
            match action {
                ui::SurfaceAction::Add { name, source } => {
                    let idx = self.surface_manager.add_surface(name.clone(), source.clone());
                    log::info!("Added surface '{}' (index {})", name, idx);
                }
                ui::SurfaceAction::AddPolygon { name, vertices, source } => {
                    let idx = self.surface_manager.add_polygon_surface(name.clone(), vertices.clone(), source.clone());
                    log::info!("Added polygon surface '{}' with {} vertices (index {})", name, vertices.len(), idx);
                }
                ui::SurfaceAction::Remove { idx } => {
                    if *idx < self.surface_manager.surfaces.len() {
                        let name = self.surface_manager.surfaces[*idx].name.clone();
                        self.surface_manager.remove_surface(*idx);
                        log::info!("Removed surface '{}'", name);
                    }
                }
                ui::SurfaceAction::UpdateVertices { idx, contour, vertices } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        if *contour == 0 {
                            // If this is a circle, update the hint center to match the moved vertices
                            if let Some(ref mut hint) = surface.circle_hint {
                                let n = vertices.len().max(1) as f32;
                                let sum = vertices.iter().fold([0.0f32, 0.0], |acc, v| {
                                    [acc[0] + v[0], acc[1] + v[1]]
                                });
                                hint.center = [sum[0] / n, sum[1] / n];
                            }
                            surface.vertices = vertices.clone();
                        } else if let Some(c) = surface.extra_contours.get_mut(*contour - 1) {
                            *c = vertices.clone();
                        }
                    }
                }
                ui::SurfaceAction::MoveDelta { idx, dx, dy } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        surface.translate(*dx, *dy);
                        // Recalculate circle hint center from vertices
                        let n = surface.vertices.len().max(1) as f32;
                        let sum = surface.vertices.iter().fold([0.0f32, 0.0], |acc, v| {
                            [acc[0] + v[0], acc[1] + v[1]]
                        });
                        let new_center = [sum[0] / n, sum[1] / n];
                        if let Some(ref mut hint) = surface.circle_hint {
                            hint.center = new_center;
                        }
                    }
                }
                ui::SurfaceAction::SetSource { idx, source } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        surface.source = source.clone();
                        log::info!("Surface '{}' source changed to: {}", surface.name, source);
                    }
                }
                ui::SurfaceAction::SetOutputType { idx, output_type } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        surface.output_type = *output_type;
                        log::info!("Surface '{}' output type changed to: {}", surface.name, output_type);
                    }
                }
                ui::SurfaceAction::SetContentMapping { idx, mapping } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        surface.content_mapping = *mapping;
                        log::info!("Surface '{}' content mapping changed to: {}", surface.name, mapping);
                    }
                }
                ui::SurfaceAction::Rename { idx, name } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        log::info!("Surface '{}' renamed to '{}'", surface.name, name);
                        surface.name = name.clone();
                    }
                }
                ui::SurfaceAction::Duplicate { idx } => {
                    if let Some(surface) = self.surface_manager.surfaces.get(*idx).cloned() {
                        let mut dup = surface;
                        dup.name = format!("{} (copy)", dup.name);
                        let offset = self.stage_editor_grid_size;
                        for v in &mut dup.vertices {
                            v[0] = (v[0] + offset).min(1.0);
                            v[1] = (v[1] + offset).min(1.0);
                        }
                        for contour in &mut dup.extra_contours {
                            for v in contour.iter_mut() {
                                v[0] = (v[0] + offset).min(1.0);
                                v[1] = (v[1] + offset).min(1.0);
                            }
                        }
                        if let Some(ref mut hint) = dup.circle_hint {
                            hint.center[0] = (hint.center[0] + offset).min(1.0);
                            hint.center[1] = (hint.center[1] + offset).min(1.0);
                        }
                        let name = dup.name.clone();
                        self.surface_manager.surfaces.push(dup);
                        log::info!("Duplicated surface '{}' → '{}'", self.surface_manager.surfaces[*idx].name, name);
                    }
                }
                ui::SurfaceAction::FlipHorizontal { idx } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        let bb = surface.bounding_box();
                        let cx = bb.x + bb.width / 2.0;
                        for v in &mut surface.vertices {
                            v[0] = cx + (cx - v[0]);
                        }
                        for contour in &mut surface.extra_contours {
                            for v in contour.iter_mut() {
                                v[0] = cx + (cx - v[0]);
                            }
                        }
                        if let Some(ref mut hint) = surface.circle_hint {
                            hint.center[0] = cx + (cx - hint.center[0]);
                        }
                        log::info!("Flipped surface '{}' horizontally", surface.name);
                    }
                }
                ui::SurfaceAction::FlipVertical { idx } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        let bb = surface.bounding_box();
                        let cy = bb.y + bb.height / 2.0;
                        for v in &mut surface.vertices {
                            v[1] = cy + (cy - v[1]);
                        }
                        for contour in &mut surface.extra_contours {
                            for v in contour.iter_mut() {
                                v[1] = cy + (cy - v[1]);
                            }
                        }
                        if let Some(ref mut hint) = surface.circle_hint {
                            hint.center[1] = cy + (cy - hint.center[1]);
                        }
                        log::info!("Flipped surface '{}' vertically", surface.name);
                    }
                }
                ui::SurfaceAction::InsertVertex { idx, after_vert_idx, position } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        // Inserting a vertex breaks circle identity
                        surface.convert_to_polygon();
                        if *after_vert_idx < surface.vertices.len() {
                            surface.vertices.insert(after_vert_idx + 1, *position);
                            log::info!("Inserted vertex on surface '{}' after vertex {}", surface.name, after_vert_idx);
                        }
                    }
                }
                ui::SurfaceAction::AddCircle { name, hint, source } => {
                    let idx = self.surface_manager.add_circle_surface(name.clone(), *hint, source.clone());
                    log::info!("Added circle surface '{}' (index {}, radius={:.3}, sides={})", name, idx, hint.radius, hint.sides);
                }
                ui::SurfaceAction::SetCircleRadius { idx, radius } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        if let Some(ref mut hint) = surface.circle_hint {
                            hint.radius = *radius;
                            surface.vertices = hint.generate_vertices();
                            log::info!("Circle '{}' radius set to {:.3}", surface.name, radius);
                        }
                    }
                }
                ui::SurfaceAction::SetCircleSides { idx, sides } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        if let Some(ref mut hint) = surface.circle_hint {
                            hint.sides = *sides;
                            surface.vertices = hint.generate_vertices();
                            log::info!("Circle '{}' sides set to {}", surface.name, sides);
                        }
                    }
                }
                ui::SurfaceAction::ConvertToPolygon { idx } => {
                    if let Some(surface) = self.surface_manager.surfaces.get_mut(*idx) {
                        surface.convert_to_polygon();
                        log::info!("Converted surface '{}' to polygon", surface.name);
                    }
                }
                ui::SurfaceAction::Combine { indices } => {
                    if let Some(new_idx) = self.surface_manager.combine_surfaces(indices) {
                        let name = self.surface_manager.surfaces[new_idx].name.clone();
                        let contour_count = 1 + self.surface_manager.surfaces[new_idx].extra_contours.len();
                        log::info!("Combined {} surfaces into '{}' ({} contours)", indices.len(), name, contour_count);
                        self.notifications.info(format!("🔗 Combined {} surfaces → '{}'", indices.len(), name));
                    }
                }
            }
        }
    }

    /// Resolve an OutputSource to a texture view from the mixer
    fn resolve_source<'a>(mixer: &'a Mixer, source: &OutputSource) -> Option<&'a wgpu::TextureView> {
        match source {
            OutputSource::Master => Some(&mixer.composite_view),
            OutputSource::Channel(ch_idx) => {
                mixer.channels.get(*ch_idx).map(|ch| &ch.composite_view)
            }
            OutputSource::Deck(ch_idx, deck_idx) => {
                mixer.channels.get(*ch_idx)
                    .and_then(|ch| ch.decks.get(*deck_idx))
                    .map(|slot| &slot.deck.texture_view)
            }
        }
    }

    /// Render content to all output windows using the surface layout.
    /// If the output has surface assignments, only those surfaces are rendered (with per-surface warp).
    /// If no assignments, all surfaces are rendered (no warp). If no surfaces at all, falls back to direct source blit.
    fn render_output_windows(&self) {
        let Some(context) = &self.context else { return };
        let Some(mixer) = &self.mixer else { return };

        for output in &self.output_windows {
            if output.calibration_mode && !self.calibration_textures.is_empty() && self.surface_manager.surfaces.is_empty() {
                // Calibration mode with no surfaces — show a single fullscreen test card
                output.render(context, &self.calibration_textures[0].1);
            } else if self.surface_manager.surfaces.is_empty() {
                // No surfaces — show master mix as fullscreen quad
                output.render(context, &mixer.composite_view);
            } else if !output.surface_assignments.is_empty() {
                // Render only assigned surfaces, with per-surface warp
                let render_infos: Vec<SurfaceRenderInfo<'_>> = output.surface_assignments.iter()
                    .enumerate()
                    .filter(|(_, a)| a.enabled)
                    .filter_map(|(ai, assignment)| {
                        let surface = self.surface_manager.surfaces.get(assignment.surface_idx)?;
                        let bb = surface.bounding_box();

                        // In calibration mode, use the test card instead of content
                        let content_view = if output.calibration_mode && !self.calibration_textures.is_empty() {
                            let color_idx = ai % self.calibration_textures.len();
                            &self.calibration_textures[color_idx].1
                        } else {
                            Self::resolve_source(mixer, &surface.source)?
                        };

                        // In calibration mode, always use Fill so the full test card is visible
                        let (uv_scale, uv_offset) = if output.calibration_mode {
                            ([1.0, 1.0], [0.0, 0.0])
                        } else {
                            match surface.content_mapping {
                                ContentMapping::Fill => ([1.0, 1.0], [0.0, 0.0]),
                                ContentMapping::Mapped => (
                                    [bb.width, bb.height],
                                    [bb.x, bb.y],
                                ),
                            }
                        };

                        Some(SurfaceRenderInfo {
                            content_view,
                            vertices: &surface.vertices,
                            bounding_box: [bb.x, bb.y, bb.width, bb.height],
                            uv_scale,
                            uv_offset,
                            warp_corners: Some(assignment.warp_corners),
                        })
                    })
                    .collect();

                output.render_surfaces(context, &render_infos);
            } else {
                // No assignments — render all surfaces without warp (fallback)
                let render_infos: Vec<SurfaceRenderInfo<'_>> = self.surface_manager.surfaces.iter()
                    .enumerate()
                    .filter_map(|(si, surface)| {
                        let bb = surface.bounding_box();

                        // In calibration mode, use test cards
                        let content_view = if output.calibration_mode && !self.calibration_textures.is_empty() {
                            let color_idx = si % self.calibration_textures.len();
                            &self.calibration_textures[color_idx].1
                        } else {
                            Self::resolve_source(mixer, &surface.source)?
                        };

                        let (uv_scale, uv_offset) = if output.calibration_mode {
                            ([1.0, 1.0], [0.0, 0.0])
                        } else {
                            match surface.content_mapping {
                                ContentMapping::Fill => ([1.0, 1.0], [0.0, 0.0]),
                                ContentMapping::Mapped => (
                                    [bb.width, bb.height],
                                    [bb.x, bb.y],
                                ),
                            }
                        };

                        Some(SurfaceRenderInfo {
                            content_view,
                            vertices: &surface.vertices,
                            bounding_box: [bb.x, bb.y, bb.width, bb.height],
                            uv_scale,
                            uv_offset,
                            warp_corners: None,
                        })
                    })
                    .collect();

                output.render_surfaces(context, &render_infos);
            }

            output.window.request_redraw();
        }
    }

    /// Apply UI actions that mutate engine state (mixer, decks, effects, transitions).
    fn apply_engine_actions(&mut self, ui_actions: &mut ui::UIActions) {
        let Some(context) = &self.context else { return };
        if let Some(mixer) = &mut self.mixer {
            ui::state::apply_crossfader_actions(mixer, ui_actions);
            ui::state::apply_channel_updates(mixer, ui_actions);
            ui::state::apply_deck_updates(mixer, ui_actions);
            ui::state::apply_scaling_mode_updates(mixer, ui_actions);
            ui::state::apply_param_updates(mixer, ui_actions);
            ui::state::apply_modulation_actions(mixer, ui_actions);
        }
        // Release camera references before deck removal
        if let Some((ch_idx, deck_idx)) = ui_actions.deck_to_remove {
            if let Some(mixer) = &self.mixer {
                if let Some(ch) = mixer.channels.get(ch_idx) {
                    if let Some(slot) = ch.decks.get(deck_idx) {
                        if let Some(cam_id) = slot.deck.camera_id() {
                            self.camera_manager.release_camera(cam_id);
                        }
                    }
                }
            }
        }
        if let (Some(mixer), Some(egui_renderer)) = (&mut self.mixer, &mut self.egui_renderer) {
            ui::state::apply_deck_and_effect_actions(
                mixer,
                context,
                &self.registry,
                ui_actions,
                egui_renderer,
                &mut self.deck_preview_textures,
            );
        }
        if let Some(mixer) = &mut self.mixer {
            ui::state::apply_transition_actions(mixer, context, &self.registry, ui_actions);
        }
        // Add new channel if requested
        if ui_actions.add_channel {
            if let Some(mixer) = &mut self.mixer {
                match mixer.add_channel(context, RENDER_WIDTH, RENDER_HEIGHT) {
                    Ok(idx) => {
                        self.notifications.info(format!("Added channel {} (index {})",
                            mixer.channels[idx].name, idx));
                    }
                    Err(e) => {
                        log::error!("Failed to add channel: {}", e);
                        self.notifications.error(format!("Error adding channel: {}", e));
                    }
                }
            }
        }
        // Add camera deck if requested
        if let Some((ch_idx, camera_id)) = ui_actions.camera_to_add.take() {
            if let (Some(context), Some(mixer), Some(egui_renderer)) =
                (&self.context, &mut self.mixer, &mut self.egui_renderer)
            {
                let cam_name = self.camera_manager.devices().iter()
                    .find(|d| d.id == camera_id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| format!("Camera {}", camera_id));

                match self.camera_manager.open_camera(camera_id, &context.device) {
                    Ok((src_w, src_h)) => {
                        match Deck::new_from_camera(context, camera_id, &cam_name, src_w, src_h, RENDER_WIDTH, RENDER_HEIGHT) {
                            Ok(deck) => {
                                if let Some(ch) = mixer.channel_mut(ch_idx) {
                                    let idx = ch.add_deck(deck);
                                    log::info!("Added camera deck {} to channel {}: {}", idx, ch_idx, cam_name);

                                    let texture_id = egui_renderer.register_native_texture(
                                        &context.device,
                                        &ch.decks[idx].deck.texture_view,
                                        wgpu::FilterMode::Linear,
                                    );
                                    self.deck_preview_textures.insert((ch_idx, idx), texture_id);
                                    self.notifications.info(format!("📹 Camera '{}' added to Ch {}", cam_name, ch_idx + 1));
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to create camera deck: {}", e);
                                self.notifications.error(format!("Failed to create camera deck: {}", e));
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to open camera '{}': {}", cam_name, e);
                        self.notifications.error(format!("Failed to open camera '{}': {}", cam_name, e));
                    }
                }
            }
        }

        // Remove channel if requested
        if let Some(ch_idx) = ui_actions.remove_channel {
            if let Some(mixer) = &mut self.mixer {
                let name = mixer.channels.get(ch_idx).map(|c| c.name.clone()).unwrap_or_default();
                if mixer.remove_channel(ch_idx) {
                    self.notifications.info(format!("Removed channel {}", name));
                    // Fix selected_deck/selected_channel if they pointed at or beyond the removed channel
                    if let Some((sel_ch, _)) = self.selected_deck {
                        if sel_ch == ch_idx {
                            self.selected_deck = None;
                        } else if sel_ch > ch_idx {
                            self.selected_deck = Some((sel_ch - 1, self.selected_deck.unwrap().1));
                        }
                    }
                    if let Some(sel_ch) = self.selected_channel {
                        if sel_ch == ch_idx {
                            self.selected_channel = None;
                        } else if sel_ch > ch_idx {
                            self.selected_channel = Some(sel_ch - 1);
                        }
                    }
                } else {
                    self.notifications.error("Cannot remove channel (minimum 2 required)".to_string());
                }
            }
        }
    }

    /// Render the mixer, blit to screen, overlay egui, and present.
    fn submit_frame(
        &mut self,
        window: &Window,
        shapes: Vec<egui::epaint::ClippedShape>,
        pixels_per_point: f32,
        textures_delta: egui::TexturesDelta,
    ) {
        let Some(context) = &self.context else { return };

        // Acquire surface texture
        let output = match context.surface.get_current_texture() {
            Ok(output) => output,
            Err(e) => {
                log::error!("Failed to get surface texture: {}", e);
                return;
            }
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Tessellate egui shapes
        let paint_jobs = self.egui_ctx.tessellate(shapes, pixels_per_point);

        let Some(egui_renderer) = &mut self.egui_renderer else { return };

        // Update egui textures
        for (id, image_delta) in &textures_delta.set {
            egui_renderer.update_texture(&context.device, &context.queue, *id, image_delta);
        }

        // Create encoder
        let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        // Update camera frames and distribute shared texture views to camera decks
        self.camera_manager.update(&context.queue);
        if let Some(mixer) = &mut self.mixer {
            for channel in &mut mixer.channels {
                for slot in &mut channel.decks {
                    if let Some(cam_id) = slot.deck.camera_id() {
                        slot.deck.camera_source_view = self.camera_manager
                            .texture_view(cam_id)
                            .cloned();
                    }
                }
            }
        }

        // Render the mixer (all channels composited)
        if let Some(mixer) = &mut self.mixer {
            if let Err(e) = mixer.render(context, &self.audio_data) {
                log::error!("Failed to render mixer: {}", e);
            }
        }

        // Create bind group to blit mixer composite to screen
        let bind_group = if let (Some(mixer), Some(blit_pipeline)) = (&self.mixer, &self.blit_pipeline) {
            Some(blit_pipeline.create_bind_group(&context.device, &mixer.composite_view))
        } else {
            None
        };

        // Screen descriptor for egui
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [context.size.width, context.size.height],
            pixels_per_point: window.scale_factor() as f32,
        };

        // Update egui buffers
        egui_renderer.update_buffers(
            &context.device,
            &context.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        // Render pass — blit mixer output + egui overlay
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if let (Some(bind_group), Some(blit_pipeline)) = (&bind_group, &self.blit_pipeline) {
                blit_pipeline.render(&mut render_pass, bind_group);
            }

            let mut render_pass_static = render_pass.forget_lifetime();
            egui_renderer.render(&mut render_pass_static, &paint_jobs, &screen_descriptor);
        }

        // Free egui textures
        for id in &textures_delta.free {
            egui_renderer.free_texture(id);
        }

        // Submit and present
        context.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("🎨 Varda VJ Software - Starting up...");

    let event_loop = EventLoop::new()?;
    let mut app = App::new();

    event_loop.run_app(&mut app)
        .map_err(|e| anyhow::anyhow!("Event loop error: {:?}", e))?;

    Ok(())
}
