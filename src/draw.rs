//! Every effect as quads of the mod's own shaders (`assets/superpowers/shaders/pipeline`), built
//! fresh each frame from the client's effects. A quad hands its shader a colour (tint, and in
//! alpha one more number) and a uv per corner; the shader does the rest.

use crate::client::{self, Effect, Kind};
use crate::flight::{self, V3};
use mod_api::{
    ClientHost, Identifier, PipelineBlend, PipelineQuad, RenderPipeline, RenderPipelineRef,
    WorldDepth, WorldDrawContext, WorldRenderView,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// The pipelines: name, blend, and whether walls hide them.
const PIPELINES: [(&str, PipelineBlend, WorldDepth); 13] = [
    ("ring", PipelineBlend::Additive, WorldDepth::Tested),
    ("bubble", PipelineBlend::Additive, WorldDepth::Tested),
    ("orb", PipelineBlend::Additive, WorldDepth::Tested),
    ("beam", PipelineBlend::Additive, WorldDepth::Tested),
    ("rainbow", PipelineBlend::Additive, WorldDepth::Tested),
    ("disk", PipelineBlend::Additive, WorldDepth::Tested),
    ("shadow", PipelineBlend::Translucent, WorldDepth::Tested),
    ("frost", PipelineBlend::Additive, WorldDepth::Tested),
    ("aurora", PipelineBlend::Additive, WorldDepth::Tested),
    ("stars", PipelineBlend::Additive, WorldDepth::Tested),
    ("flash", PipelineBlend::Translucent, WorldDepth::ThroughWalls),
    ("frostedge", PipelineBlend::Translucent, WorldDepth::ThroughWalls),
    ("speed", PipelineBlend::Additive, WorldDepth::ThroughWalls),
];

static RESOLVED: Mutex<Option<HashMap<&'static str, RenderPipelineRef>>> = Mutex::new(None);

fn id(name: &str) -> Option<Identifier> {
    Identifier::parse(&format!("{}:{name}", crate::ID)).ok()
}

/// All optional: a shader a resource pack breaks only loses that effect.
pub fn register(pipelines: &mut Vec<RenderPipeline>) {
    for (name, blend, depth) in PIPELINES {
        if let Some(id) = id(name) {
            pipelines.push(RenderPipeline::new(id.clone(), id, blend, depth).optional());
        }
    }
}

/// At start-up and after each resource reload.
pub fn resolve(host: &mut dyn ClientHost) {
    let mut found = HashMap::new();
    for (name, _, _) in PIPELINES {
        if let Some(id) = id(name) {
            if let Ok(pipeline) = host.render_assets().render_pipeline(&id) {
                found.insert(name, pipeline);
            }
        }
    }
    *RESOLVED.lock().unwrap_or_else(|p| p.into_inner()) = Some(found);
}

/// `0xAARRGGBB` from a linear colour (sent as sRGB, as the game expects) and alpha.
fn pack(rgb: [f32; 3], a: f32) -> u32 {
    let c = |v: f32| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u32;
    let a = (a.clamp(0.0, 1.0) * 255.0).round() as u32;
    a << 24 | c(rgb[0]) << 16 | c(rgb[1]) << 8 | c(rgb[2])
}

/// The camera's axes in the world.
#[derive(Clone, Copy)]
struct Basis {
    eye: V3,
    right: V3,
    up: V3,
    forward: V3,
    fov: f64,
}

fn basis(host: &dyn ClientHost, view: &WorldRenderView) -> Basis {
    match host.camera() {
        Some(c) => {
            let v = c.view.map(f64::from);
            Basis {
                eye: view.camera,
                right: flight::normalize([v[0], v[4], v[8]]),
                up: flight::normalize([v[1], v[5], v[9]]),
                forward: flight::normalize([-v[2], -v[6], -v[10]]),
                fov: f64::from(c.fov_degrees),
            }
        }
        None => Basis { eye: view.camera, right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], forward: [0.0, 0.0, 1.0], fov: 70.0 },
    }
}

/// Quads to draw, by pipeline.
#[derive(Default)]
struct Batch {
    quads: HashMap<&'static str, Vec<PipelineQuad>>,
}

const SQUARE: [[f32; 2]; 4] = [[-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0], [1.0, 1.0]];

impl Batch {
    fn push(&mut self, pipeline: &'static str, corners: [V3; 4], uv: [[f32; 2]; 4], color: u32) {
        if corners.iter().flatten().all(|v| v.is_finite()) {
            self.quads.entry(pipeline).or_default().push(PipelineQuad::new(corners, uv, color));
        }
    }

    /// A square of half-size `r` round `center`, lying across `normal`; uv -1..1.
    fn flat(&mut self, pipeline: &'static str, center: V3, normal: V3, r: f64, color: u32) {
        let n = flight::normalize(normal);
        let helper = if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
        let u = flight::normalize(flight::cross(n, helper));
        let v = flight::cross(n, u);
        self.square(pipeline, center, u, v, r, color);
    }

    /// A square of half-size `r` facing the camera; uv -1..1.
    fn facing(&mut self, pipeline: &'static str, b: &Basis, center: V3, r: f64, color: u32) {
        self.square(pipeline, center, b.right, b.up, r, color);
    }

    fn square(&mut self, pipeline: &'static str, c: V3, u: V3, v: V3, r: f64, color: u32) {
        let at = |su: f64, sv: f64| flight::add(c, flight::add(flight::scale(u, su * r), flight::scale(v, sv * r)));
        self.push(pipeline, [at(-1.0, 1.0), at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0)], SQUARE, color);
    }

    /// A strip from `a` to `b`, `width` across, turned to face the camera; uv.x runs from
    /// `u0` to `u1` along it, uv.y -1..1 across.
    #[allow(clippy::too_many_arguments)]
    fn strip(&mut self, pipeline: &'static str, b: &Basis, a: V3, bb: V3, width: f64, u0: f32, u1: f32, color: u32) {
        let along = flight::sub(bb, a);
        let to_eye = flight::sub(b.eye, a);
        let mut side = flight::cross(along, to_eye);
        if flight::length(side) < 1e-9 {
            side = b.right;
        }
        let side = flight::scale(flight::normalize(side), width);
        let corners = [flight::add(a, side), flight::sub(a, side), flight::sub(bb, side), flight::add(bb, side)];
        self.push(pipeline, corners, [[u0, 1.0], [u0, -1.0], [u1, -1.0], [u1, 1.0]], color);
    }

    /// A quad just in front of the camera covering the view; uv 0..1 (or -1..1 if `centred`).
    fn overlay(&mut self, pipeline: &'static str, b: &Basis, color: u32, centred: bool) {
        let d = 0.2;
        let h = (b.fov.to_radians() / 2.0).tan() * d * 1.15;
        let w = h * 2.4;
        let c = flight::add(b.eye, flight::scale(b.forward, d));
        let at = |sx: f64, sy: f64| flight::add(c, flight::add(flight::scale(b.right, sx * w), flight::scale(b.up, sy * h)));
        let uv = if centred { SQUARE } else { [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]] };
        self.push(pipeline, [at(-1.0, 1.0), at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0)], uv, color);
    }
}

/// Each frame, after the particles: everything the powers show, and the night sky.
pub fn world(host: &dyn ClientHost, view: &WorldRenderView, draw: &mut dyn WorldDrawContext) {
    let resolved = RESOLVED.lock().unwrap_or_else(|p| p.into_inner());
    let Some(pipelines) = resolved.as_ref() else {
        return;
    };
    let b = basis(host, view);
    let mut batch = Batch::default();
    let now = Instant::now();
    let c = client::state();
    sky(&mut batch, &b, &c.dimension, c.day_time);
    let mut flash = 0.0f32;
    let mut frost_edge = 0.0f32;
    let mut speed = 0.0f32;
    for e in &c.effects {
        let age = e.age(now);
        if age < 0.0 || age > e.duration {
            continue;
        }
        let t = age / e.duration;
        effect(&mut batch, &b, e, age, t, &mut flash, &mut frost_edge, &mut speed);
    }
    if c.lasers {
        if let Some(beam) = c.beam {
            eyes(&mut batch, &b, beam, client::linear(crate::hud::beam_color(beam.power)), 0.018 + 0.012 * f64::from(beam.power));
        }
    }
    if let Some(beam) = c.omega {
        eyes(&mut batch, &b, beam, [1.0, 0.2, 0.45], 0.9);
        batch.facing("orb", &b, beam.to, 9.0, pack([1.0, 0.4, 0.6], 1.0));
        speed = speed.max(0.6);
        flash = flash.max(0.08);
    }
    if let Some(f) = &c.flight {
        contrails(&mut batch, &b, &c.trail);
        if c.controls.boost {
            speed = speed.max((f.speed() as f32 / 3.0).min(1.0));
        }
    }
    if let Some(dash) = c.dash {
        let t = dash.elapsed().as_secs_f32();
        if t < 0.6 {
            speed = speed.max(1.0 - t / 0.6);
        }
    }
    drop(c);
    if flash > 0.0 {
        batch.overlay("flash", &b, pack([1.0, 0.96, 0.9], flash), false);
    }
    if frost_edge > 0.0 {
        batch.overlay("frostedge", &b, pack([1.0; 3], frost_edge), false);
    }
    if speed > 0.0 {
        batch.overlay("speed", &b, pack([1.0; 3], speed), true);
    }
    for (name, quads) in batch.quads {
        if let Some(&pipeline) = pipelines.get(name) {
            // (a full budget drops this batch only; the others still draw)
            let _ = draw.pipeline_quads(pipeline, &quads);
        }
    }
}

/// The night sky over the Overworld: aurora layers, shooting stars and twinkling stars.
fn sky(batch: &mut Batch, b: &Basis, dimension: &str, day_time: i64) {
    if !dimension.ends_with("overworld") {
        return;
    }
    let night = night(day_time);
    if night <= 0.001 {
        return;
    }
    let half = 4000.0;
    let layers = 16;
    for i in 0..layers {
        let f = (f64::from(i) + 0.5) / f64::from(layers);
        let y = b.eye[1] + 220.0 + f * 300.0;
        let (x0, z0, x1, z1) = (b.eye[0] - half, b.eye[2] - half, b.eye[0] + half, b.eye[2] + half);
        // uv is the world's x and z in kilometres (wrapped to keep their detail)
        let k = |v: f64| ((v.rem_euclid(65_536.0)) / 1000.0) as f32;
        let (u0, v0) = (k(x0), k(z0));
        let (u1, v1) = (u0 + (2.0 * half / 1000.0) as f32, v0 + (2.0 * half / 1000.0) as f32);
        let corners = [[x0, y, z1], [x0, y, z0], [x1, y, z0], [x1, y, z1]];
        batch.push("aurora", corners, [[u0, v1], [u0, v0], [u1, v0], [u1, v1]], pack([night; 3], f as f32));
    }
    let y = b.eye[1] + 600.0;
    let h = 6000.0;
    let corners = [[b.eye[0] - h, y, b.eye[2] + h], [b.eye[0] - h, y, b.eye[2] - h], [b.eye[0] + h, y, b.eye[2] - h], [b.eye[0] + h, y, b.eye[2] + h]];
    batch.push("stars", corners, SQUARE, pack([night; 3], 1.0));
}

/// How far into the night `day_time` is: 0 by day, 1 at midnight, easing at dusk and dawn.
pub fn night(day_time: i64) -> f32 {
    let t = day_time.rem_euclid(24_000) as f32;
    smoothstep(12_500.0, 13_800.0, t) * (1.0 - smoothstep(22_200.0, 23_500.0, t))
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease(t: f32) -> f64 {
    f64::from(1.0 - (1.0 - t).powi(3))
}

#[allow(clippy::too_many_arguments)]
fn effect(batch: &mut Batch, b: &Basis, e: &Effect, age: f32, t: f32, flash: &mut f32, frost_edge: &mut f32, speed: &mut f32) {
    match e.kind {
        Kind::Ring => {
            let r = (e.size * ease(t)).max(0.3);
            batch.flat("ring", e.at, e.to, r, pack(e.color, (1.0 - t).powf(1.5)));
        }
        Kind::Bubble => {
            let r = (e.size * ease(t)).max(0.3);
            // from inside, the bubble would wrap the whole view
            if flight::length(flight::sub(e.at, b.eye)) > r * 1.05 {
                batch.facing("bubble", b, e.at, r, pack(e.color, (1.0 - t).powf(1.5)));
            }
        }
        Kind::Orb => {
            let r = e.size * (0.4 + 0.6 * ease((t * 3.0).min(1.0)));
            batch.facing("orb", b, e.at, r, pack(e.color, (1.0 - t).powi(2)));
            if e.size > 50.0 {
                *flash = flash.max(0.85 * (1.0 - (age / 1.5).min(1.0)).powi(2));
            }
        }
        Kind::Punch => {
            *flash = flash.max(0.25 * (1.0 - (age / 0.25).min(1.0)).powi(3));
            let dir = flight::normalize(e.to);
            let scale = e.size / 16.0;
            for k in 0..4 {
                let tk = (t * 1.5 - k as f32 * 0.14).clamp(0.0, 1.0);
                if tk <= 0.0 || tk >= 1.0 {
                    continue;
                }
                let c = flight::add(e.at, flight::scale(dir, 1.5 + f64::from(tk) * e.size));
                let r = (0.5 + f64::from(tk) * 5.5) * scale;
                batch.flat("ring", c, dir, r, pack(e.color, (1.0 - tk).powf(1.5)));
            }
        }
        Kind::Bolt => {
            let strike = if age < 0.08 { 1.0 } else if hash(e.seed + (age * 22.0).floor()) > 0.4 { 1.0 } else { 0.0 };
            let strength = strike * (1.0 - t).powf(1.5);
            if strength <= 0.0 {
                return;
            }
            *flash = flash.max(0.12 * strength * (1.0 - (age / 0.15).min(1.0)));
            let n = 26;
            let mut last = e.at;
            for i in 1..=n {
                let f = f64::from(i) / f64::from(n);
                let h = f * e.size;
                let wander = |s: f32| {
                    let big = (smooth_noise(h * 0.07, f64::from(e.seed + s)) - 0.5) * 10.0;
                    let small = (smooth_noise(h * 0.6, f64::from(e.seed + s + 3.1)) - 0.5) * 1.6;
                    (big + small) * (f * 20.0).min(1.0)
                };
                let p = [e.at[0] + wander(0.0), e.at[1] + h, e.at[2] + wander(7.3)];
                batch.strip("beam", b, last, p, 0.45, 0.0, 1.0, pack(e.color, strength));
                last = p;
            }
            batch.facing("orb", b, e.at, 4.0, pack(e.color, strength));
        }
        Kind::Frost => {
            let r = e.size * ease((age / 0.9).min(1.0));
            let fade = 1.0 - smoothstep(0.7, 1.0, t);
            batch.flat("frost", flight::add(e.at, [0.0, 0.08, 0.0]), [0.0, 1.0, 0.0], r.max(0.5), pack([e.color[0] * fade, e.color[1] * fade, e.color[2] * fade], (e.size / 100.0) as f32));
            *frost_edge = frost_edge.max((age / 0.3).min(1.0) * (1.0 - t).powf(0.7) * if e.size > 30.0 { 1.0 } else { 0.6 });
        }
        Kind::Hole => {
            let dur = e.duration;
            let grow = smoothstep(0.0, 0.7, age) * (1.0 - smoothstep(dur - 0.35, dur, age));
            let rs = e.size * f64::from(grow);
            if rs < 0.02 {
                return;
            }
            let tilt = [f64::from(e.seed).sin() * 0.35, 1.0, f64::from(e.seed).cos() * 0.35];
            batch.flat("disk", e.at, tilt, rs * 7.0, pack([1.0; 3], grow));
            batch.facing("shadow", b, e.at, rs / 0.55, pack([1.0; 3], grow));
            batch.facing("orb", b, e.at, rs * 5.0, pack([0.5, 0.25, 0.9], 0.35 * grow));
            *speed = speed.max(0.25 * grow * (e.size as f32 / 7.0).min(1.0));
        }
        Kind::Trail => {
            let w = 0.6 * f64::from(1.0 - t);
            let reach = ease((age / 0.25).min(1.0));
            let end = flight::add(e.at, flight::scale(flight::sub(e.to, e.at), reach));
            batch.strip("rainbow", b, e.at, end, w.max(0.05), 0.0, reach as f32, pack([1.0; 3], (1.0 - t).powi(2)));
        }
        Kind::Charge => {
            let k = f64::from(t);
            let r = e.size * (1.0 - k * 0.85);
            for i in 0..14 {
                let a = f64::from(i) / 14.0 * std::f64::consts::TAU + k * 3.0;
                let tilt = (f64::from(i) * 1.7).sin() * 0.8;
                let from = flight::add(e.at, [a.cos() * r, tilt * r + r * 0.4, a.sin() * r]);
                batch.strip("beam", b, from, e.at, 0.3 + k * 0.6, 0.0, r as f32, pack(e.color, 0.4 + 0.6 * t));
            }
            batch.facing("orb", b, e.at, 6.0 + k * 30.0, pack(e.color, 0.5 + 0.5 * t));
            batch.flat("ring", e.at, [0.0, 1.0, 0.0], e.size * (1.0 - k) + 2.0, pack(e.color, t));
        }
    }
}

/// The two beams from the eyes (or one Omega Beam, `width` 0.9), and where they hit.
fn eyes(batch: &mut Batch, b: &Basis, beam: client::Beam, color: [f32; 3], width: f64) {
    let base = flight::add(b.eye, flight::add(flight::scale(b.forward, 0.35), flight::scale(b.up, -0.16)));
    let omega = beam.power > 5;
    let sides: &[f64] = if omega { &[0.0] } else { &[-1.0, 1.0] };
    let length = flight::length(flight::sub(beam.to, b.eye)) as f32;
    for side in sides {
        let from = flight::add(base, flight::scale(b.right, 0.09 * side));
        // the first stretch in front of the eyes is left out: so close, it would fill the view
        let dir = flight::normalize(flight::sub(beam.to, from));
        let start = flight::add(from, flight::scale(dir, if omega { 2.0 } else { 0.6 }));
        batch.strip("beam", b, start, beam.to, width, 0.0, length, pack(color, 1.0));
    }
    if beam.hit {
        let size = if omega { 6.0 } else { 0.25 + 0.12 * f64::from(beam.power) };
        batch.facing("orb", b, beam.to, size, pack([1.0, 0.55, 0.2], 1.0));
    }
}

/// Superflight's contrails: a thin ribbon from each wingtip, fading behind.
fn contrails(batch: &mut Batch, b: &Basis, trail: &std::collections::VecDeque<(V3, V3)>) {
    let n = trail.len();
    for i in 1..n {
        let fade = i as f32 / n as f32;
        let color = pack([0.75, 0.9, 1.0], fade * 0.6);
        for side in 0..2 {
            let pick = |(l, r): (V3, V3)| if side == 0 { l } else { r };
            batch.strip("beam", b, pick(trail[i - 1]), pick(trail[i]), 0.06 * f64::from(fade) + 0.02, 0.0, 1.0, color);
        }
    }
}

fn hash(x: f32) -> f32 {
    ((x * 12.9898).sin() * 43_758.547).fract().abs()
}

/// A smooth noise of `x` in 0..1, a different curve for each `seed`.
fn smooth_noise(x: f64, seed: f64) -> f64 {
    let i = x.floor();
    let f = x - i;
    let h = |n: f64| ((n * 12.9898 + seed * 78.233).sin() * 43_758.547).fract().abs();
    let u = f * f * (3.0 - 2.0 * f);
    h(i) * (1.0 - u) + h(i + 1.0) * u
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_pack_as_argb() {
        assert_eq!(pack([1.0, 0.0, 0.0], 1.0), 0xFFFF_0000);
        assert_eq!(pack([0.0; 3], 0.0), 0);
        assert_eq!(pack([1.0, 1.0, 1.0], 0.5) >> 24, 128);
    }

    #[test]
    fn night_comes_and_goes() {
        assert_eq!(night(6_000), 0.0);
        assert_eq!(night(18_000), 1.0);
        assert!(night(13_100) > 0.0 && night(13_100) < 1.0);
    }
}
