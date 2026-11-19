//! What the two effect passes draw each frame (the GPU work itself is in the `draw` mixin,
//! `src/mixins/draw.rs`, which runs once the world is drawn, before the hand and the HUD):
//!
//! 1. The lens pass, only while something bends light: it copies the picture and draws it back
//!    bent around black holes (with the event horizon black), rippled by shock waves and
//!    frosted at the edges during a Frost Nova.
//! 2. The light pass, every night and while any power shows: aurora, shock rings and bubbles,
//!    laser beams, lightning, frost sparkle, the accretion disk and the dash's trail, added on top.

use crate::client::{self, Kind};
use crate::hud::beam_color;
use crate::items::Holding;

use crate::weather;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Effects the shader takes at once.
pub const MAX_EFFECTS: usize = 48;
/// Points of vapour and smoke trails the shader takes.
pub const MAX_TRAIL: usize = 96;
/// The `Frame` uniform: two matrices, five vec4s, the effects of four vec4s each, then the
/// trails' points.
pub const UNIFORM_SIZE: usize = 64 * 2 + 16 * 5 + MAX_EFFECTS * 64 + MAX_TRAIL * 16;

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
    let air = crate::items::atmosphere(&state, camera);
    let mut lens = air.iter().any(|v| *v > 0.005);
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
    // The camera's own axes, for things held in front of it.
    let forward = normalize(unproject(&inv, 0.0, 0.0));
    let right = normalize({
        let (a, b) = (unproject(&inv, 1.0, 0.0), unproject(&inv, -1.0, 0.0));
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    });
    let up = normalize({
        let (a, b) = (unproject(&inv, 0.0, 1.0), unproject(&inv, 0.0, -1.0));
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    });
    let items = &state.items;
    match items.holding {
        Holding::Gun => {
            if let Some((size, _)) = items.gripping {
                // the gun's beam, from the hand to the cube it holds
                let hand: [f32; 3] = std::array::from_fn(|i| forward[i] * 0.55 - up[i] * 0.28 + right[i] * 0.3);
                let reach = crate::server::hold_distance(size) as f32;
                let hold: [f32; 3] = std::array::from_fn(|i| forward[i] * reach);
                effects.push([
                    [Kind::Tether as i32 as f32, seconds, 1.0, 0.0],
                    [hand[0], hand[1], hand[2], 0.0],
                    [hold[0], hold[1], hold[2], f32::from(size)],
                    [1.0, 0.55, 0.15, 1.0],
                ]);
            } else if let Some(corner) = items.selection {
                let lo = rel(corner);
                effects.push([
                    [Kind::Box as i32 as f32, seconds, 1.0, 0.0],
                    [lo[0], lo[1], lo[2], f32::from(items.size)],
                    [0.0; 4],
                    [1.0, 0.6, 0.2, 1.0],
                ]);
            }
        }
        Holding::Scepter => {
            if let Some((at, _)) = items.aim {
                let event = &weather::EVENTS[items.selected];
                let radius = marker_radius(items.selected);
                let p = rel(at);
                let color = rgb(event.color);
                effects.push([
                    [Kind::Marker as i32 as f32, seconds, 1.0, 0.0],
                    [p[0], p[1], p[2], radius],
                    [0.0; 4],
                    [color[0], color[1], color[2], 1.0],
                ]);
            }
        }
        Holding::Nothing => {}
    }
    for e in state.effects.iter().rev() {
        let age = e.age(now);
        if age < 0.0 || age > e.duration || effects.len() >= MAX_EFFECTS {
            continue;
        }
        let t = age / e.duration;
        let mut at = rel(e.at);
        let mut to = match e.kind {
            Kind::Dash => rel(e.to),
            _ => [e.to[0] as f32, e.to[1] as f32, e.to[2] as f32],
        };
        let mut size = e.size;
        let mut q_w = e.q_w;
        match e.kind {
            Kind::Tornado => {
                let off = weather::tornado_offset(e.path, e.to[2] as u8, f64::from(age));
                at = rel([e.at[0] + off[0], e.at[1], e.at[2] + off[1]]);
            }
            Kind::Meteor => {
                // `to` is where it comes from: the head falls along it to the impact
                let left = f64::from(1.0 - t) * weather::METEOR_PATH;
                at = rel(std::array::from_fn(|i| e.at[i] + e.to[i] * left));
                to = [-e.to[0] as f32, -e.to[1] as f32, -e.to[2] as f32];
            }
            Kind::Tsunami => {
                let run = f64::from(e.size);
                let front = (weather::WAVE_SPEED * f64::from(age)).min(run);
                at = rel(std::array::from_fn(|i| e.at[i] + e.to[i] * front));
                let crash = ((age - (run / weather::WAVE_SPEED) as f32) / 2.5).clamp(0.0, 1.0);
                size = weather::WAVE_HEIGHT as f32 * (1.0 - 0.6 * crash * crash);
                q_w = crash;
            }
            _ => {}
        }
        match e.kind {
            Kind::Punch => flash = flash.max(0.22 * (1.0 - (age / 0.25).min(1.0)).powi(3)),
            Kind::Lightning => flash = flash.max(0.12 * (1.0 - (age / 0.15).min(1.0)).powi(2)),
            Kind::Orb if e.size > 50.0 => flash = flash.max(0.85 * (1.0 - (age / 1.5).min(1.0)).powi(2)),
            Kind::Frost => {
                frost = frost.max((age / 0.3).min(1.0) * (1.0 - t).powf(0.7));
                lens = true;
            }
            Kind::Gravity | Kind::Shockwave => lens = true,
            Kind::Tornado | Kind::Hurricane | Kind::Tsunami | Kind::Mushroom | Kind::Smoke => lens = true,
            Kind::Flare => flash = flash.max(0.3 * (1.0 - (age / 0.8).min(1.0)).powi(2)),
            _ => {}
        }
        effects.push([
            [e.kind as i32 as f32, age, e.duration, e.seed],
            [at[0], at[1], at[2], size],
            [to[0], to[1], to[2], q_w],
            [e.color[0], e.color[1], e.color[2], e.c_w],
        ]);
    }
    drop(state);
    // the jets: flames, vapour, tracers and markers, and the trails of vapour and smoke
    let (jet_effects, trails) = crate::jet::effects(camera);
    for e in jet_effects {
        if effects.len() >= MAX_EFFECTS {
            break;
        }
        if e[0][0] as i32 == 20 {
            lens = true;
        }
        effects.push(e);
    }
    if !trails.is_empty() {
        lens = true;
    }

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
    put(&air);
    put(&[trails.len().min(MAX_TRAIL) as f32, 0.0, 0.0, 0.0]);
    for effect in &effects {
        for row in effect {
            put(row);
        }
    }
    for _ in effects.len()..MAX_EFFECTS {
        put(&[0.0; 16]);
    }
    for point in trails.iter().take(MAX_TRAIL) {
        put(point);
    }
    bytes.resize(UNIFORM_SIZE, 0);
    Packed { bytes, lens, glow: night > 0.001 || !effects.is_empty() }
}

/// The scepter's target ring: about the size of what the event will touch.
fn marker_radius(event: usize) -> f32 {
    match event {
        e if weather::ef(e).is_some() => weather::tornado_radius(weather::ef(e).unwrap_or(1)) as f32,
        weather::HURRICANE => 30.0,
        weather::TSUNAMI => 20.0,
        weather::FLOOD => 48.0,
        weather::SOLAR_FLARE => 38.0,
        weather::NUKE => 44.0,
        weather::METEOR => 17.0,
        weather::EARTHQUAKE => 25.0,
        _ => weather::VOLCANO_RADIUS as f32,
    }
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
        assert_eq!(UNIFORM_SIZE, 4816);
    }
}
