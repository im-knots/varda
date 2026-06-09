use anyhow::Result;
use wgpu::util::DeviceExt;

use super::ISFUniforms;
use crate::isf::StorageBufferDecl;

/// A storage buffer used by compute shaders
pub struct StorageBuffer {
    pub name: String,
    pub buffer: wgpu::Buffer,
    pub persistent: bool,
    pub count: u32,
    pub stride: u32,
}

/// How the compute shader should be dispatched
pub enum DispatchMode {
    /// Dispatch based on output resolution divided by workgroup size
    Resolution,
    /// Dispatch with custom parameters
    Custom {
        x_param: String,
        y_param: String,
        z_param: Option<String>,
    },
}

/// Compute pipeline for GLSL compute shaders compiled to SPIR-V
pub struct ComputePipeline {
    pub compute_pipeline: wgpu::ComputePipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
    pub output_texture: wgpu::Texture,
    pub output_view: wgpu::TextureView,
    pub storage_buffers: Vec<StorageBuffer>,
    pub default_user_params_buffer: wgpu::Buffer,
    pub user_params_binding: u32,
    pub workgroup_size: [u32; 3],
    pub dispatch_mode: DispatchMode,
    pub num_passes: u32,
}

impl ComputePipeline {
    /// Create a compute pipeline from SPIR-V bytecode.
    pub fn new(
        device: &wgpu::Device,
        spirv: &[u32],
        width: u32,
        height: u32,
        buffer_decls: &[StorageBufferDecl],
        workgroup_size: [u32; 3],
        dispatch_mode: DispatchMode,
        num_passes: u32,
    ) -> Result<Self> {
        // Convert SPIR-V to WGSL using naga
        let spirv_bytes: Vec<u8> = spirv.iter().flat_map(|word| word.to_le_bytes()).collect();

        let module =
            naga::front::spv::parse_u8_slice(&spirv_bytes, &naga::front::spv::Options::default())?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)?;

        let wgsl =
            naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())?;

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ISF Compute Shader Module"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });

        // Create uniform buffer
        let uniforms = ISFUniforms::default();
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ISF Compute Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Build bind group layout entries
        let mut layout_entries = vec![];
        let mut next_binding: u32 = 0;

        // Binding 0: ISFUniforms
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: next_binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(
                    std::mem::size_of::<ISFUniforms>() as u64
                ),
            },
            count: None,
        });
        next_binding += 1;

        // Binding 1: UserParams
        let user_params_binding = next_binding;
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: user_params_binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        next_binding += 1;

        // Binding 2: Output storage texture
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: next_binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba8Unorm,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        });
        next_binding += 1;

        // Binding 3+: Storage buffers
        for decl in buffer_decls {
            let read_only = decl.buffer_type == "read-only-storage";
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
            next_binding += 1;
        }

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ISF Compute Bind Group Layout"),
            entries: &layout_entries,
        });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ISF Compute Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // Create compute pipeline
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Create output texture
        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Compute Output Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Default user params buffer
        let default_user_params = [0u8; 256];
        let default_user_params_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Default Compute User Params Buffer"),
                contents: &default_user_params,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        // Create storage buffers
        let storage_buffers: Vec<StorageBuffer> = buffer_decls
            .iter()
            .map(|decl| {
                let size = (decl.count * decl.stride) as u64;
                let data = vec![0u8; size as usize];
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("Storage Buffer: {}", decl.name)),
                    contents: &data,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                });
                StorageBuffer {
                    name: decl.name.clone(),
                    buffer,
                    persistent: decl.persistent,
                    count: decl.count,
                    stride: decl.stride,
                }
            })
            .collect();

        Ok(Self {
            compute_pipeline,
            bind_group_layout,
            uniform_buffer,
            output_texture,
            output_view,
            storage_buffers,
            default_user_params_buffer,
            user_params_binding,
            workgroup_size,
            dispatch_mode,
            num_passes,
        })
    }

    /// Create a bind group for compute dispatch.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        user_params_buffer: Option<&wgpu::Buffer>,
    ) -> wgpu::BindGroup {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: user_params_buffer
                    .unwrap_or(&self.default_user_params_buffer)
                    .as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&self.output_view),
            },
        ];

        for (i, sb) in self.storage_buffers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: 3 + i as u32,
                resource: sb.buffer.as_entire_binding(),
            });
        }

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ISF Compute Bind Group"),
            layout: &self.bind_group_layout,
            entries: &entries,
        })
    }

    /// Update uniforms
    pub fn update_uniforms(&self, queue: &wgpu::Queue, uniforms: &ISFUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[*uniforms]));
    }

    /// Get the output texture view
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.output_view
    }

    /// Calculate dispatch group counts based on resolution and workgroup size.
    pub fn dispatch_counts(&self, width: u32, height: u32) -> (u32, u32, u32) {
        match &self.dispatch_mode {
            DispatchMode::Resolution => {
                let x = width.div_ceil(self.workgroup_size[0]);
                let y = height.div_ceil(self.workgroup_size[1]);
                (x, y, 1)
            }
            DispatchMode::Custom { .. } => {
                // TODO: resolve custom dispatch parameters from user params
                (1, 1, 1)
            }
        }
    }

    /// Zero-fill all non-persistent storage buffers.
    /// Called before pass 0 each frame so shaders can accumulate into clean buffers.
    pub fn clear_non_persistent_buffers(&self, encoder: &mut wgpu::CommandEncoder) {
        for sb in &self.storage_buffers {
            if !sb.persistent {
                encoder.clear_buffer(&sb.buffer, 0, None);
            }
        }
    }
}
