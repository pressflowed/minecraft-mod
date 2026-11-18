//! The window's half: keys, cooldowns, the effects being drawn, Superflight, camera feel and
//! sounds, and the casts it sends the server.

use crate::flight::{self, Controls, Flight, V3};
use crate::net::Cast;
use crate::powers::{self, Power};
use crate::ID;
use mod_api::{ClientHost, Host, SoundPlay, SoundSource};
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// What an effect looks like when drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A ring racing out across a plane (`to` is the plane's normal).
    Ring,
    /// A glowing shell round a point.
    Bubble,
    /// A bright ball of light.
    Orb,
    /// Rings of shock flying out from the fist along `to`.
    Punch,
    /// A jagged bolt from the sky.
    Bolt,
    /// Ice crystals spreading over the ground.
    Frost,
    /// A black hole with its accretion disk.
    Hole,
    /// The dash's rainbow trail from `at` to `to`.
    Trail,
    /// The Cataclysm's charge: light pulled into the target.
    Charge,
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub kind: Kind,
    pub start: Instant,
    pub duration: f32,
    pub at: V3,
    pub to: V3,
    /// Its full size: a radius, a length or a height.
    pub size: f64,
    /// Linear colour.
    pub color: [f32; 3],
    pub seed: f32,
}

impl Effect {
    /// Seconds since it started, negative while it waits.
    pub fn age(&self, now: Instant) -> f32 {
        if now >= self.start {
            now.duration_since(self.start).as_secs_f32()
        } else {
            -self.start.duration_since(now).as_secs_f32()
        }
    }
}

/// The laser eyes or the Omega Beam this frame.
#[derive(Clone, Copy, Debug)]
pub struct Beam {
    pub to: V3,
    pub hit: bool,
    /// 1 to 5 for the eyes; 25 for the Omega Beam.
    pub power: u8,
}

pub struct Client {
    pub panel: bool,
    pub lasers: bool,
    pub laser_power: u8,
    pub beam: Option<Beam>,
    /// The Omega Beam fires until then.
    pub omega_until: Option<Instant>,
    pub omega: Option<Beam>,
    pub ready: [Option<Instant>; 13],
    pub effects: Vec<Effect>,
    pub dash: Option<Instant>,
    pub dimension: String,
    pub day_time: i64,
    /// How long the Cataclysm's key has been held, and where it is aimed.
    pub charge: u32,
    pub charge_at: Option<V3>,
    pub flight: Option<Flight>,
    pub controls: Controls,
    /// The rotation set last frame, so the mouse's turn since is known.
    pub set_rotation: Option<(f32, f32)>,
    /// The view to draw while flying: yaw, pitch and roll.
    pub view: (f64, f64, f64),
    /// The contrails: each wingtip, oldest first.
    pub trail: VecDeque<(V3, V3)>,
    pub tapped: [Option<Instant>; 2],
    pub ticks: u64,
    seed: u32,
}

static CLIENT: Mutex<Client> = Mutex::new(Client {
    panel: true,
    lasers: false,
    laser_power: 1,
    beam: None,
    omega_until: None,
    omega: None,
    ready: [None; 13],
    effects: Vec::new(),
    dash: None,
    dimension: String::new(),
    day_time: 6000,
    charge: 0,
    charge_at: None,
    flight: None,
    controls: Controls { thrust: 0.0, roll: 0.0, lift: 0.0, boost: false },
    set_rotation: None,
    view: (0.0, 0.0, 0.0),
    trail: VecDeque::new(),
    tapped: [None, None],
    ticks: 0,
    seed: 0x9E37_79B9,
});

pub fn state() -> MutexGuard<'static, Client> {
    CLIENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn flying() -> bool {
    state().flight.is_some()
}

fn rand(c: &mut Client) -> f64 {
    let mut x = c.seed;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    c.seed = x;
    f64::from(x >> 8) / f64::from(1u32 << 24)
}

#[allow(clippy::too_many_arguments)]
fn effect(c: &mut Client, kind: Kind, delay: f32, duration: f32, at: V3, to: V3, size: f64, color: [f32; 3]) {
    let seed = rand(c) as f32 * 100.0;
    if c.effects.len() >= 160 {
        c.effects.remove(0);
    }
    let start = Instant::now() + Duration::from_secs_f32(delay.max(0.0));
    c.effects.push(Effect { kind, start, duration, at, to, size, color, seed });
}

/// sRGB `0xRRGGBB` to linear light.
pub fn linear(rgb: u32) -> [f32; 3] {
    let c = |shift: u32| (((rgb >> shift) & 0xFF) as f32 / 255.0).powf(2.2);
    [c(16), c(8), c(0)]
}

pub fn sound(host: &dyn ClientHost, name: &str, at: V3, volume: f32, pitch: f32, after_ticks: u32) {
    let play = SoundPlay::at(name, SoundSource::Players, at).with_volume(volume).with_pitch(pitch);
    let _ = host.play_local_sound(&play, after_ticks.min(1200));
}

fn send(host: &mut dyn ClientHost, cast: Cast) {
    if let Some(token) = crate::CAST.get() {
        // (refused only on a server without the mod: the power is then only seen)
        let _ = token.send(host, &cast);
    }
}

/// The local player's eye, look direction and dimension.
fn eye(host: &dyn ClientHost) -> Option<(V3, V3, String)> {
    let camera = host.camera()?;
    let player = host.local_player()?.entity();
    let host_ref: &dyn Host = host;
    let eye = player.eye_position(host_ref).unwrap_or(camera.position);
    let look = player
        .view_vector(host_ref)
        .unwrap_or_else(|| flight::look(f64::from(camera.yaw), f64::from(camera.pitch)));
    Some((eye, flight::normalize(look), camera.dimension))
}

fn target(host: &dyn ClientHost, eye: V3, dir: V3, reach: f32, otherwise: f64) -> (V3, bool) {
    match host.raycast(reach) {
        Some(hit) => ([hit.x, hit.y, hit.z], true),
        None => (flight::add(eye, flight::scale(dir, otherwise)), false),
    }
}

/// Each client tick: keys, flight, the beams and the Cataclysm's charge.
pub fn tick(host: &mut dyn ClientHost) {
    {
        let mut c = state();
        c.ticks += 1;
        if let Some(clock) = host.clock() {
            c.day_time = clock.day_time;
        }
        if let Some(camera) = host.camera() {
            c.dimension = camera.dimension;
        }
        let now = Instant::now();
        c.effects.retain(|e| e.age(now) < e.duration + 0.1);
    }
    if !host.playing() {
        return;
    }
    if host.consume_click(ID, powers::PANEL.0) {
        let mut c = state();
        c.panel = !c.panel;
    }
    if host.consume_click(ID, powers::LOOP.0) {
        if let Some(f) = state().flight.as_mut() {
            f.do_loop();
        }
    }
    for info in powers::ALL.iter() {
        if info.power != Power::Cataclysm && host.consume_click(ID, info.key) {
            activate(host, info.power);
        }
    }
    cataclysm_charge(host);
    flight_tick(host);
    beams_tick(host);
}

fn ready(power: Power) -> bool {
    let mut c = state();
    let now = Instant::now();
    let slot = power as usize;
    if c.ready[slot].is_some_and(|r| now < r) {
        return false;
    }
    c.ready[slot] = Some(now + Duration::from_secs_f32(powers::info(power).cooldown));
    true
}

fn activate(host: &mut dyn ClientHost, power: Power) {
    let Some((eye, dir, _)) = eye(host) else {
        return;
    };
    if !ready(power) {
        return;
    }
    match power {
        Power::Punch => punch(host, eye, dir, 1.0),
        Power::TitanFist => punch(host, eye, dir, 5.0),
        Power::Lasers => {
            let on = {
                let mut c = state();
                c.lasers = !c.lasers;
                if !c.lasers {
                    c.beam = None;
                }
                c.lasers
            };
            let name = if on { "minecraft:block.beacon.activate" } else { "minecraft:block.beacon.deactivate" };
            sound(host, name, eye, 1.0, 1.4, 0);
        }
        Power::OmegaBeam => {
            state().omega_until = Some(Instant::now() + Duration::from_secs(4));
            sound(host, "minecraft:block.beacon.activate", eye, 2.0, 0.5, 0);
            sound(host, "minecraft:entity.warden.sonic_charge", eye, 2.0, 0.7, 0);
            sound(host, "minecraft:entity.warden.sonic_boom", eye, 2.0, 0.5, 10);
        }
        Power::Thunder => thunder(host, eye, dir, false),
        Power::Wrath => thunder(host, eye, dir, true),
        Power::Frost => frost(host, eye, false),
        Power::AbsoluteZero => frost(host, eye, true),
        Power::Gravity => gravity(host, eye, dir, false),
        Power::Singularity => gravity(host, eye, dir, true),
        Power::Dash => dash(host, eye, dir),
        Power::Flight => toggle_flight(host, eye, dir),
        Power::Cataclysm => {}
    }
}

/// Super Punch and Titan Fist (`k` 5): shock rings fly from the fist, a shockwave races out.
fn punch(host: &mut dyn ClientHost, eye: V3, dir: V3, k: f64) {
    let (impact, _) = target(host, eye, dir, 5.0 * k as f32, 3.5);
    let ground = [impact[0], impact[1] - 0.5, impact[2]];
    let fist = flight::add(flight::add(eye, flight::scale(dir, 0.7)), [0.0, -0.25, 0.0]);
    let kf = k as f32;
    {
        let mut c = state();
        let c = &mut *c;
        effect(c, Kind::Punch, 0.0, 0.45 * kf.sqrt(), fist, dir, 16.0 * k, [1.0, 0.72, 0.35]);
        effect(c, Kind::Ring, 0.0, 1.6 * kf.sqrt(), ground, [0.0, 1.0, 0.0], 34.0 * k, [1.0, 0.55, 0.2]);
        effect(c, Kind::Bubble, 0.05, 1.2 * kf.sqrt(), flight::add(eye, flight::scale(dir, 6.0 * k)), dir, 8.0 * k, [1.0, 0.8, 0.5]);
        effect(c, Kind::Orb, 0.0, 0.35, fist, dir, 0.8 * k, [1.0, 0.9, 0.7]);
        if k > 1.0 {
            effect(c, Kind::Ring, 0.25, 2.4, ground, [0.0, 1.0, 0.0], 120.0, [1.0, 0.35, 0.1]);
            effect(c, Kind::Orb, 0.1, 1.2, flight::add(eye, flight::scale(dir, 30.0)), dir, 14.0, [1.0, 0.6, 0.25]);
        }
    }
    sound(host, "minecraft:entity.player.attack.knockback", eye, 1.0, 0.6, 0);
    sound(host, "minecraft:entity.generic.explode", impact, 2.0 * kf, 0.7 / kf.sqrt(), 0);
    sound(host, "minecraft:entity.warden.sonic_boom", impact, 1.5 * kf, 0.8 / kf.sqrt(), 1);
    if k > 1.0 {
        sound(host, "minecraft:entity.lightning_bolt.thunder", impact, 4.0, 0.5, 4);
    }
    let power = if k > 1.0 { Power::TitanFist } else { Power::Punch };
    send(host, Cast::new(power, eye, dir, 1));
}

/// Thunder Strike, or Wrath of the Gods: bolts fall where the shared scatter says.
fn thunder(host: &mut dyn ClientHost, eye: V3, dir: V3, wrath: bool) {
    let (center, _) = target(host, eye, dir, 192.0, 48.0);
    let seed = {
        let mut c = state();
        (rand(&mut c) * 255.0) as u8
    };
    let (count, radius, ticks) = if wrath { (36, 46.0, 70) } else { (7, 8.0, 18) };
    {
        let mut c = state();
        let c = &mut *c;
        for (offset, tick) in powers::bolts(seed, count, radius, ticks) {
            let at = [center[0] + offset[0], center[1], center[2] + offset[1]];
            let delay = tick as f32 / 20.0;
            effect(c, Kind::Bolt, delay, 0.9, at, [0.0; 3], 160.0, [0.55, 0.72, 1.0]);
            effect(c, Kind::Ring, delay, 1.0, [at[0], at[1] + 0.1, at[2]], [0.0, 1.0, 0.0], if wrath { 14.0 } else { 9.0 }, [0.45, 0.65, 1.0]);
        }
        effect(c, Kind::Ring, 0.0, 1.4, center, [0.0, 1.0, 0.0], if wrath { 90.0 } else { 20.0 }, [0.45, 0.65, 1.0]);
        if wrath {
            effect(c, Kind::Bubble, 0.0, 3.5, center, [0.0, 1.0, 0.0], 60.0, [0.4, 0.55, 1.0]);
        }
    }
    sound(host, "minecraft:entity.lightning_bolt.thunder", center, if wrath { 10.0 } else { 4.0 }, 0.8, 0);
    if wrath {
        for k in 1..6 {
            sound(host, "minecraft:entity.lightning_bolt.thunder", center, 8.0, 0.6 + 0.1 * k as f32, k * 12);
        }
    }
    send(host, Cast::new(if wrath { Power::Wrath } else { Power::Thunder }, center, dir, seed));
}

/// Frost Nova, or Absolute Zero: ice spreads out from the feet.
fn frost(host: &mut dyn ClientHost, eye: V3, absolute: bool) {
    let feet = [eye[0], eye[1] - 1.62, eye[2]];
    let r = if absolute { 70.0 } else { 15.0 };
    {
        let mut c = state();
        let c = &mut *c;
        effect(c, Kind::Frost, 0.0, if absolute { 8.0 } else { 3.5 }, feet, [0.0, 1.0, 0.0], r, [0.6, 0.9, 1.0]);
        effect(c, Kind::Ring, 0.0, if absolute { 3.0 } else { 1.4 }, feet, [0.0, 1.0, 0.0], r * 1.2, [0.55, 0.9, 1.0]);
        if absolute {
            effect(c, Kind::Bubble, 0.0, 3.0, feet, [0.0, 1.0, 0.0], 80.0, [0.6, 0.9, 1.0]);
            effect(c, Kind::Ring, 0.6, 3.0, feet, [0.0, 1.0, 0.0], 110.0, [0.8, 0.95, 1.0]);
        }
    }
    sound(host, "minecraft:block.glass.break", feet, 1.5, 0.6, 0);
    sound(host, "minecraft:entity.player.hurt_freeze", feet, 1.0, 0.8, 0);
    sound(host, "minecraft:block.amethyst_block.chime", feet, 2.0, 0.5, 3);
    if absolute {
        sound(host, "minecraft:entity.elder_guardian.curse", feet, 1.0, 0.5, 0);
        sound(host, "minecraft:block.glass.break", feet, 3.0, 0.4, 10);
    }
    send(host, Cast::new(if absolute { Power::AbsoluteZero } else { Power::Frost }, feet, [0.0, 1.0, 0.0], 1));
}

/// Gravity Well, or the Singularity: a black hole where the player looks.
fn gravity(host: &mut dyn ClientHost, eye: V3, dir: V3, singularity: bool) {
    let (reach, back, up) = if singularity { (160.0, 20.0, 18.0) } else { (48.0, 3.0, 2.5) };
    let (hit, found) = target(host, eye, dir, reach, if singularity { 70.0 } else { 18.0 });
    let center = if found { flight::add(hit, flight::scale(dir, -back)) } else { hit };
    let center = [center[0], center[1] + up, center[2]];
    let (rs, dur, wave) = if singularity { (7.0, 8.0, 130.0) } else { (1.6, 5.0, 26.0) };
    {
        let mut c = state();
        let c = &mut *c;
        effect(c, Kind::Hole, 0.0, dur, center, [0.0, 1.0, 0.0], rs, [1.0, 0.55, 0.2]);
        effect(c, Kind::Ring, dur - 0.15, 1.8 * if singularity { 1.6 } else { 1.0 }, center, [0.0, 1.0, 0.0], wave, [0.75, 0.4, 1.0]);
        effect(c, Kind::Bubble, dur - 0.1, 1.6, center, [0.0, 1.0, 0.0], wave * 0.6, [0.75, 0.45, 1.0]);
        effect(c, Kind::Orb, dur - 0.15, 0.8, center, [0.0, 1.0, 0.0], rs * 3.0, [0.9, 0.7, 1.0]);
    }
    let end = (dur * 20.0) as u32;
    sound(host, "minecraft:block.portal.trigger", center, if singularity { 6.0 } else { 2.0 }, 0.5, 0);
    sound(host, "minecraft:block.beacon.activate", center, 2.0, 0.5, 0);
    sound(host, "minecraft:entity.warden.sonic_boom", center, if singularity { 8.0 } else { 3.0 }, 0.5, end.saturating_sub(2));
    sound(host, "minecraft:entity.generic.explode", center, if singularity { 10.0 } else { 4.0 }, 0.5, end);
    send(host, Cast::new(if singularity { Power::Singularity } else { Power::Gravity }, center, dir, 1));
}

/// Hyper Dash: shoots the player forward.
fn dash(host: &mut dyn ClientHost, eye: V3, dir: V3) {
    let kick = [dir[0] * 3.4, (dir[1] * 3.4 + 0.35).clamp(-4.0, 4.0), dir[2] * 3.4];
    host.push_player([kick[0] as f32, kick[1] as f32, kick[2] as f32]);
    let end = flight::add(eye, flight::scale(dir, 16.0));
    {
        let mut c = state();
        let c = &mut *c;
        c.dash = Some(Instant::now());
        let behind = flight::add(eye, flight::scale(dir, -1.0));
        let low = [0.0, -0.4, 0.0];
        effect(c, Kind::Trail, 0.0, 1.4, flight::add(behind, low), flight::add(end, low), 0.6, [1.0, 1.0, 1.0]);
        effect(c, Kind::Ring, 0.0, 0.9, behind, dir, 7.0, [0.6, 1.0, 0.85]);
    }
    sound(host, "minecraft:entity.breeze.wind_burst", eye, 1.5, 0.7, 0);
    sound(host, "minecraft:item.trident.riptide_3", eye, 1.2, 1.2, 0);
    send(host, Cast::new(Power::Dash, eye, dir, 1));
}

/// The Cataclysm: hold its key for two seconds. Letting go early calls it off.
fn cataclysm_charge(host: &mut dyn ClientHost) {
    let held = host.is_down(ID, powers::info(Power::Cataclysm).key);
    let Some((eye, dir, _)) = eye(host) else {
        return;
    };
    if !held {
        let mut c = state();
        c.charge = 0;
        c.charge_at = None;
        return;
    }
    let (charge, aim) = {
        let mut c = state();
        let now = Instant::now();
        if c.ready[Power::Cataclysm as usize].is_some_and(|r| now < r) {
            return;
        }
        c.charge += 1;
        (c.charge, c.charge_at)
    };
    let center = match aim {
        Some(at) => at,
        None => {
            let (hit, _) = target(host, eye, dir, 1024.0, 200.0);
            let mut c = state();
            c.charge_at = Some(hit);
            effect(&mut c, Kind::Charge, 0.0, powers::CATACLYSM_CHARGE as f32 / 20.0 + 0.2, hit, [0.0, 1.0, 0.0], 60.0, [1.0, 0.35, 0.1]);
            hit
        }
    };
    if charge % 10 == 1 {
        sound(host, "minecraft:block.beacon.ambient", eye, 2.0, 0.5 + charge as f32 / 40.0, 0);
        sound(host, "minecraft:entity.warden.heartbeat", center, 8.0, 0.6, 0);
    }
    if charge < powers::CATACLYSM_CHARGE {
        return;
    }
    if !ready(Power::Cataclysm) {
        return;
    }
    {
        let mut c = state();
        c.charge = 0;
        c.charge_at = None;
    }
    cataclysm(host, eye, center);
}

fn cataclysm(host: &mut dyn ClientHost, eye: V3, center: V3) {
    let r = powers::CATACLYSM_RADIUS;
    {
        let mut c = state();
        let c = &mut *c;
        let up = [0.0, 1.0, 0.0];
        effect(c, Kind::Orb, 0.0, 6.0, center, up, 90.0, [1.0, 0.7, 0.35]);
        effect(c, Kind::Orb, 0.4, 7.0, flight::add(center, [0.0, 120.0, 0.0]), up, 110.0, [1.0, 0.45, 0.15]);
        effect(c, Kind::Bubble, 0.0, 4.0, center, up, r * 1.05, [1.0, 0.75, 0.45]);
        effect(c, Kind::Ring, 0.0, 4.5, center, up, r, [1.0, 0.5, 0.15]);
        effect(c, Kind::Ring, 0.8, 5.0, center, up, r * 1.35, [1.0, 0.3, 0.1]);
        effect(c, Kind::Ring, 1.6, 5.0, flight::add(center, [0.0, 60.0, 0.0]), up, r * 0.8, [1.0, 0.85, 0.6]);
        for k in 0..18 {
            let a = k as f64 / 18.0 * std::f64::consts::TAU + rand(c) * 0.3;
            let d = 60.0 + rand(c) * (r - 80.0);
            let at = [center[0] + a.cos() * d, center[1], center[2] + a.sin() * d];
            let delay = 0.2 + rand(c) as f32 * 3.0;
            effect(c, Kind::Bolt, delay, 0.8, at, [0.0; 3], 220.0, [1.0, 0.6, 0.35]);
        }
    }
    // The blast is heard when its shock arrives: sound covers about 17 blocks a tick.
    let distance = flight::length(flight::sub(center, eye));
    let delay = (distance / 17.0) as u32;
    sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.3, delay);
    sound(host, "minecraft:entity.warden.sonic_boom", eye, 1.0, 0.4, delay);
    sound(host, "minecraft:entity.lightning_bolt.thunder", eye, 1.0, 0.4, delay + 4);
    sound(host, "minecraft:entity.ender_dragon.growl", eye, 1.0, 0.5, delay + 10);
    sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.25, delay + 20);
    let dir = flight::normalize(flight::sub(center, eye));
    send(host, Cast::new(Power::Cataclysm, center, dir, 1));
}

fn toggle_flight(host: &mut dyn ClientHost, eye: V3, dir: V3) {
    let player = host.local_player().map(|p| p.entity());
    let host_ref: &dyn Host = host;
    let velocity = player.and_then(|p| p.velocity(host_ref)).unwrap_or([0.0; 3]);
    let rotation = player.and_then(|p| p.rotation(host_ref));
    let on = {
        let mut c = state();
        if c.flight.take().is_some() {
            c.set_rotation = None;
            c.trail.clear();
            false
        } else {
            let (yaw, pitch) = rotation.map_or_else(|| flight::yaw_pitch(dir), |r| (f64::from(r[0]), f64::from(r[1])));
            c.flight = Some(Flight::start(yaw, pitch, velocity));
            c.view = (yaw, pitch, 0.0);
            c.set_rotation = None;
            true
        }
    };
    if on {
        sound(host, "minecraft:item.elytra.flying", eye, 0.8, 1.2, 0);
        sound(host, "minecraft:entity.breeze.wind_burst", eye, 1.0, 1.4, 0);
        let mut c = state();
        effect(&mut c, Kind::Ring, 0.0, 0.8, [eye[0], eye[1] - 1.6, eye[2]], [0.0, 1.0, 0.0], 9.0, [0.8, 0.95, 1.0]);
    } else {
        sound(host, "minecraft:entity.player.attack.sweep", eye, 1.0, 0.7, 0);
    }
    send(host, Cast::new(Power::Flight, eye, dir, u8::from(on)));
}

/// Superflight, each tick: steer, then set the player's velocity to the flight's.
fn flight_tick(host: &mut dyn ClientHost) {
    let Some(player) = host.local_player().map(|p| p.entity()) else {
        return;
    };
    let host_ref: &dyn Host = host;
    let velocity = player.velocity(host_ref).unwrap_or([0.0; 3]);
    let position = player.position(host_ref);
    let target = {
        let mut c = state();
        let controls = c.controls;
        let Some(f) = c.flight.as_mut() else {
            return;
        };
        f.tick(controls);
        let (target, attitude) = (f.velocity, f.attitude);
        if let Some(p) = position {
            let right = attitude.right();
            let tip = |side: f64| flight::add(flight::add(p, flight::scale(right, 1.1 * side)), [0.0, 1.0, 0.0]);
            c.trail.push_back((tip(-1.0), tip(1.0)));
            while c.trail.len() > 36 {
                c.trail.pop_front();
            }
        }
        target
    };
    // The game moves the player itself: push the difference, at most 4 a tick on each axis.
    let push = flight::sub(target, velocity).map(|v| v.clamp(-4.0, 4.0) as f32);
    host.push_player(push);
    let ticks = state().ticks;
    if ticks % 20 == 0 {
        if let Some(p) = position {
            let speed = flight::length(target) as f32;
            sound(host, "minecraft:item.elytra.flying", p, (0.2 + speed * 0.25).min(1.0), 0.8 + speed * 0.15, 0);
        }
    }
}

/// Superflight, each frame: the mouse turns the flyer in its own frame, and the player's
/// rotation is set back to the flyer's nose (at most 80 degrees up or down, so the next turn
/// is measured whole; the camera draws the true pitch).
pub fn flight_frame(host: &mut dyn ClientHost) {
    let Some(player) = host.local_player().map(|p| p.entity()) else {
        return;
    };
    let host_ref: &dyn Host = host;
    let Some([yaw, pitch]) = player.rotation(host_ref) else {
        return;
    };
    let set = {
        let mut c = state();
        let last = c.set_rotation;
        let Some(f) = c.flight.as_mut() else {
            return;
        };
        if let Some((last_yaw, last_pitch)) = last {
            let mut d_yaw = f64::from(yaw - last_yaw);
            d_yaw = (d_yaw + 180.0).rem_euclid(360.0) - 180.0;
            let d_pitch = f64::from(pitch - last_pitch);
            if d_yaw.abs() < 90.0 && d_pitch.abs() < 90.0 {
                f.attitude.yaw_by(d_yaw);
                f.attitude.pitch_by(d_pitch);
            }
        }
        let view = f.attitude.view();
        c.view = view;
        let set = (view.0 as f32, (view.1 as f32).clamp(-80.0, 80.0));
        c.set_rotation = Some(set);
        set
    };
    let _ = host.set_local_rotation(set.0, set.1);
}

/// The movement keys while flying: they steer the flight, not the feet.
pub fn movement(forward: &mut f32, strafe: &mut f32, jump: &mut bool, sneak: &mut bool, sprint: bool) {
    let mut c = state();
    if c.flight.is_none() {
        return;
    }
    c.controls = Controls {
        thrust: f64::from(*forward).clamp(-1.0, 1.0),
        // vanilla's strafe is positive to the left
        roll: -f64::from(*strafe).clamp(-1.0, 1.0),
        lift: f64::from(u8::from(*jump)) - f64::from(u8::from(*sneak)),
        boost: sprint && *forward > 0.0,
    };
    *forward = 0.0;
    *strafe = 0.0;
    *jump = false;
    *sneak = false;
}

/// A double tap of the left or right key while flying: a barrel roll that way.
pub fn tap(mapping: &str) {
    let side = match mapping {
        "key.left" => 0,
        "key.right" => 1,
        _ => return,
    };
    let mut c = state();
    if c.flight.is_none() {
        return;
    }
    let now = Instant::now();
    let double = c.tapped[side].is_some_and(|t| now.duration_since(t) < Duration::from_millis(280));
    c.tapped[side] = if double { None } else { Some(now) };
    if double {
        if let Some(f) = c.flight.as_mut() {
            f.barrel_roll(side == 1);
        }
    }
}

/// The use key while the laser eyes are on raises their power instead: true to drop the press.
pub fn use_key() -> bool {
    let mut c = state();
    if !c.lasers {
        return false;
    }
    c.laser_power = c.laser_power % 5 + 1;
    true
}

/// The laser eyes and the Omega Beam: where they hit, and what the server is told.
fn beams_tick(host: &mut dyn ClientHost) {
    let Some((eye, dir, _)) = eye(host) else {
        return;
    };
    let (lasers, power, omega, ticks) = {
        let mut c = state();
        let omega = c.omega_until.is_some_and(|t| Instant::now() < t);
        if !omega {
            c.omega_until = None;
            c.omega = None;
        }
        (c.lasers, c.laser_power, omega, c.ticks)
    };
    if lasers {
        let (to, hit) = target(host, eye, dir, 96.0, 96.0);
        state().beam = Some(Beam { to, hit, power });
        let every = u64::from(7 - power.min(5)).max(1);
        if hit && ticks % every == 0 {
            send(host, Cast::new(Power::Lasers, to, dir, power));
        }
        if hit && ticks % 4 == 0 {
            sound(host, "minecraft:block.fire.extinguish", to, 0.5, 1.6, 0);
        }
        if ticks % 30 == 0 {
            sound(host, "minecraft:block.beacon.ambient", eye, 0.9, 0.6 + 0.25 * f32::from(power), 0);
        }
    }
    if omega {
        let (to, hit) = target(host, eye, dir, 256.0, 256.0);
        state().omega = Some(Beam { to, hit, power: 25 });
        if ticks % 2 == 0 {
            send(host, Cast::new(Power::OmegaBeam, eye, dir, 5));
        }
        if ticks % 6 == 0 {
            sound(host, "minecraft:entity.generic.explode", to, 2.0, 0.8, 0);
            let mut c = state();
            effect(&mut c, Kind::Ring, 0.0, 0.8, to, flight::scale(dir, -1.0), 10.0, [1.0, 0.3, 0.5]);
        }
        if ticks % 12 == 0 {
            sound(host, "minecraft:block.beacon.ambient", eye, 2.0, 0.5, 0);
        }
    }
}

/// Each effect's push on the camera, the flyer's roll, and the beams' tremor.
pub fn shake(shake: &mut mod_api::Shake) {
    let c = state();
    let now = Instant::now();
    let mut amount = 0.0f32;
    for e in &c.effects {
        let age = e.age(now);
        if age < 0.0 || age > e.duration {
            continue;
        }
        let t = age / e.duration;
        amount += match e.kind {
            Kind::Punch => 3.0 * (e.size as f32 / 16.0).sqrt() * (1.0 - t).powi(3),
            Kind::Ring => (e.size as f32 / 30.0).min(4.0) * (1.0 - t).powi(3),
            Kind::Bolt => 0.6 * (1.0 - t).powi(2),
            Kind::Hole => 0.3 + e.size as f32 * 0.08 + 0.6 * t * t,
            Kind::Trail => 1.2 * (1.0 - t).powi(4),
            Kind::Charge => 0.4 + 1.2 * t,
            _ => 0.0,
        };
    }
    if c.lasers && c.beam.is_some_and(|b| b.hit) {
        amount += 0.05 * f32::from(c.laser_power);
    }
    if c.omega.is_some() {
        amount += 1.4;
    }
    if c.flight.is_some() {
        shake.roll += c.view.2 as f32;
        if c.controls.boost {
            amount += 0.15;
        }
    }
    if amount > 0.0 {
        let amount = amount.min(6.0);
        let t = c.ticks as f32 * 0.05 + now.elapsed().as_secs_f32();
        let wobble = |f: f32, p: f32| (t * f + p).sin() * 0.6 + (t * f * 2.31 + p * 1.7).sin() * 0.4;
        shake.pitch += amount * wobble(47.0, 0.0);
        shake.yaw += amount * wobble(41.0, 1.3) * 0.8;
        shake.roll += amount * wobble(37.0, 2.1) * 0.5;
    }
}

/// The field of view: a dash and speed widen it.
pub fn fov(degrees: f32) -> f32 {
    let c = state();
    let mut out = degrees;
    if let Some(dash) = c.dash {
        let t = dash.elapsed().as_secs_f32();
        if t < 1.2 {
            let kick = if t < 0.12 { t / 0.12 } else { (1.0 - (t - 0.12) / 1.08).powi(2) };
            out += 28.0 * kick;
        }
    }
    if let Some(f) = &c.flight {
        out += (f.speed() as f32 * 6.0).min(22.0);
    }
    out.clamp(1.0, 170.0)
}

/// The view while flying, for the camera: yaw and pitch (the roll goes in through the shake).
pub fn flight_view() -> Option<(f32, f32)> {
    let c = state();
    c.flight.map(|_| (c.view.0 as f32, c.view.1 as f32))
}
