//! Deck rendering: the source, then the effect chain, then analyzer capture.

use super::{Deck, Effect, PreprocessorSlot};
use crate::analyzer::traits::{AnalyzerStateSnapshot, TextureData};
use crate::analyzer::{AnalyzerRegistry, DeckAnalyzers, PreprocessorCategory};
use crate::audio::AudioData;
use crate::isf::PhaseInput;
use crate::modulation::ModulationEngine;
use crate::params::{ParamValue, ShaderParams};
use crate::renderer::GpuContext;
use crate::source::{SourceControl, SourceFrame};
use anyhow::Result;
use std::collections::HashMap;
use std::time::Instant;

/// Upload analyzer texture data to a preprocessor slot's GPU texture.
///
/// If dimensions changed, recreates the texture and view. Otherwise writes data in place.
fn upload_texture_to_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &mut PreprocessorSlot,
    tex_data: &TextureData,
) {
    if tex_data.width == 0 || tex_data.height == 0 || tex_data.data.is_empty() {
        return;
    }
    if tex_data.generation != 0 && slot.last_uploaded_generation == Some(tex_data.generation) {
        return;
    }
    // The slot's format is fixed by the shader's declaration and the pipeline layout. Recreating
    // the texture in another encoding would misread the bytes, so a mismatch is refused.
    if super::preprocessor_texture_format(&tex_data.format) != slot.format {
        log::warn!(
            "Preprocessor '{}' published '{}' but the shader declared {:?}; dropping upload",
            slot.name,
            tex_data.format,
            slot.format
        );
        return;
    }
    let bytes_per_texel = match slot.format {
        wgpu::TextureFormat::Rgba32Float => 16,
        _ => 4,
    };

    let current_size = slot.texture.size();
    if current_size.width != tex_data.width || current_size.height != tex_data.height {
        // Dimensions changed — recreate texture
        let new_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("Preprocessor: {}", slot.name)),
            size: wgpu::Extent3d {
                width: tex_data.width,
                height: tex_data.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Data texture (packed analyzer output), not part of the color path. Format is the
            // encoding the shader declared.
            format: slot.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        slot.view = new_texture.create_view(&wgpu::TextureViewDescriptor::default());
        slot.texture = new_texture;
    }

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &slot.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &tex_data.data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_texel * tex_data.width),
            rows_per_image: Some(tex_data.height),
        },
        wgpu::Extent3d {
            width: tex_data.width,
            height: tex_data.height,
            depth_or_array_layers: 1,
        },
    );
    slot.last_uploaded_generation = (tex_data.generation != 0).then_some(tex_data.generation);
}

/// Accumulate phase times: for each `PhaseInput`, adds
/// `dt * param_value * multiply_by * scale` to the accumulator.
///
/// Uses modulated parameter values, not stored bases: a shader declaring `PHASE_INPUTS` reads
/// `PHASE_TIME_N` instead of the raw speed uniform, so integrating the base would hide
/// modulation.
fn accumulate_phase_times(
    accumulators: &mut [f32; 4],
    dt: f32,
    phase_inputs: Option<&[PhaseInput]>,
    params: &mut ShaderParams,
    modulation: &ModulationEngine,
    param_prefix: &str,
) {
    let Some(inputs) = phase_inputs else {
        return;
    };
    for pi in inputs {
        if pi.index < 4 {
            let mut rate = params
                .get_float_modulated(&pi.param, modulation, Some(param_prefix))
                .unwrap_or(1.0);
            for factor in &pi.multiply_by {
                rate *= params
                    .get_float_modulated(factor, modulation, Some(param_prefix))
                    .unwrap_or(1.0);
            }
            accumulators[pi.index] += dt * rate * pi.scale;
        }
    }
}

fn collect_preprocessor_state(
    slots: &[PreprocessorSlot],
    params: &mut ShaderParams,
    phases: [f32; 4],
    modulation: &ModulationEngine,
    param_prefix: &str,
    states: &mut HashMap<String, AnalyzerStateSnapshot>,
) {
    for slot in slots {
        if slot.param_bindings.is_empty() && slot.phase_bindings.is_empty() {
            continue;
        }
        let state = states.entry(slot.analyzer_type.clone()).or_default();
        state.values.reserve(
            slot.param_bindings
                .len()
                .saturating_add(slot.phase_bindings.len()),
        );
        for (local_name, param_name) in &slot.param_bindings {
            if let Some(value) = params.get_modulated(param_name, modulation, Some(param_prefix)) {
                state.values.insert(local_name.clone(), value);
            }
        }
        for (local_name, index) in &slot.phase_bindings {
            if let Some(value) = phases.get(*index) {
                state
                    .values
                    .insert(local_name.clone(), ParamValue::Float(*value));
            }
        }
    }
}

impl Deck {
    /// Apply modulation, the transport and residency to the source's own
    /// controls. Runs every frame for every deck, visible or not. `scratch` is
    /// shared across the frame's decks so resolving keys allocates nothing.
    pub fn control_source(
        &mut self,
        modulation: &ModulationEngine,
        awake: bool,
        clock: crate::source::SourceClock,
        target_fps: u32,
        scratch: &mut String,
    ) {
        let mut ctx = SourceControl::new(&self.uuid, modulation, awake, clock, target_fps, scratch);
        self.source.control(&mut ctx);
    }

    /// Record the source's CPU-to-GPU uploads for this frame.
    pub fn upload_source(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.source.upload(encoder);
    }

    /// Called after the frame's uploads were submitted.
    pub fn after_source_submit(&mut self) {
        self.source.after_submit();
    }

    /// Render the deck to its texture (source + effect chain)
    ///
    /// # Errors
    ///
    /// Propagates any error from [`Deck::render_with_prefix`].
    pub fn render(
        &mut self,
        context: &GpuContext,
        audio_data: &AudioData,
        modulation: &ModulationEngine,
        deck_idx: usize,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        let prefix = format!("deck{deck_idx}");
        self.render_with_prefix(context, audio_data, modulation, &prefix, cmd_buffers, None)
    }

    /// Start the analyzers this deck's shader declares, using the built-in registry.
    ///
    /// For embedders without their own registry, such as `examples/shader_preview`. Without it, a
    /// shader with a `PREPROCESSORS` block renders with unbound preprocessor textures and no
    /// error. Idempotent, and safe when the shader declares nothing.
    pub fn start_declared_preprocessors(&mut self) {
        let registry = analyzer_registry();
        self.ensure_preprocessor_analyzers(&registry);
    }

    /// Start analyzers for every preprocessor slot that needs one. Called at deck creation and
    /// when effects change.
    pub(crate) fn ensure_preprocessor_analyzers(&mut self, registry: &AnalyzerRegistry) {
        // Collect all (analyzer_type, options) needed by preprocessor slots
        let mut needed: Vec<(String, serde_json::Value)> = Vec::new();
        for slot in self.source.preprocessor_slots() {
            needed.push((slot.analyzer_type.clone(), slot.options.clone()));
        }
        for effect in &self.effects {
            for slot in &effect.preprocessor_textures {
                needed.push((slot.analyzer_type.clone(), slot.options.clone()));
            }
        }

        // Host-inline preprocessors live in the deck's own set, one per type.
        let host_inline: Vec<&str> = needed
            .iter()
            .map(|(ty, _)| ty.as_str())
            .filter(|ty| registry.category_for(ty) == Some(PreprocessorCategory::HostInline))
            .collect();
        self.host_inline.retain(&host_inline);
        for (analyzer_type, options) in &needed {
            if host_inline.contains(&analyzer_type.as_str())
                && !self.host_inline.ensure(analyzer_type, registry, options)
            {
                log::warn!(
                    "Deck '{}': failed to start preprocessor '{}'",
                    self.uuid,
                    analyzer_type
                );
            }
        }

        // Deduplicate by analyzer_type and request each
        let mut seen = std::collections::HashSet::new();
        for (analyzer_type, options) in &needed {
            // GPU-inline preprocessors have no factory or worker thread; the deck render path
            // drives them. Passing one to `DeckAnalyzers` would log a spurious failure on every
            // load. Host-inline ones are handled above.
            if registry
                .category_for(analyzer_type)
                .is_some_and(|c| c != PreprocessorCategory::CpuAnalyzer)
            {
                continue;
            }
            if seen.insert(analyzer_type.clone())
                && self.analyzers.latest_snapshot(analyzer_type).is_none()
            {
                if self
                    .analyzers
                    .request(analyzer_type, registry, options)
                    .is_some()
                {
                    log::info!(
                        "Deck '{}': auto-started analyzer '{}'",
                        self.uuid,
                        analyzer_type
                    );
                } else {
                    log::warn!(
                        "Deck '{}': failed to start analyzer '{}'",
                        self.uuid,
                        analyzer_type
                    );
                }
            }
        }
    }

    /// Render this deck, containing any GPU error it raises.
    ///
    /// wgpu reports validation errors through a device-wide handler whose default panics, which
    /// would stop the render thread. A malformed shader instead quarantines the deck, which keeps
    /// its last good frame while everything else renders.
    ///
    /// # Errors
    ///
    /// Propagates errors raised while encoding the source and effect chain. GPU validation errors
    /// quarantine the deck and return `Ok(())`.
    pub fn render_with_prefix(
        &mut self,
        context: &GpuContext,
        audio_data: &AudioData,
        modulation: &ModulationEngine,
        param_prefix: &str,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
        gpu_timing: Option<(&wgpu::QuerySet, u32, u32)>,
    ) -> Result<()> {
        if self.gpu_error.is_some() {
            // Quarantined: the texture still holds the last good frame.
            return Ok(());
        }

        let before = cmd_buffers.len();
        let result = {
            let scope = context.errors.scope(&format!("deck {}", self.uuid));
            let result = self.render_with_prefix_inner(
                context,
                audio_data,
                modulation,
                param_prefix,
                cmd_buffers,
                gpu_timing,
            );
            match scope.faulted() {
                Some(message) => Err(message),
                None => Ok(result),
            }
        };

        match result {
            Ok(inner) => inner,
            Err(message) => {
                // Drop this deck's partial command buffers so the same error is not raised again
                // downstream.
                cmd_buffers.truncate(before);
                log::error!(
                    "Deck '{}' ({}) raised a GPU error and was disabled: {}",
                    self.uuid,
                    self.source_name,
                    message
                );
                self.gpu_error = Some(message);
                Ok(())
            }
        }
    }

    fn render_with_prefix_inner(
        &mut self,
        context: &GpuContext,
        audio_data: &AudioData,
        modulation: &ModulationEngine,
        param_prefix: &str,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
        gpu_timing: Option<(&wgpu::QuerySet, u32, u32)>,
    ) -> Result<()> {
        // Update preprocessor textures from analyzer snapshots before rendering
        if self.analyzers.has_active_instances() {
            if let Some(slots) = self.source.preprocessor_slots_mut() {
                upload_preprocessor_slots(&self.analyzers, context, slots);
            }
            for effect in &mut self.effects {
                upload_preprocessor_slots(
                    &self.analyzers,
                    context,
                    &mut effect.preprocessor_textures,
                );
            }
        }

        // Write begin GPU timestamp if timing is enabled
        if let Some((query_set, begin_idx, _)) = gpu_timing {
            let mut enc = context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("GPU Timing Begin"),
                });
            enc.write_timestamp(query_set, begin_idx);
            cmd_buffers.push(enc.finish());
        }

        // Advance render_time by a fixed dt so skipped frames don't cause animation jumps.
        let time_delta = self.render_dt;
        self.render_time += time_delta;
        let time = self.render_time;
        self.frame_count += 1;

        // Derive per-deck FPS from wall-clock render interval (for UI display only)
        let now = Instant::now();
        let wall_dt = (now - self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        if wall_dt > 0.0 && wall_dt < 1.0 {
            let instant_fps = 1.0 / wall_dt;
            self.fps_smoothed = 0.1 * instant_fps + 0.9 * self.fps_smoothed;
        }

        // Accumulate generator phase times using the fixed dt
        accumulate_phase_times(
            &mut self.phase_accumulators,
            time_delta,
            self.generator_phase_inputs.as_deref(),
            &mut self.generator_params,
            modulation,
            param_prefix,
        );
        let generator_phase_times = self.phase_accumulators;

        let enabled_effects: Vec<usize> = self
            .effects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.enabled)
            .map(|(i, _)| i)
            .collect();

        let source_to_b = enabled_effects.len() % 2 == 1;

        // Depth-sensor preprocessor passes run before the shader so its bindings hold this frame's
        // fields.
        self.run_depth_preprocess(context, cmd_buffers);

        // Host-inline preprocessors step for this frame before anything draws with their outputs.
        if !self.host_inline.is_empty() {
            self.step_host_inline(
                context,
                time_delta,
                generator_phase_times,
                modulation,
                param_prefix,
            );
        }

        let (target, target_texture) = if source_to_b {
            (&self.texture_b_view, &self.texture_b)
        } else {
            (&self.texture_view, &self.texture)
        };
        let mut frame = SourceFrame {
            gpu: context,
            target,
            target_texture,
            width: self.texture.width(),
            height: self.texture.height(),
            transparent: self.transparent,
            time,
            time_delta,
            frame_index: self.frame_count,
            phase_times: generator_phase_times,
            audio: audio_data,
            modulation,
            param_prefix,
            params: &mut self.generator_params,
            cmd_buffers,
        };
        self.source.render(&mut frame)?;

        // Apply effect chain (ping-pong between textures)
        let mut read_from_b = source_to_b;
        for &effect_idx in &enabled_effects {
            // Accumulate phase times for this effect
            let effect = &mut self.effects[effect_idx];
            let Effect {
                phase_accumulators,
                phase_inputs_config,
                params,
                param_prefix,
                ..
            } = effect;
            accumulate_phase_times(
                phase_accumulators,
                time_delta,
                phase_inputs_config.as_deref(),
                params,
                modulation,
                param_prefix,
            );
            let effect_phase_times = effect.phase_accumulators;

            let uniforms = crate::generator::pass::uniforms(
                audio_data,
                time,
                time_delta,
                self.frame_count,
                0,
                [self.texture.width() as f32, self.texture.height() as f32],
                effect_phase_times,
            );
            let (input_view, output_view) = if read_from_b {
                (&self.texture_b_view, &self.texture_view)
            } else {
                (&self.texture_view, &self.texture_b_view)
            };
            self.effects[effect_idx].apply_with_modulation(
                context,
                input_view,
                output_view,
                &uniforms,
                Some(modulation),
                cmd_buffers,
            )?;
            read_from_b = !read_from_b;
        }

        // Capture the frame and the live values each analyzer declared. The state is immutable once
        // sent to the worker, so an async result is tagged with the exact map it evaluated.
        let mut analyzer_states = HashMap::new();
        if self.analyzers.has_active_instances() {
            collect_preprocessor_state(
                self.source.preprocessor_slots(),
                &mut self.generator_params,
                generator_phase_times,
                modulation,
                param_prefix,
                &mut analyzer_states,
            );
            for effect in &mut self.effects {
                collect_preprocessor_state(
                    &effect.preprocessor_textures,
                    &mut effect.params,
                    effect.phase_accumulators,
                    modulation,
                    &effect.param_prefix,
                    &mut analyzer_states,
                );
            }
        }
        if let Some(readback_cmd) =
            self.analyzers
                .capture_frame(&context.device, &self.texture, &analyzer_states)
        {
            cmd_buffers.push(readback_cmd);
        }

        // Write end GPU timestamp if timing is enabled
        if let Some((query_set, _, end_idx)) = gpu_timing {
            let mut enc = context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("GPU Timing End"),
                });
            enc.write_timestamp(query_set, end_idx);
            cmd_buffers.push(enc.finish());
        }

        Ok(())
    }

    /// Step the host-inline preprocessors with this frame's bound values and upload their
    /// outputs into the preprocessor slots that declare them.
    fn step_host_inline(
        &mut self,
        context: &GpuContext,
        time_delta: f32,
        generator_phase_times: [f32; 4],
        modulation: &ModulationEngine,
        param_prefix: &str,
    ) {
        let mut states = HashMap::new();
        collect_preprocessor_state(
            self.source.preprocessor_slots(),
            &mut self.generator_params,
            generator_phase_times,
            modulation,
            param_prefix,
            &mut states,
        );
        for effect in &mut self.effects {
            collect_preprocessor_state(
                &effect.preprocessor_textures,
                &mut effect.params,
                effect.phase_accumulators,
                modulation,
                &effect.param_prefix,
                &mut states,
            );
        }
        self.host_inline.step(
            time_delta,
            self.frame_count,
            (self.texture.width(), self.texture.height()),
            &states,
        );
        let host_inline = &self.host_inline;
        let upload = |slots: &mut [PreprocessorSlot]| {
            for slot in slots {
                if let Some(tex_data) = host_inline
                    .latest(&slot.analyzer_type)
                    .and_then(|snapshot| snapshot.textures.get(&slot.name))
                {
                    upload_texture_to_slot(&context.device, &context.queue, slot, tex_data);
                }
            }
        };
        if let Some(slots) = self.source.preprocessor_slots_mut() {
            upload(slots);
        }
        for effect in &mut self.effects {
            upload(&mut effect.preprocessor_textures);
        }
    }

    /// Run the depth-sensor preprocessor's conversion passes for this frame.
    ///
    /// Skipped unless a new sensor frame arrived since the last run (sensor ~30 Hz, deck typically
    /// 60), and while the sensor is disconnected, which holds the outputs at their last good
    /// values.
    fn run_depth_preprocess(
        &mut self,
        context: &GpuContext,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) {
        let Some(state) = &mut self.depth_prepro else {
            return;
        };
        let Some(input) = &state.input else {
            return;
        };

        if !input.connected {
            if !state.warned_disconnected {
                log::warn!(
                    "Deck '{}': depth sensor {} disconnected — preprocessor outputs frozen",
                    self.uuid,
                    state.sensor_id
                );
                state.warned_disconnected = true;
            }
            return;
        }
        if state.warned_disconnected {
            log::info!(
                "Deck '{}': depth sensor {} reconnected",
                self.uuid,
                state.sensor_id
            );
            state.warned_disconnected = false;
        }

        if state.last_generation == Some(input.generation) {
            return;
        }
        state.last_generation = Some(input.generation);

        state
            .pipeline
            .update_uniform(&context.queue, &state.params, input.frame_dt);
        let rgb = state.wants_rgb.then_some(&input.rgb_view);
        // Split the borrow: `run` needs `&mut pipeline` while `input` is behind
        // the same `&mut state`, so clone the cheap view handles first.
        let depth_view = input.depth_view.clone();
        let rgb_view = rgb.cloned();
        state
            .pipeline
            .run(&context.device, &depth_view, rgb_view.as_ref(), cmd_buffers);
    }

    /// Resize the deck's render targets, and let the source redraw for the new
    /// size (vector art re-rasterizes rather than being magnified).
    pub fn resize(&mut self, context: &GpuContext, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        (self.texture, self.texture_view) =
            super::source::deck_target(context, "Deck Texture (Linear)", width, height);
        (self.texture_b, self.texture_b_view) =
            super::source::deck_target(context, "Deck Texture B (Linear)", width, height);
        self.source.resize(context, width, height);
    }

    /// Get the final output texture view (after effect chain)
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.texture_view
    }
}

/// Upload analyzer texture data into preprocessor slots.
fn upload_preprocessor_slots(
    analyzers: &DeckAnalyzers,
    context: &GpuContext,
    slots: &mut [PreprocessorSlot],
) {
    for slot in slots {
        if let Some(snapshot) = analyzers.latest_snapshot(&slot.analyzer_type)
            && let Some(tex_data) = snapshot.textures.get(&slot.name)
        {
            upload_texture_to_slot(&context.device, &context.queue, slot, tex_data);
        }
    }
}

/// Every analyzer and preprocessor a deck can declare: the analyzer module's
/// own, plus the depth sensor's.
pub(crate) fn analyzer_registry() -> AnalyzerRegistry {
    crate::depth::preprocess::register(crate::analyzer::default_registry())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isf::ISFInput;
    use crate::isf::PhaseInput;
    use crate::modulation::{ModulationSource, StepInterpolation};
    use crate::params::ShaderParams;

    /// Empty engine — every parameter reads back its base value.
    fn no_modulation() -> ModulationEngine {
        ModulationEngine::new()
    }

    /// A unipolar step sequencer at 0 Hz holds +1.0, so modulation depth is exactly `amount`.
    /// Bipolar sources are range-scaled by 0.5 and would halve the depth.
    fn constant_modulation(target: &str, amount: f32) -> ModulationEngine {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::StepSequencer {
            steps: vec![1.0; 2],
            rate: 0.0,
            interpolation: StepInterpolation::None,
            bipolar: false,
        });
        engine.assign(target, &uuid, amount);
        engine.update_free_running(
            0.0,
            &crate::modulation::AudioValues::default(),
            &crate::modulation::AnalyzerValues::default(),
        );
        engine
    }

    fn phase(param: &str, index: usize, scale: f32) -> PhaseInput {
        PhaseInput {
            param: param.into(),
            index,
            scale,
            multiply_by: Vec::new(),
        }
    }

    fn phase_product(param: &str, multiply_by: &[&str], index: usize, scale: f32) -> PhaseInput {
        PhaseInput {
            multiply_by: multiply_by.iter().map(|s| (*s).to_string()).collect(),
            ..phase(param, index, scale)
        }
    }

    fn float_input(name: &str, default: f64, min: f32, max: f32) -> ISFInput {
        ISFInput {
            name: name.into(),
            input_type: "float".into(),
            default: Some(serde_json::json!(default)),
            min: Some(min),
            max: Some(max),
            label: None,
            values: None,
            labels: None,
            identity: None,
            group: None,
        }
    }

    #[test]
    fn isf_uniforms_size_is_96_bytes() {
        // Shaders declare the block themselves, so fields are only ever
        // appended: a shader declaring the older, shorter block still binds.
        assert_eq!(
            std::mem::size_of::<crate::renderer::ISFUniforms>(),
            96,
            "80 bytes through PHASE_TIME_3, then JITTER, JITTERINDEX, HISTORYVALID"
        );
    }

    #[test]
    fn accumulate_phase_times_basic() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase("speed", 0, 1.0)];
        let isf_inputs = vec![float_input("speed", 2.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&isf_inputs);
        let modulation = no_modulation();

        // dt=0.1, speed=2.0, scale=1.0 → accumulate 0.2
        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &modulation,
            "deck0",
        );
        assert!((accum[0] - 0.2).abs() < 1e-5);
        assert_eq!(accum[1], 0.0);

        // Accumulate again: 0.2 + 0.2 = 0.4
        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &modulation,
            "deck0",
        );
        assert!((accum[0] - 0.4).abs() < 1e-5);
    }

    #[test]
    fn accumulate_phase_times_with_scale() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase("speed", 0, 0.3)];
        let isf_inputs = vec![float_input("speed", 1.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        // dt=0.5, speed=1.0, scale=0.3 → 0.15
        accumulate_phase_times(
            &mut accum,
            0.5,
            Some(&inputs),
            &mut params,
            &no_modulation(),
            "deck0",
        );
        assert!((accum[0] - 0.15).abs() < 1e-5);
    }

    #[test]
    fn accumulate_phase_times_speed_change_is_continuous() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase("speed", 0, 1.0)];
        let isf_inputs = vec![float_input("speed", 1.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&isf_inputs);
        let modulation = no_modulation();

        // Run 10 frames at speed=1.0, dt=0.016
        for _ in 0..10 {
            accumulate_phase_times(
                &mut accum,
                0.016,
                Some(&inputs),
                &mut params,
                &modulation,
                "deck0",
            );
        }
        let before_change = accum[0];

        // Change speed to 3.0 — no jump should occur
        params.set_float("speed", 3.0);
        accumulate_phase_times(
            &mut accum,
            0.016,
            Some(&inputs),
            &mut params,
            &modulation,
            "deck0",
        );
        let after_change = accum[0];

        // Value should increase by dt*3.0, not jump to TIME*3.0
        let expected_delta = 0.016 * 3.0;
        assert!(
            (after_change - before_change - expected_delta).abs() < 1e-5,
            "Phase time should be continuous: before={before_change}, after={after_change}, expected delta={expected_delta}"
        );
    }

    #[test]
    fn accumulate_phase_times_multi_index() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![
            phase("speed", 0, 1.0),
            phase("rot_x", 1, 1.0),
            phase("rot_y", 2, 1.0),
            phase("rot_z", 3, 1.0),
        ];
        let isf_inputs = vec![
            float_input("speed", 1.0, 0.0, 5.0),
            float_input("rot_x", 0.5, -1.0, 1.0),
            float_input("rot_y", 0.3, -1.0, 1.0),
            float_input("rot_z", 0.0, -1.0, 1.0),
        ];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &no_modulation(),
            "deck0",
        );
        assert!((accum[0] - 0.1).abs() < 1e-5); // speed=1.0 * 0.1
        assert!((accum[1] - 0.05).abs() < 1e-5); // rot_x=0.5 * 0.1
        assert!((accum[2] - 0.03).abs() < 1e-5); // rot_y=0.3 * 0.1
        assert!((accum[3] - 0.0).abs() < 1e-5); // rot_z=0.0 * 0.1
    }

    #[test]
    fn accumulate_phase_times_none_is_noop() {
        let mut accum = [0.0f32; 4];
        let mut params = ShaderParams::from_inputs(&[]);
        accumulate_phase_times(
            &mut accum,
            0.1,
            None,
            &mut params,
            &no_modulation(),
            "deck0",
        );
        assert_eq!(accum, [0.0; 4]);
    }

    #[test]
    fn accumulate_phase_times_uses_modulated_value() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase("speed", 0, 1.0)];
        let isf_inputs = vec![float_input("speed", 1.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        // amount 0.5 over a 0..5 range lifts the effective speed by 2.5 → 3.5
        let modulation = constant_modulation("deck0/speed", 0.5);
        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &modulation,
            "deck0",
        );
        assert!(
            (accum[0] - 0.35).abs() < 1e-5,
            "modulated speed 3.5 over dt 0.1 should accumulate 0.35, got {}",
            accum[0]
        );
    }

    #[test]
    fn accumulate_phase_times_ignores_modulation_of_other_prefixes() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase("speed", 0, 1.0)];
        let isf_inputs = vec![float_input("speed", 1.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        let modulation = constant_modulation("deck9/speed", 0.5);
        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &modulation,
            "deck0",
        );
        assert!((accum[0] - 0.1).abs() < 1e-5);
    }

    #[test]
    fn accumulate_phase_times_modulation_onset_is_continuous() {
        let inputs = vec![phase("speed", 0, 1.0)];
        let isf_inputs = vec![float_input("speed", 1.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        let unmodulated = no_modulation();
        let mut accum = [0.0f32; 4];
        for _ in 0..10 {
            accumulate_phase_times(
                &mut accum,
                0.016,
                Some(&inputs),
                &mut params,
                &unmodulated,
                "deck0",
            );
        }
        let before = accum[0];

        // Modulation kicking in changes the rate, never the phase itself.
        let modulated = constant_modulation("deck0/speed", 0.4);
        accumulate_phase_times(
            &mut accum,
            0.016,
            Some(&inputs),
            &mut params,
            &modulated,
            "deck0",
        );
        let expected_delta = 0.016 * (1.0 + 0.4 * 5.0);
        assert!(
            (accum[0] - before - expected_delta).abs() < 1e-5,
            "phase should stay continuous across modulation onset: before={before}, after={}, expected delta={expected_delta}",
            accum[0]
        );
    }

    #[test]
    fn accumulate_phase_times_multiplies_second_param_into_rate() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase_product("speed", &["rot_speed"], 0, 0.5)];
        let isf_inputs = vec![
            float_input("speed", 2.0, 0.0, 5.0),
            float_input("rot_speed", 3.0, 0.0, 5.0),
        ];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        // dt 0.1 × speed 2.0 × rot_speed 3.0 × scale 0.5 → 0.3
        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &no_modulation(),
            "deck0",
        );
        assert!((accum[0] - 0.3).abs() < 1e-5, "got {}", accum[0]);
    }

    #[test]
    fn accumulate_phase_times_product_responds_to_modulating_either_operand() {
        let inputs = vec![phase_product("speed", &["rot_speed"], 0, 1.0)];
        let isf_inputs = vec![
            float_input("speed", 1.0, 0.0, 5.0),
            float_input("rot_speed", 1.0, 0.0, 5.0),
        ];

        // Modulating the second operand must move the rate just as the first does.
        for target in ["deck0/speed", "deck0/rot_speed"] {
            let mut params = ShaderParams::from_inputs(&isf_inputs);
            let mut accum = [0.0f32; 4];
            accumulate_phase_times(
                &mut accum,
                0.1,
                Some(&inputs),
                &mut params,
                &constant_modulation(target, 0.2),
                "deck0",
            );
            // The modulated operand becomes 1.0 + 0.2 * 5.0 = 2.0, the other stays 1.0.
            assert!(
                (accum[0] - 0.2).abs() < 1e-5,
                "modulating {target} should double the rate, got {}",
                accum[0]
            );
        }
    }

    #[test]
    fn accumulate_phase_times_multiplies_every_listed_factor() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase_product(
            "speed",
            &["time_scale", "flow_speed"],
            0,
            1.0,
        )];
        let isf_inputs = vec![
            float_input("speed", 2.0, 0.0, 5.0),
            float_input("time_scale", 1.5, 0.0, 5.0),
            float_input("flow_speed", 3.0, 0.0, 5.0),
        ];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        // dt 0.1 × 2.0 × 1.5 × 3.0 → 0.9
        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &no_modulation(),
            "deck0",
        );
        assert!((accum[0] - 0.9).abs() < 1e-5, "got {}", accum[0]);
    }

    #[test]
    fn accumulate_phase_times_missing_multiply_by_target_is_unit_factor() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase_product("speed", &["not_a_param"], 0, 1.0)];
        let isf_inputs = vec![float_input("speed", 2.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &no_modulation(),
            "deck0",
        );
        assert!((accum[0] - 0.2).abs() < 1e-5, "got {}", accum[0]);
    }

    #[test]
    fn accumulate_phase_times_product_is_continuous_across_operand_change() {
        let inputs = vec![phase_product("speed", &["rot_speed"], 0, 1.0)];
        let isf_inputs = vec![
            float_input("speed", 1.0, 0.0, 5.0),
            float_input("rot_speed", 1.0, 0.0, 5.0),
        ];
        let mut params = ShaderParams::from_inputs(&isf_inputs);
        let modulation = no_modulation();

        let mut accum = [0.0f32; 4];
        for _ in 0..10 {
            accumulate_phase_times(
                &mut accum,
                0.016,
                Some(&inputs),
                &mut params,
                &modulation,
                "deck0",
            );
        }
        let before = accum[0];

        params.set_float("rot_speed", 4.0);
        accumulate_phase_times(
            &mut accum,
            0.016,
            Some(&inputs),
            &mut params,
            &modulation,
            "deck0",
        );
        let expected_delta = 0.016 * 4.0;
        assert!(
            (accum[0] - before - expected_delta).abs() < 1e-5,
            "phase should stay continuous when the second operand changes: before={before}, after={}",
            accum[0]
        );
    }

    #[test]
    fn accumulate_phase_times_modulation_clamps_to_param_range() {
        let mut accum = [0.0f32; 4];
        let inputs = vec![phase("speed", 0, 1.0)];
        let isf_inputs = vec![float_input("speed", 4.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&isf_inputs);

        // 4.0 + 1.0 * 5.0 would be 9.0; the parameter max caps it at 5.0
        let modulation = constant_modulation("deck0/speed", 1.0);
        accumulate_phase_times(
            &mut accum,
            0.1,
            Some(&inputs),
            &mut params,
            &modulation,
            "deck0",
        );
        assert!((accum[0] - 0.5).abs() < 1e-5, "got {}", accum[0]);
    }

    // ── Zero-size texture guard on resize ───────────────────────────

    #[test]
    fn deck_resize_zero_dimensions_does_not_panic() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut deck = crate::deck::Deck::solid_color(&gpu, [1.0, 0.0, 0.0, 1.0], 64, 64);

        // Zero width — must not panic (clamped to 1)
        deck.resize(&gpu, 0, 64);

        // Zero height — must not panic (clamped to 1)
        deck.resize(&gpu, 64, 0);

        // Both zero — must not panic (clamped to 1x1)
        deck.resize(&gpu, 0, 0);

        // Normal resize still works
        deck.resize(&gpu, 128, 128);
    }

    /// A generator whose `step` parameter is bound into the test counter.
    fn counter_deck(gpu: &GpuContext) -> crate::deck::Deck {
        use crate::analyzer::host_inline::tests::{COUNTER, registry};
        let source = format!(
            r#"/*{{
    "INPUTS": [{{"NAME": "step", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.0, "MAX": 10.0}}],
    "PREPROCESSORS": [{{"NAME": "count", "TYPE": "{COUNTER}", "PARAM_BINDINGS": {{"step": "step"}}}}]
}}*/
#version 450
layout(location = 0) out vec4 fragColor;
void main() {{ fragColor = vec4(1.0); }}
"#
        );
        let shader = crate::isf::ISFShader::from_string(&source).expect("parse");
        let mut deck = crate::deck::Deck::from_shader(gpu, shader, 64, 64).expect("deck");
        deck.ensure_preprocessor_analyzers(&registry());
        deck
    }

    fn render(deck: &mut crate::deck::Deck, gpu: &GpuContext) {
        let mut cmd_buffers = Vec::new();
        deck.render(
            gpu,
            &AudioData::default(),
            &no_modulation(),
            0,
            &mut cmd_buffers,
        )
        .expect("render");
        gpu.queue.submit(cmd_buffers);
    }

    fn count(deck: &crate::deck::Deck) -> f32 {
        use crate::analyzer::host_inline::tests::COUNTER;
        deck.host_inline
            .latest(COUNTER)
            .expect("running")
            .scalar("count")
    }

    #[test]
    fn host_inline_preprocessors_step_once_per_frame_with_bound_parameters() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut deck = counter_deck(&gpu);
        render(&mut deck, &gpu);
        render(&mut deck, &gpu);
        assert_eq!(count(&deck), 4.0, "two frames at the bound step of 2");
    }

    #[test]
    fn host_inline_state_is_saved_with_the_source_config() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut deck = counter_deck(&gpu);
        render(&mut deck, &gpu);
        let saved = deck.source_config();
        let states = saved
            .get("preprocessor_state")
            .and_then(serde_json::Value::as_object)
            .expect("preprocessor_state saved")
            .clone();

        let mut restored = counter_deck(&gpu);
        restored.restore_preprocessor_state(&states);
        render(&mut restored, &gpu);
        assert_eq!(count(&restored), 4.0, "resumed from 2 and stepped by 2");
    }
}
