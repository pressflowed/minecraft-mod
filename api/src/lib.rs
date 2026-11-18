//! Aurora Skies: night skies with glowing aurora curtains, shooting stars and twinkling stars.
//!
//! A trait mod (Horizon's docs/native-mods.md, "Trait mods"): the launcher links this crate into
//! the game and calls [`hooks`]. Two things happen:
//!
//! - [`Hooks::tick`] reads the time of day and the dimension from the window's game, so the
//!   render thread knows how dark it is.
//! - [`RenderHooks::world_drawn`] runs once the world is drawn (before the hand and the HUD) and
//!   draws one full-screen pass ([`shader::SKY_WGSL`]) additively over the open sky.
//!
//! Only the catalog's hook points (`tick`, `render_needs`, `world_drawn`) and `Host` helpers
//! (`camera`, `day_time`) are used.

mod shader;

use horizon_client::mod_api::{
    Hooks, Host, RenderHooks, RenderNeedsEvent, TickEvent, WorldDrawnEvent, wgpu,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Instant;

/// The launcher calls this to get the mod.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(AuroraSkies {
        started: Instant::now(),
        gpu: Mutex::new(None),
    })
}

/// The time of day in ticks, from the last tick of the window's game.
static DAY_TIME: AtomicI64 = AtomicI64::new(6000);
/// Whether the camera is in the Overworld: the Nether and the End have no night sky.
static OVERWORLD: AtomicBool = AtomicBool::new(false);

struct AuroraSkies {
    started: Instant,
    /// The sky pass, built on the first night frame for the colour target's format.
    gpu: Mutex<Option<SkyPass>>,
}

/// The bytes of the shader's `Frame` uniform: a mat4x4 and two vec4s.
const UNIFORM_SIZE: u64 = 96;

struct SkyPass {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
}

impl SkyPass {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("aurora_skies sky"),
            source: wgpu::ShaderSource::Wgsl(shader::SKY_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aurora_skies sky"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The Depth32Float depth as an unfilterable float texture: OpenGL can't build a
                // pass that reads a texture_depth_2d (as Orbital Laser's post pass binds it).
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
            label: Some("aurora_skies sky"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        // Light adds to the picture; the picture's alpha stays as it is.
        let blend = wgpu::BlendState {
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("aurora_skies sky"),
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
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("aurora_skies frame"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            format,
            pipeline,
            layout,
            uniform,
        }
    }
}

/// How far into the night `day_time` is: 0 by day, 1 in the middle of the night, easing in at
/// dusk (from 12,500 ticks) and out at dawn (to 23,500).
fn night(day_time: i64) -> f32 {
    let t = day_time.rem_euclid(24_000) as f32;
    smoothstep(12_500.0, 13_800.0, t) * (1.0 - smoothstep(22_200.0, 23_500.0, t))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Hooks for AuroraSkies {
    fn render(&self) -> Option<&dyn RenderHooks> {
        Some(self)
    }

    /// Every tick, in every `Game`: only the window's has a camera, and that is the one the
    /// player looks through.
    fn tick(&self, host: &mut dyn Host, _event: &mut TickEvent) {
        let Some(camera) = host.camera() else {
            return;
        };
        OVERWORLD.store(camera.dimension.ends_with("overworld"), Ordering::Relaxed);
        DAY_TIME.store(host.day_time(), Ordering::Relaxed);
    }
}

impl RenderHooks for AuroraSkies {
    /// The pass reads the world's depth to find the open sky.
    fn render_needs(&self, event: &mut RenderNeedsEvent<'_>) {
        event.needs.world_depth = true;
    }

    fn world_drawn(&self, event: &mut WorldDrawnEvent) {
        if !OVERWORLD.load(Ordering::Relaxed) {
            return;
        }
        let night = night(DAY_TIME.load(Ordering::Relaxed));
        if night <= 0.001 {
            return;
        }
        let world = &mut *event.world;
        if world.size[0] == 0 || world.size[1] == 0 || world.depth_size[0] == 0 {
            return;
        }
        let mut gpu = self.gpu.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if gpu.as_ref().map(|pass| pass.format) != Some(world.color_format) {
            *gpu = Some(SkyPass::new(world.device, world.color_format));
        }
        let Some(pass) = gpu.as_ref() else {
            return;
        };

        // The shader's `Frame`: inverse camera matrix, then two vec4s.
        let seconds = (self.started.elapsed().as_secs_f64() % 3600.0) as f32;
        let wrap = |v: f64| (v.rem_euclid(65_536.0)) as f32;
        let mut bytes = Vec::with_capacity(UNIFORM_SIZE as usize);
        for value in world.view_proj.inverse().to_cols_array() {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [
            seconds,
            night,
            world.size[0] as f32,
            world.size[1] as f32,
            world.depth_size[0] as f32,
            world.depth_size[1] as f32,
            wrap(world.camera.x),
            wrap(world.camera.z),
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        world.queue.write_buffer(&pass.uniform, 0, &bytes);

        let group = world.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("aurora_skies sky"),
            layout: &pass.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pass.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(world.depth),
                },
            ],
        });
        let mut render = world.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("aurora_skies sky"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: world.color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        render.set_pipeline(&pass.pipeline);
        render.set_bind_group(0, &group, &[]);
        render.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::night;

    #[test]
    fn day_is_dark_free_and_midnight_is_full() {
        assert_eq!(night(6_000), 0.0);
        assert_eq!(night(18_000), 1.0);
        assert_eq!(night(18_000 + 24_000 * 5), 1.0);
        assert_eq!(night(-6_000), 1.0);
        let dusk = night(13_100);
        assert!(dusk > 0.0 && dusk < 1.0);
    }
}
