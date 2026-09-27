//! Creating outputs, and the window host that gives window sinks their OS
//! windows. See /spec/output-sink-providers.md.

use super::VardaApp;
use crate::output::Output;
use crate::renderer::context::SurfaceAssignment;
use crate::renderer::edge_blend::SurfaceOverlapZones;

/// Logical box a new output window is fitted into when the stage has no saved
/// size for it.
const DEFAULT_OUTPUT_WINDOW_BOX: (f64, f64) = (1280.0, 720.0);

/// Opening size for a new output window, as the master's aspect ratio fitted
/// inside [`DEFAULT_OUTPUT_WINDOW_BOX`].
///
/// The master is letterboxed into whatever size the window ends up, so this only
/// picks the starting shape — but starting at the master's aspect means a
/// vertical or square stage doesn't open in a 16:9 window with bars down both
/// sides that the operator has to drag out by hand.
fn default_output_window_size(render_width: u32, render_height: u32) -> (f64, f64) {
    let (box_w, box_h) = DEFAULT_OUTPUT_WINDOW_BOX;
    if render_width == 0 || render_height == 0 {
        return (box_w, box_h);
    }
    let aspect = f64::from(render_width) / f64::from(render_height);
    if aspect >= box_w / box_h {
        (box_w, box_w / aspect)
    } else {
        (box_h * aspect, box_h)
    }
}

impl VardaApp {
    /// Build an output from its saved or requested config and add it. A sink
    /// this run cannot build is kept as a placeholder holding its config, so
    /// the output survives a save; the reason is shown. Returns the UUID.
    ///
    /// # Errors
    ///
    /// Fails only when no GPU resources can be built for the output at all.
    pub(crate) fn create_output(
        &mut self,
        config: &crate::scene::OutputConfig,
    ) -> anyhow::Result<String> {
        let mut config = config.clone();
        config.migrate_legacy();
        let render = (self.render.width, self.render.height);
        let (sink, reason) = {
            let mut env = crate::output::SinkEnv {
                gpu: &self.render.context,
                services: &mut self.sources.services,
                width: render.0,
                height: render.1,
            };
            self.output.sinks.restore(&config.target, &mut env)
        };
        if config.name.is_empty() {
            config.name = format!("Output {}", self.output.outputs.len() + 1);
        }
        if let Some(reason) = reason {
            let message = format!(
                "Output '{}' is unavailable here and kept as is: {reason}",
                config.name
            );
            log::warn!("{message}");
            self.session.notifications.warn(message);
        }
        let output = Output::new(
            &self.render.context,
            &self.sources.services,
            config.uuid.clone(),
            config.name.clone(),
            sink,
            render,
        )?;
        log::info!(
            "Created output '{}' ({})",
            config.name,
            output.sink().sink_type()
        );
        self.output.outputs.push(output);
        let idx = self.output.outputs.len() - 1;
        self.apply_output_settings(idx, &config)?;
        Ok(config.uuid)
    }

    /// Make the live outputs match a saved stage. Outputs the stage does not
    /// have are stopped and closed. An output it has is updated in place: one
    /// whose sink settings match keeps its sink, so a recording or stream in
    /// progress keeps running through a reload; one whose settings differ is
    /// stopped and its sink rebuilt. New outputs are created, and the result
    /// takes the saved order. Returns a message per output that failed.
    pub(crate) fn reconcile_outputs(
        &mut self,
        saved: &[crate::scene::OutputConfig],
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let gone: Vec<String> = self
            .output
            .outputs
            .iter()
            .filter(|live| !saved.iter().any(|config| config.uuid == live.uuid))
            .map(|live| live.uuid.clone())
            .collect();
        for uuid in gone {
            let passthrough = self.output.close_output(&uuid).ok().flatten();
            self.release_passthrough(passthrough);
        }

        for config in saved {
            let mut config = config.clone();
            config.migrate_legacy();
            let live = self
                .output
                .outputs
                .iter()
                .position(|live| !config.uuid.is_empty() && live.uuid == config.uuid);
            let result = match live {
                Some(idx) => {
                    let current = self.output.outputs[idx].sink().config();
                    let rebuilt = if same_sink(&current, &config.target) {
                        Ok(())
                    } else {
                        self.apply_sink_config(idx, &config.target)
                    };
                    rebuilt.and_then(|()| self.apply_output_settings(idx, &config))
                }
                None => self.create_output(&config).map(|_| ()),
            };
            if let Err(e) = result {
                log::error!("Failed to restore output '{}': {e:#}", config.name);
                errors.push(format!("output '{}': {e:#}", config.name));
            }
        }

        let position = |uuid: &str| {
            saved
                .iter()
                .position(|config| config.uuid == uuid)
                .unwrap_or(usize::MAX)
        };
        self.output.outputs.sort_by_key(|o| position(&o.uuid));
        errors
    }

    /// Apply everything a saved output holds besides its sink: its name,
    /// surfaces, blending, calibration, rotation and format.
    fn apply_output_settings(
        &mut self,
        idx: usize,
        config: &crate::scene::OutputConfig,
    ) -> anyhow::Result<()> {
        // One-time migration: pre-8i.5 files stored warp on the assignment.
        // Move it onto the surface (first assignment wins; an existing surface
        // warp, such as a dome mesh, takes precedence).
        for a in &config.surface_assignments {
            if let Some(warp) = &a.legacy_warp_mode
                && let Some((_, surface)) = self
                    .output
                    .surface_manager
                    .find_by_uuid_mut(&a.surface_uuid)
                && surface.warp.is_none()
            {
                surface.warp = Some(warp.clone());
            }
        }
        let output = &mut self.output.outputs[idx];
        if !config.name.is_empty() {
            output.name.clone_from(&config.name);
        }
        output.surface_assignments = config
            .surface_assignments
            .iter()
            .map(|a| SurfaceAssignment {
                surface_uuid: a.surface_uuid.clone(),
                enabled: a.enabled,
                overlap_zones: SurfaceOverlapZones::default(),
            })
            .collect();
        output.edge_blend_mode = config.edge_blend_mode;
        output.edge_blend = config.edge_blend;
        output.calibration_mode = config.calibration_mode;
        output.unassigned = config.unassigned;
        output.tonemap_override = config.tonemap_override;
        output.set_rotation(&self.render.context, config.rotation)?;
        if let Err(error) = output.set_presentation_request(
            &self.render.context,
            &self.sources.services,
            config.presentation,
        ) {
            log::warn!(
                "Output '{}' presentation request fell back during restore: {error}",
                output.name
            );
        }
        self.refresh_presentation_notification(idx);
        Ok(())
    }

    /// The window host: create the OS windows window sinks are waiting for,
    /// and hand each its window. Windows need the event loop, so the runner
    /// calls this with it once per loop.
    pub fn create_pending_outputs(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let render = (self.render.width, self.render.height);
        for idx in 0..self.output.outputs.len() {
            let output = &mut self.output.outputs[idx];
            let Some(mut attrs) = output.sink().window_request() else {
                continue;
            };
            attrs = attrs.with_title(format!("Varda - {}", output.name));
            if attrs.inner_size.is_none() {
                let (w, h) = default_output_window_size(render.0, render.1);
                attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(w, h));
            }
            let window = match event_loop.create_window(attrs) {
                Ok(window) => window,
                Err(e) => {
                    log::error!("Failed to create output window: {e}");
                    self.session
                        .notifications
                        .error(format!("Failed to create window: {e}"));
                    continue;
                }
            };
            let gpu = &self.render.context;
            let services = &self.sources.services;
            let attached = output
                .sink_mut()
                .attach_window(gpu, services, window)
                .and_then(|()| output.resize(gpu, render))
                .and_then(|()| {
                    output.set_presentation_request(gpu, services, output.presentation_request())
                });
            if let Err(e) = attached {
                log::error!("Failed to open output window '{}': {e:#}", output.name);
                self.session
                    .notifications
                    .error(format!("Failed to create output: {e:#}"));
                continue;
            }
            if let Some(notice) = output.sink_mut().take_notice() {
                log::warn!("{notice}");
                self.session.notifications.warn(notice);
            }
            log::info!("Opened output window '{}'", output.name);
            self.refresh_presentation_notification(idx);
        }
    }

    /// Refresh the monitors the display type offers, from the event loop.
    pub fn refresh_monitors(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(monitors) = self
            .sources
            .services
            .get_mut::<crate::output::window::Monitors>()
        {
            monitors.list = event_loop
                .available_monitors()
                .map(|m| (m.name().unwrap_or_else(|| "Unknown".to_string()), m))
                .collect();
        }
    }

    /// Close the output shown in the OS window `window_id`. Returns its name.
    pub fn close_output_window_by_id(
        &mut self,
        window_id: winit::window::WindowId,
    ) -> Option<String> {
        let idx = self
            .output
            .outputs
            .iter()
            .position(|o| o.sink().window_id() == Some(window_id))?;
        let uuid = self.output.outputs[idx].uuid.clone();
        let name = self.output.outputs[idx].name.clone();
        let passthrough = self.output.close_output(&uuid).ok().flatten();
        self.release_passthrough(passthrough);
        Some(name)
    }

    /// Follow a resize of the OS window `window_id`.
    pub fn resize_output_window_by_id(
        &mut self,
        window_id: winit::window::WindowId,
        new_size: winit::dpi::PhysicalSize<u32>,
    ) {
        let render = (self.render.width, self.render.height);
        let gpu = &self.render.context;
        if let Some(output) = self
            .output
            .outputs
            .iter_mut()
            .find(|o| o.sink().window_id() == Some(window_id))
        {
            output
                .sink_mut()
                .window_resized(gpu, new_size.width, new_size.height);
            if let Err(e) = output.resize(gpu, render) {
                log::error!(
                    "Output '{}' could not follow its window: {e:#}",
                    output.name
                );
            }
        }
    }
}

/// Whether a live sink already has every setting a saved one names. The
/// saved config may leave fields to the type's defaults.
fn same_sink(
    live: &crate::engine::value::provider::ProviderConfig,
    saved: &crate::engine::value::provider::ProviderConfig,
) -> bool {
    live.type_id() == saved.type_id()
        && saved
            .fields()
            .iter()
            .all(|(key, value)| live.get(key) == Some(value))
}

#[cfg(test)]
mod tests {
    use super::default_output_window_size;

    /// Aspect the window opens at, to compare against the master's.
    fn aspect((w, h): (f64, f64)) -> f64 {
        w / h
    }

    #[test]
    fn sixteen_by_nine_master_fills_the_default_box() {
        assert_eq!(default_output_window_size(1920, 1080), (1280.0, 720.0));
        assert_eq!(default_output_window_size(3840, 2160), (1280.0, 720.0));
    }

    #[test]
    fn vertical_master_opens_vertical() {
        let size = default_output_window_size(1080, 1920);
        assert!(
            (aspect(size) - 1080.0 / 1920.0).abs() < 1e-9,
            "expected the master's aspect, got {size:?}"
        );
        // Height is the binding dimension for anything taller than the box.
        assert!((size.1 - 720.0).abs() < 1e-9, "got {size:?}");
        assert!(
            size.0 < size.1,
            "a vertical stage must not open wider than tall"
        );
    }

    #[test]
    fn square_master_opens_square() {
        assert_eq!(default_output_window_size(1080, 1080), (720.0, 720.0));
    }

    #[test]
    fn ultrawide_master_is_bounded_by_width() {
        let size = default_output_window_size(5120, 1440);
        assert!((size.0 - 1280.0).abs() < 1e-9, "got {size:?}");
        assert!(
            (aspect(size) - 5120.0 / 1440.0).abs() < 1e-9,
            "got {size:?}"
        );
    }

    #[test]
    fn zero_resolution_falls_back_to_the_box() {
        // set_render_resolution rejects zero, so this only guards against a
        // divide by zero if that ever changes.
        assert_eq!(default_output_window_size(0, 0), (1280.0, 720.0));
        assert_eq!(default_output_window_size(1920, 0), (1280.0, 720.0));
    }
}
