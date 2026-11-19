//! What the `draw` mixin (`src/mixins/draw.rs`) asks the mod each frame, as plain data: the
//! effect passes' uniform, the jets' and the weapons' models to draw, the meshes, and the shaders.
//! The mixin owns every GPU object; nothing here touches the GPU.

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

/// The `draw` mixin's GPU objects, kept between frames. The mod can't name their types (they are
/// the mixin's own), so they are kept as `Any`.
static GPU: Mutex<Option<Box<dyn Any + Send>>> = Mutex::new(None);

pub fn gpu_state() -> MutexGuard<'static, Option<Box<dyn Any + Send>>> {
    GPU.lock().unwrap_or_else(|p| p.into_inner())
}

/// Whether the player has the mod on (set each tick; the render thread has no host to ask).
pub static ENABLED: AtomicBool = AtomicBool::new(true);

// ---- turning the picture ----------------------------------------------------------------------
//
// The game caps the roll a mod's camera shake gives at 10 degrees. Superflight's bank and barrel
// rolls and a jet's cockpit lean go further: the shake gives the first `SHAKE_ROLL` degrees, and
// the `draw` mixin turns the finished picture of the world the rest of the way. The view is
// widened (`compute_fov`) by the zoom that turning needs to keep the corners filled, so what the
// player sees stays the same size.

/// The roll the camera shake gives (the game's cap is 10).
pub const SHAKE_ROLL: f32 = 9.0;

/// The roll the picture is turned by this frame, degrees, and the screen's width over height.
static PICTURE_ROLL: AtomicU32 = AtomicU32::new(0);
static ASPECT: AtomicU32 = AtomicU32::new(0x3FE3_8E39); // 16:9

/// Splits the camera's total roll (degrees) into the shake's part and the picture's.
pub fn split_roll(total: f32) -> (f32, f32) {
    let total = if total.is_finite() { (total + 180.0).rem_euclid(360.0) - 180.0 } else { 0.0 };
    let shake = total.clamp(-SHAKE_ROLL, SHAKE_ROLL);
    (shake, total - shake)
}

/// How much a picture turned by `degrees` must be enlarged to fill a screen `aspect` wide.
pub fn zoom_for(degrees: f32, aspect: f32) -> f32 {
    let (s, c) = degrees.to_radians().sin_cos();
    let aspect = aspect.max(1.0 / aspect);
    (c.abs() + aspect * s.abs()).max(1.0)
}

/// Sets this frame's picture roll, and returns the field of view (vertical degrees) that keeps
/// the picture its size once it is enlarged to turn.
pub fn turn_picture(roll: f32, fov: f32) -> f32 {
    let roll = if roll.abs() < 0.01 { 0.0 } else { roll };
    PICTURE_ROLL.store(roll.to_bits(), Ordering::Relaxed);
    if roll == 0.0 {
        return fov;
    }
    let zoom = zoom_for(roll, f32::from_bits(ASPECT.load(Ordering::Relaxed)));
    let half = (fov.clamp(1.0, 170.0) * 0.5).to_radians();
    (2.0 * (half.tan() * zoom).atan()).to_degrees().clamp(1.0, 170.0)
}

/// The picture turn's uniform: cos, sin, zoom, aspect.
pub const ROLL_SIZE: usize = 16;

/// Turns the picture about the middle of the screen: each pixel reads the copy turned back and
/// shrunk by the zoom.
pub const ROLL_WGSL: &str = r#"
struct Roll { c: f32, s: f32, zoom: f32, aspect: f32 }
@group(0) @binding(0) var<uniform> roll: Roll;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var smooth_sampler: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> Out {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.position = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}

@fragment
fn fs_roll(in: Out) -> @location(0) vec4<f32> {
    // y up, in screen heights, from the middle
    let p = vec2<f32>((in.uv.x - 0.5) * roll.aspect, 0.5 - in.uv.y);
    // turned back by the roll, shrunk by the zoom
    let q = vec2<f32>(roll.c * p.x + roll.s * p.y, -roll.s * p.x + roll.c * p.y) / roll.zoom;
    let uv = vec2<f32>(q.x / roll.aspect + 0.5, 0.5 - q.y);
    return textureSampleLevel(picture, smooth_sampler, uv, 0.0);
}
"#;

/// The jets' `Frame` uniform: a matrix and fifteen vec4s.
pub const JET_FRAME_SIZE: usize = 64 + 16 * 15;
/// The jets' `Object` uniform, and its slot in the buffer (dynamic offsets go in 256s).
pub const OBJECT_SIZE: usize = 64 + 128 + 32;
pub const OBJECT_STRIDE: u64 = 256;
pub const MAX_DRAWS: usize = 48;

/// The models: the five jets, then a missile and a bomb.
pub const MISSILE: usize = crate::jet_sim::COUNT;
pub const BOMB: usize = MISSILE + 1;
pub const MODELS: usize = BOMB + 1;

/// The effect passes' shader, and the jets' (models, then laying their layer over the picture).
pub const EFFECTS_WGSL: &str = crate::shader::WGSL;
pub const JET_WGSL: &str = crate::jet_shader::WGSL;
pub const COMPOSITE_WGSL: &str = crate::jet_shader::COMPOSITE;
/// The effect passes' uniform size.
pub const EFFECTS_SIZE: usize = crate::render::UNIFORM_SIZE;
/// Floats per model vertex.
pub const VERTEX_FLOATS: usize = crate::jet_mesh::VERTEX_FLOATS;

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

impl Draw {
    /// The `Object` uniform's bytes for this draw: its solid pass, or its glass pass.
    pub fn object(&self, glass: bool) -> [u8; OBJECT_SIZE] {
        let mut out = [0u8; OBJECT_SIZE];
        let mut put = |at: usize, values: &[f32]| {
            for (k, v) in values.iter().enumerate() {
                out[at + k * 4..at + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        };
        put(0, &self.model);
        put(64, &self.parts);
        put(192, &[self.glow, f32::from(u8::from(self.hide_pilot)), f32::from(u8::from(self.world_test)), f32::from(u8::from(glass))]);
        put(208, &self.cut);
        out
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

/// Model `id`'s vertices (floats, `VERTEX_FLOATS` each) and triangle indices.
pub fn mesh(id: usize) -> (Vec<f32>, Vec<u32>) {
    let mesh = match id {
        MISSILE => crate::jet_mesh::ordnance(true),
        BOMB => crate::jet_mesh::ordnance(false),
        kind if kind < crate::jet_sim::COUNT => crate::jet_mesh::jet(kind),
        _ => return (Vec::new(), Vec::new()),
    };
    (mesh.floats(), mesh.indices)
}

/// This frame, as the passes draw it.
pub struct Frame {
    /// The effect passes' uniform, and which of them run.
    pub effects: Vec<u8>,
    pub lens: bool,
    pub glow: bool,
    /// The jets' uniform and their models, when any show.
    pub jets: Option<(Vec<u8>, Vec<Draw>)>,
    /// The picture turn's uniform (`ROLL_WGSL`), when the picture turns.
    pub roll: Option<[u8; ROLL_SIZE]>,
}

fn started() -> Instant {
    static AT: OnceLock<Instant> = OnceLock::new();
    *AT.get_or_init(Instant::now)
}

/// What to draw now, from the camera at `camera` (the world's position) through `view_proj`
/// (camera-relative world to clip, column-major); None when the mod is off.
pub fn frame(view_proj: [f32; 16], inv: [f32; 16], size: [u32; 2], depth_size: [u32; 2], camera: [f64; 3]) -> Option<Frame> {
    if !ENABLED.load(Ordering::Relaxed) || size[0] == 0 || size[1] == 0 || depth_size[0] == 0 || depth_size[1] == 0 {
        return None;
    }
    let aspect = size[0] as f32 / size[1] as f32;
    ASPECT.store(aspect.to_bits(), Ordering::Relaxed);
    let turn = f32::from_bits(PICTURE_ROLL.load(Ordering::Relaxed));
    let roll = (turn != 0.0 && turn.is_finite()).then(|| {
        let (s, c) = turn.to_radians().sin_cos();
        let mut bytes = [0u8; ROLL_SIZE];
        for (k, v) in [c, s, zoom_for(turn, aspect), aspect].into_iter().enumerate() {
            bytes[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes
    });
    let seconds = (started().elapsed().as_secs_f64() % 3600.0) as f32;
    let packed = crate::render::pack(seconds, size, depth_size, camera, view_proj, inv);
    let mut jets = crate::jet::gpu(view_proj, size, depth_size, camera, seconds);
    if DEMO.load(Ordering::Relaxed) && jets.is_none() {
        jets = Some(demo_jets(view_proj));
    }
    if !packed.glow && !packed.lens && jets.is_none() && roll.is_none() {
        return None;
    }
    Some(Frame { effects: packed.bytes, lens: packed.lens, glow: packed.glow, jets, roll })
}

static DEMO: AtomicBool = AtomicBool::new(false);

/// For checking the `draw` mixin outside the game: a night sky, a shock wave and every model in a
/// row ahead of a camera looking south.
#[doc(hidden)]
pub fn demo() {
    DEMO.store(true, Ordering::Relaxed);
    ENABLED.store(true, Ordering::Relaxed);
    crate::client::DAY_TIME.store(18_000, Ordering::Relaxed);
    turn_picture(30.0, 70.0);
    let mut c = crate::client::state();
    c.dimension = "minecraft:overworld".to_owned();
    let e = crate::client::Effect::new(crate::client::Kind::Shockwave, 0.0, 60.0, [0.0, 66.0, 40.0], [0.0, 1.0, 0.0], 14.0, [1.0, 0.6, 0.3]);
    c.push_effect(e);
}

fn demo_jets(view_proj: [f32; 16]) -> (Vec<u8>, Vec<Draw>) {
    let mut frame: Vec<f32> = view_proj.to_vec();
    frame.extend_from_slice(&[0.0, -1.0, 0.15, 1.0]);
    frame.resize(JET_FRAME_SIZE / 4, 0.0);
    let draws = (0..MODELS)
        .map(|mesh| Draw {
            mesh,
            model: model_matrix([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [(mesh as f64 - 3.0) * 16.0, -4.0, 60.0]),
            parts: [0.0; 32],
            glow: 1.0,
            hide_pilot: false,
            world_test: true,
            cut: [0.0; 4],
        })
        .collect();
    (frame.iter().flat_map(|f| f.to_le_bytes()).collect(), draws)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shake_and_the_picture_share_the_roll() {
        assert_eq!(split_roll(5.0), (5.0, 0.0));
        assert_eq!(split_roll(-40.0), (-SHAKE_ROLL, -40.0 + SHAKE_ROLL));
        let (a, b) = split_roll(350.0);
        assert!((a + b + 10.0).abs() < 1e-4);
        assert_eq!(zoom_for(0.0, 16.0 / 9.0), 1.0);
        assert!((zoom_for(90.0, 16.0 / 9.0) - 16.0 / 9.0).abs() < 1e-4);
        // a turned picture is shown wider, so it keeps its size
        assert!(turn_picture(30.0, 70.0) > 90.0);
        assert_eq!(turn_picture(0.0, 70.0), 70.0);
    }
}
