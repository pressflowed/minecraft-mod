//! Draws the jets and their weapons: models into a 4x multisampled layer of their own (with its
//! own depth, and the world's depth hiding them behind hills), resolved, then laid over the
//! picture. Only plain wgpu here, so the preview harness can drive it too.

use crate::jet_mesh::{self, VERTEX_FLOATS};
use crate::jet_shader;
use crate::wgpu;

/// The `Frame` uniform: a matrix and fifteen vec4s.
pub const FRAME_SIZE: usize = 64 + 16 * 15;
/// The `Object` uniform, and its slot in the buffer (dynamic offsets go in 256s).
pub const OBJECT_SIZE: usize = 64 + 128 + 32;
const OBJECT_STRIDE: u64 = 256;
const MAX_DRAWS: usize = 48;
const SAMPLES: u32 = 4;
const LAYER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The models: the five jets, then a missile and a bomb.
pub const MISSILE: usize = jet_sim_count();
pub const BOMB: usize = MISSILE + 1;

const fn jet_sim_count() -> usize {
    crate::jet_sim::COUNT
}

/// One model to draw.
#[derive(Clone, Debug)]
pub struct Draw {
    pub mesh: usize,
    /// Body to camera-relative world, column-major.
    pub model: [f32; 16],
    pub parts: [f32; 32],
    /// The nozzles' glow (0 to about 2).
    pub glow: f32,
    pub hide_pilot: bool,
    pub world_test: bool,
    /// The cockpit's opening in the hull, cut away (see `jet_mesh::cockpit_cut`); zero for none.
    pub cut: [f32; 4],
}

struct Model {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

struct Targets {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::TextureView,
}

pub struct JetGpu {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    solid: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    frame: wgpu::Buffer,
    objects: wgpu::Buffer,
    models: Vec<Option<Model>>,
    targets: Option<Targets>,
}

impl JetGpu {
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("superpowers jets"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: both,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: both,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(OBJECT_SIZE as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("superpowers jets"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("superpowers jets"),
            source: wgpu::ShaderSource::Wgsl(jet_shader::WGSL.into()),
        });
        let f = 4u64;
        let attributes = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 3 * f, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 6 * f, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 9 * f, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 10 * f, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 11 * f, shader_location: 5 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 14 * f, shader_location: 6 },
        ];
        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = |write: bool, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("superpowers jets"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_jet"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: (VERTEX_FLOATS * 4) as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    // reversed depth: nearer is larger
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_jet"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: LAYER_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let solid = pipeline(true, None);
        let glass = pipeline(false, Some(premultiplied));
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("superpowers jets composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let composite_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("superpowers jets composite"),
            source: wgpu::ShaderSource::Wgsl(jet_shader::COMPOSITE.into()),
        });
        let composite_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("superpowers jets composite"),
            bind_group_layouts: &[Some(&composite_layout)],
            ..Default::default()
        });
        let over = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("superpowers jets composite"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState { module: &composite_module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &composite_module,
                entry_point: Some("fs_composite"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(over), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("superpowers jets"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let frame = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers jets frame"),
            size: FRAME_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let objects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers jets objects"),
            size: OBJECT_STRIDE * (MAX_DRAWS as u64 * 2),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            format,
            layout,
            solid,
            glass,
            composite_layout,
            composite,
            sampler,
            frame,
            objects,
            models: (0..BOMB + 1).map(|_| None).collect(),
            targets: None,
        }
    }

    fn model(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, id: usize) {
        if id >= self.models.len() || self.models[id].is_some() {
            return;
        }
        let mesh = match id {
            MISSILE => jet_mesh::ordnance(true),
            BOMB => jet_mesh::ordnance(false),
            kind => jet_mesh::jet(kind),
        };
        let floats = mesh.floats();
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers jet vertices"),
            size: (floats.len() * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        queue.write_buffer(&vertices, 0, &bytes);
        let indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers jet indices"),
            size: (mesh.indices.len() * 4) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bytes: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        queue.write_buffer(&indices, 0, &bytes);
        self.models[id] = Some(Model { vertices, indices, count: mesh.indices.len() as u32 });
    }

    fn targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.targets.as_ref().map(|t| t.size) == Some(size) {
            return;
        }
        let texture = |format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("superpowers jets layer"),
                    size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        self.targets = Some(Targets {
            size,
            color: texture(LAYER_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT),
            depth: texture(wgpu::TextureFormat::Depth32Float, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT),
            resolved: texture(LAYER_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING),
        });
    }

    /// Draws `draws` over the picture in `color_view`, hidden where `world_depth` is nearer.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        world_depth: &wgpu::TextureView,
        size: [u32; 2],
        frame: &[u8],
        draws: &[Draw],
    ) {
        if draws.is_empty() || size[0] == 0 || size[1] == 0 {
            return;
        }
        let draws = &draws[..draws.len().min(MAX_DRAWS)];
        for d in draws {
            self.model(device, queue, d.mesh);
        }
        self.targets(device, size);
        queue.write_buffer(&self.frame, 0, frame);
        // each draw twice: its solid parts, then its glass
        let mut objects = vec![0u8; OBJECT_STRIDE as usize * draws.len() * 2];
        for (i, d) in draws.iter().enumerate() {
            for glass in [false, true] {
                let slot = (i * 2 + usize::from(glass)) * OBJECT_STRIDE as usize;
                let mut put = |at: usize, values: &[f32]| {
                    for (k, v) in values.iter().enumerate() {
                        objects[slot + at + k * 4..slot + at + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
                    }
                };
                put(0, &d.model);
                put(64, &d.parts);
                put(192, &[d.glow, f32::from(u8::from(d.hide_pilot)), f32::from(u8::from(d.world_test)), f32::from(u8::from(glass))]);
                put(208, &d.cut);
            }
        }
        queue.write_buffer(&self.objects, 0, &objects);
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("superpowers jets"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.frame.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.objects, offset: 0, size: wgpu::BufferSize::new(OBJECT_SIZE as u64) }),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(world_depth) },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("superpowers jets"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.color,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolved),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            for glass in [false, true] {
                pass.set_pipeline(if glass { &self.glass } else { &self.solid });
                for (i, d) in draws.iter().enumerate() {
                    let Some(Some(model)) = self.models.get(d.mesh) else {
                        continue;
                    };
                    let offset = ((i * 2 + usize::from(glass)) as u64 * OBJECT_STRIDE) as u32;
                    pass.set_bind_group(0, &group, &[offset]);
                    pass.set_vertex_buffer(0, model.vertices.slice(..));
                    pass.set_index_buffer(model.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..model.count, 0, 0..1);
                }
            }
        }
        let composite_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("superpowers jets composite"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&targets.resolved) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("superpowers jets composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &composite_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A column-major matrix from the jet's axes (left, up, forward) and where it is, camera-relative.
pub fn model_matrix(left: [f64; 3], up: [f64; 3], fwd: [f64; 3], at: [f64; 3]) -> [f32; 16] {
    [
        left[0] as f32, left[1] as f32, left[2] as f32, 0.0,
        up[0] as f32, up[1] as f32, up[2] as f32, 0.0,
        fwd[0] as f32, fwd[1] as f32, fwd[2] as f32, 0.0,
        at[0] as f32, at[1] as f32, at[2] as f32, 1.0,
    ]
}
