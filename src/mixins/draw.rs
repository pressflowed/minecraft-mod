fn superpowers_draw(world: &mut WorldDrawn<'_>) {
    // Once the world is drawn: the Superpowers effects and the jets, on the GPU.
    //
    // 1. The lens pass, only while something bends light: the picture is copied and drawn back
    //    bent around black holes, rippled by shock waves, frosted at the edges.
    // 2. The jets and their weapons: models into a 4x multisampled layer of their own (with its
    //    own depth, the world's depth hiding them behind hills), resolved, laid over the picture.
    // 3. The light pass, every night and while any power shows: aurora, shock rings, beams,
    //    lightning, storms, flames, added on top.
    // 4. Past the 10 degrees of roll the game lets a camera shake give, the picture of the world
    //    is turned the rest of the way (Superflight's rolls, a jet's cockpit), enlarged to keep
    //    its corners filled; the view was widened to match.
    //
    // What to draw comes from the mod (`mod_superpowers::gfx`) as plain data; the GPU objects
    // live here and are kept between frames in the mod's `gpu_state`, as `Any`. (A patched
    // function may not declare a `static` or an `impl`, so these are plain structs and functions.)
    struct Effects {
        format: wgpu::TextureFormat,
        layout: wgpu::BindGroupLayout,
        glow: wgpu::RenderPipeline,
        lens: wgpu::RenderPipeline,
        uniform: wgpu::Buffer,
        sampler: wgpu::Sampler,
        // the copy of the picture the lens pass reads, and its size
        scene: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
        roll_layout: wgpu::BindGroupLayout,
        roll: wgpu::RenderPipeline,
        roll_uniform: wgpu::Buffer,
    }

    struct Model {
        vertices: wgpu::Buffer,
        indices: wgpu::Buffer,
        count: u32,
    }

    struct Layer {
        size: [u32; 2],
        color: wgpu::TextureView,
        depth: wgpu::TextureView,
        resolved: wgpu::TextureView,
    }

    struct Jets {
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
        layer: Option<Layer>,
    }

    struct Gpu {
        effects: Option<Effects>,
        jets: Option<Jets>,
    }

    const SAMPLES: u32 = 4;
    const LAYER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

    fn premultiplied_over(alpha_kept: bool) -> wgpu::BlendState {
        let color = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let alpha = if alpha_kept {
            wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            }
        } else {
            color
        };
        wgpu::BlendState { color, alpha }
    }

    fn make_effects(device: &wgpu::Device, format: wgpu::TextureFormat) -> Effects {
        let fragment = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("superpowers"),
            entries: &[
                fragment(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                // The Depth32Float depth as an unfilterable float texture: OpenGL can't build a
                // pass that reads a texture_depth_2d.
                fragment(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                fragment(
                    2,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                fragment(3, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("superpowers"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("superpowers"),
            source: wgpu::ShaderSource::Wgsl(mod_superpowers::gfx::EFFECTS_WGSL.into()),
        });
        let pipeline = |entry: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("superpowers"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // light adds to the picture; the picture's alpha stays as it is
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let glow = pipeline("fs_glow", Some(additive));
        let lens = pipeline("fs_lens", None);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers frame"),
            size: mod_superpowers::gfx::EFFECTS_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("superpowers scene"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let roll_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("superpowers roll"),
            entries: &[
                fragment(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                fragment(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                fragment(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let roll_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("superpowers roll"),
            bind_group_layouts: &[Some(&roll_layout)],
            ..Default::default()
        });
        let roll_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("superpowers roll"),
            source: wgpu::ShaderSource::Wgsl(mod_superpowers::gfx::ROLL_WGSL.into()),
        });
        let roll = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("superpowers roll"),
            layout: Some(&roll_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &roll_module,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &roll_module,
                entry_point: Some("fs_roll"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let roll_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers roll"),
            size: mod_superpowers::gfx::ROLL_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Effects { format, layout, glow, lens, uniform, sampler, scene: None, roll_layout, roll, roll_uniform }
    }

    fn make_jets(device: &wgpu::Device, format: wgpu::TextureFormat) -> Jets {
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
                        min_binding_size: wgpu::BufferSize::new(mod_superpowers::gfx::OBJECT_SIZE as u64),
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
            source: wgpu::ShaderSource::Wgsl(mod_superpowers::gfx::JET_WGSL.into()),
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
        let pipeline = |write: bool, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("superpowers jets"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_jet"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: (mod_superpowers::gfx::VERTEX_FLOATS * 4) as u64,
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
        let glass = pipeline(false, Some(premultiplied_over(false)));
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
            source: wgpu::ShaderSource::Wgsl(mod_superpowers::gfx::COMPOSITE_WGSL.into()),
        });
        let composite_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("superpowers jets composite"),
            bind_group_layouts: &[Some(&composite_layout)],
            ..Default::default()
        });
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
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(premultiplied_over(true)), write_mask: wgpu::ColorWrites::ALL })],
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
            size: mod_superpowers::gfx::JET_FRAME_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let objects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers jets objects"),
            size: mod_superpowers::gfx::OBJECT_STRIDE * (mod_superpowers::gfx::MAX_DRAWS as u64 * 2),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Jets {
            format,
            layout,
            solid,
            glass,
            composite_layout,
            composite,
            sampler,
            frame,
            objects,
            models: (0..mod_superpowers::gfx::MODELS).map(|_| None).collect(),
            layer: None,
        }
    }

    fn upload_model(jets: &mut Jets, device: &wgpu::Device, queue: &wgpu::Queue, id: usize) {
        if id >= jets.models.len() || jets.models[id].is_some() {
            return;
        }
        let (floats, indices) = mod_superpowers::gfx::mesh(id);
        if floats.is_empty() || indices.is_empty() {
            return;
        }
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers jet vertices"),
            size: (floats.len() * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        queue.write_buffer(&vertices, 0, &bytes);
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("superpowers jet indices"),
            size: (indices.len() * 4) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        queue.write_buffer(&index_buffer, 0, &bytes);
        jets.models[id] = Some(Model { vertices, indices: index_buffer, count: indices.len() as u32 });
    }

    fn make_layer(device: &wgpu::Device, size: [u32; 2]) -> Layer {
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
        Layer {
            size,
            color: texture(LAYER_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT),
            depth: texture(wgpu::TextureFormat::Depth32Float, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT),
            resolved: texture(LAYER_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING),
        }
    }

    // Draws the models over the picture, hidden where the world's depth is nearer.
    fn draw_jets(jets: &mut Jets, world: &mut WorldDrawn<'_>, frame: &[u8], draws: &[mod_superpowers::gfx::Draw]) {
        let size = world.size;
        if draws.is_empty() || frame.len() != mod_superpowers::gfx::JET_FRAME_SIZE {
            return;
        }
        let draws = &draws[..draws.len().min(mod_superpowers::gfx::MAX_DRAWS)];
        for d in draws {
            upload_model(jets, world.device, world.queue, d.mesh);
        }
        if jets.layer.as_ref().map(|l| l.size) != Some(size) {
            jets.layer = Some(make_layer(world.device, size));
        }
        world.queue.write_buffer(&jets.frame, 0, frame);
        // each draw twice: its solid parts, then its glass
        let stride = mod_superpowers::gfx::OBJECT_STRIDE as usize;
        let mut objects = Vec::new();
        objects.resize(stride * draws.len() * 2, 0u8);
        for (i, d) in draws.iter().enumerate() {
            for glass in [false, true] {
                let slot = (i * 2 + usize::from(glass)) * stride;
                objects[slot..slot + mod_superpowers::gfx::OBJECT_SIZE].copy_from_slice(&d.object(glass));
            }
        }
        world.queue.write_buffer(&jets.objects, 0, &objects);
        let Some(layer) = jets.layer.as_ref() else {
            return;
        };
        let group = world.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("superpowers jets"),
            layout: &jets.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: jets.frame.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &jets.objects,
                        offset: 0,
                        size: wgpu::BufferSize::new(mod_superpowers::gfx::OBJECT_SIZE as u64),
                    }),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(world.depth) },
            ],
        });
        {
            let mut pass = world.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("superpowers jets"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &layer.color,
                    depth_slice: None,
                    resolve_target: Some(&layer.resolved),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &layer.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            for glass in [false, true] {
                pass.set_pipeline(if glass { &jets.glass } else { &jets.solid });
                for (i, d) in draws.iter().enumerate() {
                    let Some(Some(model)) = jets.models.get(d.mesh) else {
                        continue;
                    };
                    let offset = ((i * 2 + usize::from(glass)) * stride) as u32;
                    pass.set_bind_group(0, &group, &[offset]);
                    pass.set_vertex_buffer(0, model.vertices.slice(..));
                    pass.set_index_buffer(model.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..model.count, 0, 0..1);
                }
            }
        }
        let composite_group = world.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("superpowers jets composite"),
            layout: &jets.composite_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&layer.resolved) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&jets.sampler) },
            ],
        });
        let mut pass = world.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("superpowers jets composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: world.color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&jets.composite);
        pass.set_bind_group(0, &composite_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // One full-screen pass of the effects shader.
    fn effects_pass(world: &mut WorldDrawn<'_>, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup) {
        let mut pass = world.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("superpowers"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: world.color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    let size = world.size;
    let camera = [world.camera.x, world.camera.y, world.camera.z];
    let view_proj = world.view_proj.to_cols_array();
    let inverse = world.view_proj.inverse().to_cols_array();
    let Some(frame) = mod_superpowers::gfx::frame(view_proj, inverse, size, world.depth_size, camera) else {
        return;
    };
    if frame.effects.len() != mod_superpowers::gfx::EFFECTS_SIZE {
        return;
    }
    let mut kept = mod_superpowers::gfx::gpu_state();
    if !kept.as_ref().is_some_and(|k| k.is::<Gpu>()) {
        *kept = Some(Box::new(Gpu { effects: None, jets: None }));
    }
    let Some(gpu) = kept.as_mut().and_then(|k| k.downcast_mut::<Gpu>()) else {
        return;
    };
    if gpu.effects.as_ref().map(|e| e.format) != Some(world.color_format) {
        gpu.effects = Some(make_effects(world.device, world.color_format));
    }
    if frame.jets.is_some() && gpu.jets.as_ref().map(|j| j.format) != Some(world.color_format) {
        gpu.jets = Some(make_jets(world.device, world.color_format));
    }
    let Some(effects) = gpu.effects.as_mut() else {
        return;
    };
    world.queue.write_buffer(&effects.uniform, 0, &frame.effects);
    if effects.scene.as_ref().map(|s| s.2) != Some(size) {
        let texture = world.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("superpowers scene"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: effects.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        effects.scene = Some((texture, view, size));
    }
    let Some((scene_texture, scene_view, _)) = effects.scene.as_ref() else {
        return;
    };
    if frame.lens {
        world.encoder.copy_texture_to_texture(
            world.color.as_image_copy(),
            scene_texture.as_image_copy(),
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
    }
    let group = world.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("superpowers"),
        layout: &effects.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: effects.uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(world.depth) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(scene_view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&effects.sampler) },
        ],
    });
    // the lens pass, then the jets over it (so storms behind them don't cover them), then light
    if frame.lens {
        effects_pass(world, &effects.lens, &group);
    }
    if let (Some((jet_frame, draws)), Some(jets)) = (frame.jets.as_ref(), gpu.jets.as_mut()) {
        draw_jets(jets, world, jet_frame, draws);
    }
    let Some(effects) = gpu.effects.as_ref() else {
        return;
    };
    if frame.glow {
        effects_pass(world, &effects.glow, &group);
    }
    if let (Some(roll), Some((scene_texture, scene_view, _))) = (frame.roll.as_ref(), effects.scene.as_ref()) {
        world.queue.write_buffer(&effects.roll_uniform, 0, roll);
        world.encoder.copy_texture_to_texture(
            world.color.as_image_copy(),
            scene_texture.as_image_copy(),
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        let roll_group = world.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("superpowers roll"),
            layout: &effects.roll_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: effects.roll_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(scene_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&effects.sampler) },
            ],
        });
        effects_pass(world, &effects.roll, &roll_group);
    }
}
