//! Depth-sensor shader preprocessor — GPU-inline conversion of a raw sensor
//! stream into render-ready fields for ISF shaders.
//!
//! A shader declares `{"NAME": "mask", "TYPE": "depth_sensor"}` in its ISF
//! `PREPROCESSORS` block; the engine acquires an attached sensor through the
//! ref-counted [`crate::depth::DepthSensorManager`] and this pipeline converts
//! the shared `R16Uint` depth texture into `depth` / `mask` / `motion` / `rgb`.
//!
//! Nothing here touches host memory: the sensor's pixels are already GPU
//! resident and stay there. See preprocess.wgsl and
//! spec/depth-sensor-preprocessor.md.

use crate::analyzer::traits::{AnalyzerSchema, TextureOutputDef};

/// The ISF `TYPE` string shaders declare to request this preprocessor.
pub const PREPROCESSOR_TYPE: &str = "depth_sensor";

/// Millimetre span the normalized `near`/`far` faders address.
const RANGE_MM: f32 = 8000.0;
/// Maximum hole-fill / feather radius, matching `MAX_RADIUS` in preprocess.wgsl.
const MAX_RADIUS: f32 = 8.0;

/// One shader-visible output of the `depth_sensor` preprocessor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// Distance normalized to `0..1` across `[near, far]`; `0.0` is invalid.
    Depth,
    /// Feathered silhouette occupancy.
    Mask,
    /// Approximate screen-space velocity of the depth surface, UV units/second.
    Motion,
    /// The sensor's colour stream, mirrored to match the depth outputs.
    Rgb,
}

impl Output {
    /// All outputs, in binding-allocation order.
    pub const ALL: [Output; 4] = [Output::Depth, Output::Mask, Output::Motion, Output::Rgb];

    /// The ISF `NAME` a shader uses to select this output.
    pub fn name(self) -> &'static str {
        match self {
            Output::Depth => "depth",
            Output::Mask => "mask",
            Output::Motion => "motion",
            Output::Rgb => "rgb",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Output::ALL.into_iter().find(|o| o.name() == name)
    }

    /// Schema format key, resolved by [`crate::analyzer::traits::texture_format_from_str`].
    pub fn format_key(self) -> &'static str {
        match self {
            Output::Depth => "r16float",
            Output::Mask => "r8unorm",
            Output::Motion => "rg16float",
            Output::Rgb => "color_path",
        }
    }

    /// Resolved from [`Self::format_key`] so the schema string and the texture
    /// the pipeline actually allocates cannot drift apart.
    pub fn wgpu_format(self) -> wgpu::TextureFormat {
        crate::analyzer::traits::texture_format_from_str(self.format_key())
            .expect("depth_sensor output format keys are resolvable")
    }

    fn description(self) -> &'static str {
        match self {
            Output::Depth => "Normalized depth in [near, far]; 0 = invalid/out of range",
            Output::Mask => "Feathered silhouette occupancy",
            Output::Motion => "Approximate depth-surface velocity, UV units per second",
            Output::Rgb => "Sensor colour stream, mirrored to match depth",
        }
    }
}

/// Schema published to the preprocessor registry.
pub(crate) fn schema() -> AnalyzerSchema {
    AnalyzerSchema {
        scalars: Vec::new(),
        textures: Output::ALL
            .into_iter()
            .map(|o| TextureOutputDef {
                name: o.name().to_string(),
                description: o.description().to_string(),
                format: o.format_key().to_string(),
            })
            .collect(),
    }
}

/// Router-exposed preprocessor parameters (`deck/<uuid>/depth_prepro/*`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthPreprocessParams {
    pub near_mm: f32,
    pub far_mm: f32,
    /// Temporal EMA factor; higher smooths more but smears fast motion.
    pub smoothing: f32,
    /// Nearest-valid search radius for unresolved texels, in texels.
    pub hole_fill: f32,
    /// Silhouette edge softness, in texels.
    pub mask_feather: f32,
    /// Multiplier on the `motion` output.
    pub motion_gain: f32,
    /// Flip X. On by default — the sensor faces the audience, so unmirrored
    /// output moves the wrong way relative to the projection.
    pub mirror: bool,
}

impl Default for DepthPreprocessParams {
    fn default() -> Self {
        Self {
            near_mm: 500.0,
            far_mm: 4000.0,
            smoothing: 0.5,
            hole_fill: 2.0,
            mask_feather: 3.0,
            motion_gain: 0.4 * 8.0,
            mirror: true,
        }
    }
}

impl DepthPreprocessParams {
    /// Denormalize a fader value (`0..1`) into the physical range of `name` and
    /// apply it. Returns `false` for an unknown param (nothing mutated).
    ///
    /// Split out from `Deck::set_depth_prepro_param` so the mapping is unit
    /// testable without a live sensor, mirroring `PointCloudParams`.
    pub(crate) fn set_normalized_param(&mut self, name: &str, value: f32) -> bool {
        let v = value.clamp(0.0, 1.0);
        match name {
            "near" => {
                self.near_mm = v * RANGE_MM;
                // Keep the range non-degenerate; `far` always stays above `near`.
                self.far_mm = self.far_mm.max(self.near_mm + 1.0);
            }
            "far" => self.far_mm = (v * RANGE_MM).max(self.near_mm + 1.0),
            "smoothing" => self.smoothing = v,
            "hole_fill" => self.hole_fill = v * 4.0,
            "mask_feather" => self.mask_feather = v * MAX_RADIUS,
            "motion_gain" => self.motion_gain = v * 8.0,
            "mirror" => self.mirror = v >= 0.5,
            _ => return false,
        }
        true
    }

    /// Normalized (`0..1`) value of `name`, for snapshots and UI readback.
    pub(crate) fn normalized_param(&self, name: &str) -> Option<f32> {
        Some(match name {
            "near" => self.near_mm / RANGE_MM,
            "far" => self.far_mm / RANGE_MM,
            "smoothing" => self.smoothing,
            "hole_fill" => self.hole_fill / 4.0,
            "mask_feather" => self.mask_feather / MAX_RADIUS,
            "motion_gain" => self.motion_gain / 8.0,
            "mirror" => f32::from(u8::from(self.mirror)),
            _ => return None,
        })
    }
}

/// The sensor index a shader's `PREPROCESSORS` block asks for, if it declares
/// this preprocessor at all.
///
/// `OPTIONS: {"device": N}` pins a specific sensor; absent, the first enumerated
/// device is used. Deliberately not a live router param — switching sensors means
/// rebuilding the pipeline and its textures, which is not a mid-set operation.
pub fn requested_device(metadata: &crate::isf::ISFMetadata) -> Option<u32> {
    let mut found = false;
    let mut index = 0;
    for pp in &metadata.preprocessors {
        if pp.preprocessor_type != PREPROCESSOR_TYPE {
            continue;
        }
        found = true;
        if let Some(d) = pp.options.get("device").and_then(serde_json::Value::as_u64) {
            index = u32::try_from(d).unwrap_or(0);
        }
    }
    found.then_some(index)
}

/// Resolve and acquire the sensor a shader requires, building its pipeline.
///
/// Returns `Ok(None)` when the shader declares no `depth_sensor` preprocessor.
/// Returns `Err` when it declares one and no sensor can be acquired: this is a
/// *required* preprocessor, so the caller must abort the load rather than
/// degrade. See /spec/effect-preprocessing.md § Required Preprocessors.
///
/// The sensor is opened through the ref-counted manager, so it is shared with
/// any point-cloud decks or other preprocessor decks already using it.
pub fn acquire_for_shader(
    manager: &mut super::DepthSensorManager,
    device: &wgpu::Device,
    metadata: &crate::isf::ISFMetadata,
    shader_name: &str,
) -> anyhow::Result<Option<AcquiredSensor>> {
    let Some(requested) = requested_device(metadata) else {
        return Ok(None);
    };

    let detected = manager.devices().len();
    if detected == 0 {
        anyhow::bail!(
            "Shader '{shader_name}' requires a depth sensor — none detected. \
             Connect a Kinect and use Rescan."
        );
    }
    let Some(info) = manager
        .devices()
        .iter()
        .find(|d| d.id == requested)
        .cloned()
    else {
        anyhow::bail!(
            "Shader '{shader_name}' requires depth sensor #{requested} — only {detected} detected."
        );
    };

    let (width, height) = super::open_depth_sensor(manager, info.id, device).map_err(|e| {
        anyhow::anyhow!(
            "Failed to open depth sensor '{}' for shader '{shader_name}': {e}",
            info.name
        )
    })?;

    log::info!(
        "Shader '{shader_name}': acquired depth sensor '{}' ({width}x{height}) for preprocessing",
        info.name
    );
    Ok(Some(AcquiredSensor {
        id: info.id,
        name: info.name,
        pipeline: DepthPreprocessPipeline::new(device, width, height),
    }))
}

/// A sensor reference acquired for a shader, with its ready-to-run pipeline.
pub struct AcquiredSensor {
    pub id: super::DepthSensorId,
    pub name: String,
    pub pipeline: DepthPreprocessPipeline,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuParams {
    // near_mm, far_mm, 1/(far-near), smoothing
    range: [f32; 4],
    // width, height, hole_fill radius, mask_feather radius
    dims: [f32; 4],
    // motion_gain, dt, mirror, pad
    misc: [f32; 4],
}

/// Owned output textures plus the passes that fill them.
///
/// The textures live here rather than in the deck's `PreprocessorSlot`s: the
/// slots hold cheap `wgpu::Texture`/`TextureView` clones of these, so the
/// shader's bind group stays valid regardless of what the device does. See
/// spec/effect-preprocessing.md Decision #4.
pub struct DepthPreprocessPipeline {
    normalize: wgpu::RenderPipeline,
    mask: wgpu::RenderPipeline,
    color: wgpu::RenderPipeline,
    normalize_bgl: wgpu::BindGroupLayout,
    mask_bgl: wgpu::BindGroupLayout,
    color_bgl: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,

    outputs: Vec<(Output, wgpu::Texture, wgpu::TextureView)>,
    /// Ping-pong depth history for temporal smoothing and motion differencing.
    history: [wgpu::TextureView; 2],
    _history_tex: [wgpu::Texture; 2],
    /// Ping-pong silhouette history, for the mask's temporal hysteresis.
    mask_history: [wgpu::TextureView; 2],
    _mask_history_tex: [wgpu::Texture; 2],
    read_idx: usize,

    width: u32,
    height: u32,
}

impl DepthPreprocessPipeline {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Depth Preprocess Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("preprocess.wgsl").into()),
        });

        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let tex_entry =
            |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
        // Every read is a `textureLoad`, so nothing needs a sampler and nothing
        // needs to be filterable.
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };

        let normalize_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Depth Preprocess Normalize BGL"),
            entries: &[
                uniform_entry,
                tex_entry(1, wgpu::TextureSampleType::Uint),
                tex_entry(2, unfiltered),
            ],
        });
        let mask_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Depth Preprocess Mask BGL"),
            entries: &[
                uniform_entry,
                tex_entry(3, unfiltered),
                tex_entry(5, unfiltered),
            ],
        });
        let color_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Depth Preprocess Color BGL"),
            entries: &[uniform_entry, tex_entry(4, unfiltered)],
        });

        let make_pipeline =
            |label: &str,
             bgl: &wgpu::BindGroupLayout,
             entry: &str,
             targets: &[Option<wgpu::ColorTargetState>]| {
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[Some(bgl)],
                    immediate_size: 0,
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_fullscreen"),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry),
                        targets,
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };

        let plain = |format: wgpu::TextureFormat| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };

        let normalize = make_pipeline(
            "Depth Preprocess Normalize",
            &normalize_bgl,
            "fs_normalize",
            &[
                plain(Output::Depth.wgpu_format()),
                plain(wgpu::TextureFormat::R16Float),
                plain(Output::Motion.wgpu_format()),
            ],
        );
        let mask = make_pipeline(
            "Depth Preprocess Mask",
            &mask_bgl,
            "fs_mask",
            &[
                plain(Output::Mask.wgpu_format()),
                plain(Output::Mask.wgpu_format()),
            ],
        );
        let color = make_pipeline(
            "Depth Preprocess Color",
            &color_bgl,
            "fs_color",
            &[plain(Output::Rgb.wgpu_format())],
        );

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Depth Preprocess Uniform"),
            size: std::mem::size_of::<GpuParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let make_texture = |label: &str, format: wgpu::TextureFormat| {
            device.create_texture(&wgpu::TextureDescriptor {
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
                // COPY_SRC so the fields can be read back — used by the GPU
                // behaviour tests, and the only way to debug this pass on real
                // hardware.
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };

        // All four outputs are allocated regardless of what the shader declared.
        // The total is ~4.6 MB at VGA, and unconditional allocation keeps slot
        // resolution a pure lookup with no ordering dependency on the ISF header.
        let outputs = Output::ALL
            .into_iter()
            .map(|o| {
                let tex = make_texture(&format!("Depth Preprocess {}", o.name()), o.wgpu_format());
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                (o, tex, view)
            })
            .collect();

        let history_tex = [
            make_texture("Depth Preprocess History 0", wgpu::TextureFormat::R16Float),
            make_texture("Depth Preprocess History 1", wgpu::TextureFormat::R16Float),
        ];
        let history = [
            history_tex[0].create_view(&wgpu::TextureViewDescriptor::default()),
            history_tex[1].create_view(&wgpu::TextureViewDescriptor::default()),
        ];
        let mask_history_tex = [
            make_texture("Depth Preprocess Mask History 0", Output::Mask.wgpu_format()),
            make_texture("Depth Preprocess Mask History 1", Output::Mask.wgpu_format()),
        ];
        let mask_history = [
            mask_history_tex[0].create_view(&wgpu::TextureViewDescriptor::default()),
            mask_history_tex[1].create_view(&wgpu::TextureViewDescriptor::default()),
        ];

        Self {
            normalize,
            mask,
            color,
            normalize_bgl,
            mask_bgl,
            color_bgl,
            uniform,
            outputs,
            history,
            _history_tex: history_tex,
            mask_history,
            _mask_history_tex: mask_history_tex,
            read_idx: 0,
            width,
            height,
        }
    }

    pub fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Clone the texture + view for one output, for a deck's `PreprocessorSlot`.
    /// Both are `Arc`-backed handles to the same GPU resource — no copy.
    pub fn output(&self, output: Output) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        self.outputs
            .iter()
            .find(|(o, _, _)| *o == output)
            .map(|(_, t, v)| (t.clone(), v.clone()))
    }

    fn view_of(&self, output: Output) -> &wgpu::TextureView {
        &self
            .outputs
            .iter()
            .find(|(o, _, _)| *o == output)
            .expect("all outputs are allocated in new()")
            .2
    }

    /// Upload this frame's params. `dt` is the sensor's inter-frame interval, not
    /// the render interval — using render dt would misreport velocity whenever
    /// the deck runs faster than the sensor.
    pub fn update_uniform(&self, queue: &wgpu::Queue, params: &DepthPreprocessParams, dt: f32) {
        let span = (params.far_mm - params.near_mm).max(1.0);
        let gpu = GpuParams {
            range: [
                params.near_mm,
                params.far_mm,
                1.0 / span,
                params.smoothing.clamp(0.0, 0.99),
            ],
            dims: [
                self.width as f32,
                self.height as f32,
                params.hole_fill.clamp(0.0, MAX_RADIUS),
                params.mask_feather.clamp(0.0, MAX_RADIUS),
            ],
            misc: [
                params.motion_gain,
                dt.max(1.0e-4),
                f32::from(u8::from(params.mirror)),
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&gpu));
    }

    /// Run the conversion passes for one sensor frame.
    ///
    /// `rgb_src` is `None` when the shader did not declare the `rgb` output, in
    /// which case the colour pass is skipped entirely.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        depth_src: &wgpu::TextureView,
        rgb_src: Option<&wgpu::TextureView>,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) {
        let write_idx = 1 - self.read_idx;

        let normalize_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Depth Preprocess Normalize BG"),
            layout: &self.normalize_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_src),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.history[self.read_idx]),
                },
            ],
        });
        let mask_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Depth Preprocess Mask BG"),
            layout: &self.mask_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(self.view_of(Output::Depth)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&self.mask_history[self.read_idx]),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Depth Preprocess Encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Depth Preprocess Normalize Pass"),
                color_attachments: &[
                    attachment(self.view_of(Output::Depth)),
                    attachment(&self.history[write_idx]),
                    attachment(self.view_of(Output::Motion)),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.normalize);
            pass.set_bind_group(0, &normalize_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Depth Preprocess Mask Pass"),
                color_attachments: &[
                    attachment(self.view_of(Output::Mask)),
                    attachment(&self.mask_history[write_idx]),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.mask);
            pass.set_bind_group(0, &mask_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(rgb) = rgb_src {
            let color_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Depth Preprocess Color BG"),
                layout: &self.color_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(rgb),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Depth Preprocess Color Pass"),
                color_attachments: &[attachment(self.view_of(Output::Rgb))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.color);
            pass.set_bind_group(0, &color_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        cmd_buffers.push(encoder.finish());
        self.read_idx = write_idx;
    }
}

fn attachment(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        ops: wgpu::Operations {
            // Every texel is written unconditionally, so the load is discardable.
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
        depth_slice: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_names_round_trip() {
        for o in Output::ALL {
            assert_eq!(Output::from_name(o.name()), Some(o));
        }
        assert_eq!(Output::from_name("nope"), None);
    }

    #[test]
    fn schema_lists_every_output_with_a_resolvable_format() {
        let schema = schema();
        assert_eq!(schema.textures.len(), Output::ALL.len());
        assert!(schema.scalars.is_empty());
        for def in &schema.textures {
            let format = crate::analyzer::traits::texture_format_from_str(&def.format);
            assert!(format.is_some(), "unresolvable format key {}", def.format);
            let output = Output::from_name(&def.name).expect("schema name is an Output");
            assert_eq!(format, Some(output.wgpu_format()));
        }
    }

    #[test]
    fn normalized_params_round_trip() {
        let mut p = DepthPreprocessParams::default();
        for (name, value) in [
            ("near", 0.1_f32),
            ("far", 0.75),
            ("smoothing", 0.25),
            ("hole_fill", 0.5),
            ("mask_feather", 0.125),
            ("motion_gain", 0.9),
            ("mirror", 0.0),
        ] {
            assert!(p.set_normalized_param(name, value), "{name} not applied");
            let got = p.normalized_param(name).expect("readable");
            assert!((got - value).abs() < 1e-5, "{name}: {got} != {value}");
        }
    }

    #[test]
    fn unknown_param_is_rejected_and_mutates_nothing() {
        let mut p = DepthPreprocessParams::default();
        let before = p;
        assert!(!p.set_normalized_param("orbit_yaw", 1.0));
        assert_eq!(p, before);
        assert_eq!(p.normalized_param("orbit_yaw"), None);
    }

    #[test]
    fn far_always_stays_above_near() {
        let mut p = DepthPreprocessParams::default();
        // Raising `near` past `far` must push `far` up, not invert the range.
        p.set_normalized_param("far", 0.1);
        p.set_normalized_param("near", 0.9);
        assert!(p.far_mm > p.near_mm, "{:?}", p);
        // And lowering `far` below `near` must clamp rather than invert.
        p.set_normalized_param("far", 0.0);
        assert!(p.far_mm > p.near_mm, "{:?}", p);
    }

    #[test]
    fn mirror_is_on_by_default() {
        assert!(DepthPreprocessParams::default().mirror);
    }

    #[test]
    fn pipeline_builds_on_headless() {
        let Ok(gpu) = crate::renderer::GpuContext::new_headless() else {
            return;
        };
        let pipeline = DepthPreprocessPipeline::new(&gpu.device, 64, 48);
        assert_eq!(pipeline.resolution(), (64, 48));
        for o in Output::ALL {
            let (tex, _) = pipeline.output(o).expect("output allocated");
            assert_eq!(tex.format(), o.wgpu_format());
            assert_eq!(tex.width(), 64);
            assert_eq!(tex.height(), 48);
        }
    }

    // ── GPU behaviour ────────────────────────────────────────────────────────
    //
    // These drive the real passes against a synthetic depth image and read the
    // results back. They are the only thing that can catch a wrong sign, a
    // mirrored axis, or a hole-fill that quietly does nothing.

    const W: u32 = 16;
    const H: u32 = 8;

    /// Upload a millimetre depth image to an `R16Uint` texture shaped like the
    /// one `DepthSensorManager` owns.
    fn upload_depth(gpu: &crate::renderer::GpuContext, mm: &[u16]) -> wgpu::TextureView {
        assert_eq!(mm.len() as u32, W * H);
        let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test depth src"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(mm),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 2),
                rows_per_image: Some(H),
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Read back a single-channel `f16` output as `f32`.
    fn read_r16f(gpu: &crate::renderer::GpuContext, texture: &wgpu::Texture) -> Vec<f32> {
        // 256-byte row alignment is a wgpu copy requirement.
        let unpadded = W * 2;
        let padded = unpadded.div_ceil(256) * 256;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded * H),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(5)),
            })
            .expect("poll");
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((W * H) as usize);
        for y in 0..H {
            let row = (y * padded) as usize;
            for x in 0..W {
                let o = row + (x * 2) as usize;
                out.push(f32::from(half::f16::from_le_bytes([data[o], data[o + 1]])));
            }
        }
        drop(data);
        buffer.unmap();
        out
    }

    /// Run the passes once against `mm` and return the `depth` output.
    fn run_once(
        gpu: &crate::renderer::GpuContext,
        params: &DepthPreprocessParams,
        mm: &[u16],
    ) -> (DepthPreprocessPipeline, Vec<f32>) {
        let mut pipeline = DepthPreprocessPipeline::new(&gpu.device, W, H);
        let src = upload_depth(gpu, mm);
        pipeline.update_uniform(&gpu.queue, params, 1.0 / 30.0);
        let mut cmds = Vec::new();
        pipeline.run(&gpu.device, &src, None, &mut cmds);
        gpu.queue.submit(cmds);
        let (depth_tex, _) = pipeline.output(Output::Depth).expect("depth output");
        let depth = read_r16f(gpu, &depth_tex);
        (pipeline, depth)
    }

    fn test_params() -> DepthPreprocessParams {
        DepthPreprocessParams {
            near_mm: 1000.0,
            far_mm: 2000.0,
            smoothing: 0.0,
            hole_fill: 0.0,
            mask_feather: 0.0,
            motion_gain: 1.0,
            mirror: false,
        }
    }

    #[test]
    fn depth_normalizes_within_range_and_invalidates_outside_it() {
        let Ok(gpu) = crate::renderer::GpuContext::new_headless() else {
            return;
        };
        // Column 0 too near, column 1 at the far end, column 2 mid-range,
        // column 3 unresolved (0 mm), the rest too far.
        let mut mm = vec![9000u16; (W * H) as usize];
        for y in 0..H as usize {
            let row = y * W as usize;
            mm[row] = 500; // below near
            mm[row + 1] = 2000; // == far
            mm[row + 2] = 1500; // midpoint
            mm[row + 3] = 0; // sensor could not resolve
        }

        let (_pipe, depth) = run_once(&gpu, &test_params(), &mm);

        assert_eq!(depth[0], 0.0, "below near must be invalid");
        assert!(
            (depth[1] - 1.0).abs() < 1e-2,
            "far end should normalize to 1.0, got {}",
            depth[1]
        );
        assert!(
            (depth[2] - 0.5).abs() < 1e-2,
            "midpoint should normalize to 0.5, got {}",
            depth[2]
        );
        assert_eq!(depth[3], 0.0, "unresolved must be invalid");
        assert_eq!(depth[4], 0.0, "beyond far must be invalid");
    }

    #[test]
    fn hole_fill_closes_a_punched_hole_from_its_nearest_valid_neighbour() {
        let Ok(gpu) = crate::renderer::GpuContext::new_headless() else {
            return;
        };
        let mut mm = vec![1500u16; (W * H) as usize];
        let hole = (3 * W + 5) as usize;
        mm[hole] = 0;

        // Radius 0: the hole stays a hole.
        let mut params = test_params();
        let (_p, without) = run_once(&gpu, &params, &mm);
        assert_eq!(without[hole], 0.0, "radius 0 must not fill");

        // Radius 2: the surrounding surface fills it.
        params.hole_fill = 2.0;
        let (_p, with) = run_once(&gpu, &params, &mm);
        assert!(
            (with[hole] - 0.5).abs() < 1e-2,
            "hole should fill with the neighbouring surface, got {}",
            with[hole]
        );
    }

    #[test]
    fn mirror_flips_the_x_axis() {
        let Ok(gpu) = crate::renderer::GpuContext::new_headless() else {
            return;
        };
        // Only the leftmost column is in range.
        let mut mm = vec![9000u16; (W * H) as usize];
        for y in 0..H as usize {
            mm[y * W as usize] = 1500;
        }

        let mut params = test_params();
        let (_p, unmirrored) = run_once(&gpu, &params, &mm);
        assert!(unmirrored[0] > 0.0 && unmirrored[(W - 1) as usize] == 0.0);

        params.mirror = true;
        let (_p, mirrored) = run_once(&gpu, &params, &mm);
        assert_eq!(mirrored[0], 0.0, "mirrored: left column should be empty");
        assert!(
            mirrored[(W - 1) as usize] > 0.0,
            "mirrored: right column should hold the subject"
        );
    }

    #[test]
    fn motion_is_zero_on_the_first_frame_and_on_a_static_scene() {
        let Ok(gpu) = crate::renderer::GpuContext::new_headless() else {
            return;
        };
        // A ramp, so the depth gradient is non-zero everywhere and motion is
        // only zero because nothing moved — not because the gradient vanished.
        let mm: Vec<u16> = (0..W * H).map(|i| 1000 + (i % W) as u16 * 50).collect();

        let mut pipeline = DepthPreprocessPipeline::new(&gpu.device, W, H);
        let src = upload_depth(&gpu, &mm);
        let params = test_params();

        for _ in 0..3 {
            pipeline.update_uniform(&gpu.queue, &params, 1.0 / 30.0);
            let mut cmds = Vec::new();
            pipeline.run(&gpu.device, &src, None, &mut cmds);
            gpu.queue.submit(cmds);
        }

        let (motion_tex, _) = pipeline.output(Output::Motion).expect("motion output");
        // Rg16Float: read the R channel of each texel via the same row stride
        // logic, doubled for two components.
        let motion = read_rg16f_x(&gpu, &motion_tex);
        for (i, v) in motion.iter().enumerate() {
            assert!(
                v.abs() < 1e-3,
                "static scene produced motion {v} at texel {i}"
            );
        }
    }

    /// Read the R channel of an `Rg16Float` texture.
    fn read_rg16f_x(gpu: &crate::renderer::GpuContext, texture: &wgpu::Texture) -> Vec<f32> {
        let unpadded = W * 4;
        let padded = unpadded.div_ceil(256) * 256;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback rg"),
            size: u64::from(padded * H),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(5)),
            })
            .expect("poll");
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((W * H) as usize);
        for y in 0..H {
            let row = (y * padded) as usize;
            for x in 0..W {
                let o = row + (x * 4) as usize;
                out.push(f32::from(half::f16::from_le_bytes([data[o], data[o + 1]])));
            }
        }
        drop(data);
        buffer.unmap();
        out
    }
}
