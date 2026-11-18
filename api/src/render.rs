//! Two GPU passes once the world is drawn, before the hand and the HUD:
//!
//! 1. The lens pass, only while something bends light: it copies the picture and draws it back
//!    bent around black holes (with the event horizon black), rippled by shock waves and
//!    frosted at the edges during a Frost Nova.
//! 2. The light pass, every night and while any power shows: aurora, shock rings and bubbles,
//!    laser beams, lightning, frost sparkle, the accretion disk and the dash's trail, added on top.

use crate::client::{self, Kind};
use crate::hud::beam_color;
use crate::shader;
use horizon_client::mod_api::{RenderHooks, RenderNeedsEvent, WorldDrawnEvent, wgpu};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Effects the shader takes at once.
pub const MAX_EFFECTS: usize = 48;
/// The `Frame` uniform: two matrices, three vec4s, then the effects of four vec4s each.
pub const UNIFORM_SIZE: usize = 64 * 2 + 16 * 3 + MAX_EFFECTS * 64;

pub struct Renderer {
    started: Instant,
    gpu: Mutex<Option<Gpu>>,
}

struct Gpu {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    glow: wgpu::RenderPipeline,
    lens: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// The copy of the picture the lens pass reads, and its size.
    scene: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
}

impl Renderer {
    pub fn new() -> Self {
        Self { started: Instant::now(), gpu: Mutex::new(None) }
    }
}

impl Gpu {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
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
                // pass that reads a texture_depth_2d (as Orbital Laser's post pass binds it).
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
            source: wgpu::ShaderSource::Wgsl(shader::WGSL.into()),
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
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // Light adds to the picture; the picture's alpha stays as it is.
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
            size: UNIFORM_SIZE as u64,
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
        Self { format, layout, glow, lens, uniform, sampler, scene: None }
    }

    /// The copy of the picture, made for this size when it changes.
    fn scene(&mut self, device: &wgpu::Device, size: [u32; 2]) -> &(wgpu::Texture, wgpu::TextureView, [u32; 2]) {
        if self.scene.as_ref().map(|s| s.2) != Some(size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("superpowers scene"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.scene = Some((texture, view, size));
        }
        self.scene.as_ref().expect("made above")
    }
}

/// `m` (column-major) times `v`.
fn mul(m: &[f32; 16], v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|row| (0..4).map(|col| m[col * 4 + row] * v[col]).sum())
}

/// The camera-relative point a screen position (NDC) shows on the near plane.
fn unproject(inv: &[f32; 16], x: f32, y: f32) -> [f32; 3] {
    let p = mul(inv, [x, y, 1.0, 1.0]);
    [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn rgb(color: u32) -> [f32; 3] {
    let c = |shift: u32| ((color >> shift) & 0xFF) as f32 / 255.0;
    // sRGB to linear light, near enough
    [c(16).powf(2.2), c(8).powf(2.2), c(0).powf(2.2)]
}

/// What the frame's uniform holds, before it becomes bytes.
pub struct Packed {
    pub bytes: Vec<u8>,
    /// Something bends light: the lens pass runs.
    pub lens: bool,
    /// Anything to light: the light pass runs.
    pub glow: bool,
}

/// The `Frame` uniform for this frame. `camera` is the camera's world position, `view_proj`
/// camera-relative world to clip space.
pub fn pack(seconds: f32, size: [u32; 2], depth_size: [u32; 2], camera: [f64; 3], view_proj: [f32; 16], inv: [f32; 16]) -> Packed {
    let state = client::state();
    let night = if state.dimension.ends_with("overworld") {
        night(client::DAY_TIME.load(Ordering::Relaxed))
    } else {
        0.0
    };
    let now = Instant::now();
    let rel = |p: [f64; 3]| -> [f32; 3] {
        [(p[0] - camera[0]) as f32, (p[1] - camera[1]) as f32, (p[2] - camera[2]) as f32]
    };
    let mut effects: Vec<[[f32; 4]; 4]> = Vec::with_capacity(MAX_EFFECTS);
    let mut flash = 0.0f32;
    let mut frost = 0.0f32;
    let mut lens = false;
    // speed lines: flying fast, or the Omega Beam
    let mut speed = state.flying.map_or(0.0, |f| if f.boost { (f.speed as f32 / 3.0).min(1.0) } else { 0.0 });
    if state.omega.is_some() {
        speed = speed.max(0.4);
    }
    if state.charge > 0 {
        speed = speed.max(0.6 * state.charge as f32 / client::CHARGE_TICKS as f32);
    }
    // The laser eyes first: two beams from just under and in front of the eyes.
    let beams: Vec<client::Beam> = state.beam.filter(|_| state.lasers).into_iter().chain(state.omega).collect();
    for beam in beams {
        let forward = normalize(unproject(&inv, 0.0, 0.0));
        let right = normalize({
            let (a, b) = (unproject(&inv, 1.0, 0.0), unproject(&inv, -1.0, 0.0));
            [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
        });
        let up = normalize({
            let (a, b) = (unproject(&inv, 0.0, 1.0), unproject(&inv, 0.0, -1.0));
            [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
        });
        let eye_base = rel(beam.from);
        let to = rel(beam.to);
        // the Omega Beam (power 12) is one beam from under the eyes, in hot pink
        let omega = beam.power > 5;
        let color = if omega { [1.0, 0.12, 0.35] } else { rgb(beam_color(beam.power)) };
        let sides: &[(f32, f32)] = if omega { &[(0.0, 1.0)] } else { &[(-1.0, 1.0), (1.0, 0.0)] };
        for &(side, impact) in sides {
            let from: [f32; 3] = std::array::from_fn(|i| {
                eye_base[i] + forward[i] * 0.35 - up[i] * if omega { 0.4 } else { 0.16 } + right[i] * 0.09 * side
            });
            effects.push([
                [Kind::Laser as i32 as f32, seconds, 1.0, side],
                [from[0], from[1], from[2], f32::from(beam.power)],
                [to[0], to[1], to[2], if beam.hit { impact } else { 0.0 }],
                [color[0], color[1], color[2], 1.0],
            ]);
        }
    }
    for e in state.effects.iter().rev() {
        let age = e.age(now);
        if age < 0.0 || age > e.duration || effects.len() >= MAX_EFFECTS {
            continue;
        }
        let t = age / e.duration;
        let at = rel(e.at);
        let to = match e.kind {
            Kind::Dash => rel(e.to),
            _ => [e.to[0] as f32, e.to[1] as f32, e.to[2] as f32],
        };
        match e.kind {
            Kind::Punch => flash = flash.max(0.22 * (1.0 - (age / 0.25).min(1.0)).powi(3)),
            Kind::Lightning => flash = flash.max(0.12 * (1.0 - (age / 0.15).min(1.0)).powi(2)),
            Kind::Orb if e.size > 50.0 => flash = flash.max(0.85 * (1.0 - (age / 1.5).min(1.0)).powi(2)),
            Kind::Frost => {
                frost = frost.max((age / 0.3).min(1.0) * (1.0 - t).powf(0.7));
                lens = true;
            }
            Kind::Gravity | Kind::Shockwave => lens = true,
            _ => {}
        }
        effects.push([
            [e.kind as i32 as f32, age, e.duration, e.seed],
            [at[0], at[1], at[2], e.size],
            [to[0], to[1], to[2], 0.0],
            [e.color[0], e.color[1], e.color[2], 1.0],
        ]);
    }
    drop(state);

    let mut bytes = Vec::with_capacity(UNIFORM_SIZE);
    let mut put = |values: &[f32]| {
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    };
    put(&inv);
    put(&view_proj);
    // (small enough that world-locked patterns keep their detail in 32-bit floats)
    let wrap = |v: f64| v.rem_euclid(2048.0) as f32;
    put(&[seconds, night, size[0] as f32, size[1] as f32]);
    put(&[depth_size[0] as f32, depth_size[1] as f32, wrap(camera[0]), wrap(camera[2])]);
    put(&[effects.len() as f32, flash, frost, speed]);
    for effect in &effects {
        for row in effect {
            put(row);
        }
    }
    bytes.resize(UNIFORM_SIZE, 0);
    Packed { bytes, lens, glow: night > 0.001 || !effects.is_empty() }
}

/// How far into the night `day_time` is: 0 by day, 1 in the middle of the night, easing in at
/// dusk (from 12,500 ticks) and out at dawn (to 23,500).
pub fn night(day_time: i64) -> f32 {
    let t = day_time.rem_euclid(24_000) as f32;
    smoothstep(12_500.0, 13_800.0, t) * (1.0 - smoothstep(22_200.0, 23_500.0, t))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl RenderHooks for Renderer {
    /// The passes read the world's depth: what is sky, and where surfaces are.
    fn render_needs(&self, event: &mut RenderNeedsEvent<'_>) {
        event.needs.world_depth = true;
    }

    fn world_drawn(&self, event: &mut WorldDrawnEvent) {
        let world = &mut *event.world;
        if world.size[0] == 0 || world.size[1] == 0 || world.depth_size[0] == 0 || world.depth_size[1] == 0 {
            return;
        }
        let inv = world.view_proj.inverse().to_cols_array();
        let view_proj = world.view_proj.to_cols_array();
        let camera = [world.camera.x, world.camera.y, world.camera.z];
        let seconds = (self.started.elapsed().as_secs_f64() % 3600.0) as f32;
        let packed = pack(seconds, world.size, world.depth_size, camera, view_proj, inv);
        if !packed.glow && !packed.lens {
            return;
        }
        let mut gpu = self.gpu.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if gpu.as_ref().map(|g| g.format) != Some(world.color_format) {
            *gpu = Some(Gpu::new(world.device, world.color_format));
        }
        let Some(gpu) = gpu.as_mut() else {
            return;
        };
        world.queue.write_buffer(&gpu.uniform, 0, &packed.bytes);
        let size = world.size;
        gpu.scene(world.device, size);
        let (scene_texture, scene_view, _) = gpu.scene.as_ref().expect("made above");
        if packed.lens {
            world.encoder.copy_texture_to_texture(
                world.color.as_image_copy(),
                scene_texture.as_image_copy(),
                wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            );
        }
        let group = world.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("superpowers"),
            layout: &gpu.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: gpu.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(world.depth) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(scene_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&gpu.sampler) },
            ],
        });
        let passes = [(packed.lens, &gpu.lens), (packed.glow, &gpu.glow)];
        for (run, pipeline) in passes {
            if !run {
                continue;
            }
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
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_is_dark_free_and_midnight_is_full() {
        assert_eq!(night(6_000), 0.0);
        assert_eq!(night(18_000), 1.0);
        assert_eq!(night(18_000 + 24_000 * 5), 1.0);
        assert_eq!(night(-6_000), 1.0);
        let dusk = night(13_100);
        assert!(dusk > 0.0 && dusk < 1.0);
    }

    #[test]
    fn the_uniform_has_the_shaders_size() {
        let identity: [f32; 16] = std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
        let packed = pack(1.0, [8, 8], [8, 8], [0.0; 3], identity, identity);
        assert_eq!(packed.bytes.len(), UNIFORM_SIZE);
        assert_eq!(UNIFORM_SIZE, 3248);
    }
}
