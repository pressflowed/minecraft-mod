//! The window's half: keys, cooldowns, the effects being drawn, camera shake, field of view,
//! sounds, and what each power asks of the server.

use crate::net::{Act, CHANNEL, Use};
use crate::{ID, NAME, VERSION};
use horizon_client::mod_api::{Host, NativeCamera, NativeMod, NativeShake};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

/// A power: its key's name (`key.superpowers.<key>`), its label, its default key and its
/// cooldown in seconds.
pub struct Power {
    pub key: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    pub cooldown: f32,
    /// The panel's accent colour, `0xRRGGBB`.
    pub color: u32,
}

pub const PUNCH: usize = 0;
pub const LASERS: usize = 1;
pub const THUNDER: usize = 2;
pub const FROST: usize = 3;
pub const GRAVITY: usize = 4;
pub const DASH: usize = 5;

pub const POWERS: [Power; 6] = [
    Power { key: "punch", label: "Super Punch", default: "key.keyboard.z", cooldown: 1.2, color: 0xFFB347 },
    Power { key: "lasers", label: "Laser Eyes", default: "key.keyboard.g", cooldown: 0.25, color: 0xFF3B3B },
    Power { key: "thunder", label: "Thunder Strike", default: "key.keyboard.v", cooldown: 3.0, color: 0x7FB8FF },
    Power { key: "frost", label: "Frost Nova", default: "key.keyboard.b", cooldown: 4.0, color: 0x9FF3FF },
    Power { key: "gravity", label: "Gravity Well", default: "key.keyboard.n", cooldown: 8.0, color: 0xC17BFF },
    Power { key: "dash", label: "Hyper Dash", default: "key.keyboard.j", cooldown: 1.0, color: 0x7CFFB2 },
];

/// The key that shows and hides the panel.
pub const PANEL: (&str, &str, &str) = ("panel", "Powers panel", "key.keyboard.h");

/// What an effect looks like: the shader's `kind` numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Shockwave = 1,
    Punch = 2,
    Laser = 3,
    Lightning = 4,
    Frost = 5,
    Gravity = 6,
    Dash = 7,
}

/// One effect being drawn, in world coordinates. It starts at `start` (possibly later than
/// now) and lasts `duration` seconds.
#[derive(Clone, Debug)]
pub struct Effect {
    pub kind: Kind,
    pub start: Instant,
    pub duration: f32,
    pub at: [f64; 3],
    /// A direction or a second point, depending on the kind.
    pub to: [f64; 3],
    /// A size: a radius, a length or a height.
    pub size: f32,
    pub color: [f32; 3],
    pub seed: f32,
}

impl Effect {
    /// Seconds since it started: negative while it waits.
    pub fn age(&self, now: Instant) -> f32 {
        if now >= self.start {
            now.duration_since(self.start).as_secs_f32()
        } else {
            -self.start.duration_since(now).as_secs_f32()
        }
    }
}

/// The laser eyes this frame: from the camera to what they hit.
#[derive(Clone, Copy, Debug)]
pub struct Beam {
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub hit: bool,
    pub power: u8,
}

pub struct Client {
    pub panel: bool,
    pub lasers: bool,
    pub laser_power: u8,
    pub beam: Option<Beam>,
    /// When each power is ready again.
    pub ready: [Option<Instant>; 6],
    pub effects: Vec<Effect>,
    /// The last dash: the field of view widens, then eases back.
    pub dash: Option<Instant>,
    pub dimension: String,
    ticks: u64,
    seed: u32,
}

static CLIENT: Mutex<Client> = Mutex::new(Client {
    panel: true,
    lasers: false,
    laser_power: 1,
    beam: None,
    ready: [None; 6],
    effects: Vec::new(),
    dash: None,
    dimension: String::new(),
    ticks: 0,
    seed: 0x9E37_79B9,
});

/// When the shake's wobble started counting.
static EPOCH: OnceLock<Instant> = OnceLock::new();

/// A screen is open (chat, inventory, pause): the power keys type instead.
static SCREEN: AtomicBool = AtomicBool::new(false);
/// The time of day in ticks, for the aurora.
pub static DAY_TIME: AtomicI64 = AtomicI64::new(6000);

pub fn state() -> MutexGuard<'static, Client> {
    CLIENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn lasers_on() -> bool {
    state().lasers
}

pub fn screen_open(open: bool) {
    SCREEN.store(open, Ordering::Relaxed);
}

/// The mod's row on Options > Mods, with every key.
pub fn mod_row() -> NativeMod {
    let mut keys: Vec<[String; 3]> = POWERS
        .iter()
        .map(|p| [p.key.to_owned(), p.label.to_owned(), p.default.to_owned()])
        .collect();
    keys.push([PANEL.0.to_owned(), PANEL.1.to_owned(), PANEL.2.to_owned()]);
    NativeMod {
        id: ID.to_owned(),
        title: format!("{NAME} {VERSION}"),
        description: "Six super powers with big visuals: Super Punch, Laser Eyes, Thunder Strike, \
                      Frost Nova, Gravity Well and Hyper Dash. Press H for the keys. Aurora skies at night."
            .to_owned(),
        keys,
    }
}

/// The key the player chose for a power, as the panel shows it: `Z`, `RMB`, `F5`.
pub fn key_label(host: &dyn Host, key: &str, default: &str) -> String {
    let bound = host.mod_key(ID, key, default);
    match bound.as_str() {
        "key.mouse.left" => "LMB".to_owned(),
        "key.mouse.right" => "RMB".to_owned(),
        "key.mouse.middle" => "MMB".to_owned(),
        _ => bound
            .rsplit('.')
            .next()
            .unwrap_or("?")
            .to_ascii_uppercase(),
    }
}

/// Where the camera looks, as a unit vector (vanilla's yaw and pitch, in degrees).
pub fn look(camera: &NativeCamera) -> [f64; 3] {
    let yaw = f64::from(camera.yaw).to_radians();
    let pitch = f64::from(camera.pitch).to_radians();
    [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()]
}

fn add(a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

fn rand(client: &mut Client) -> f32 {
    // xorshift: plenty for where sparks and bolts go
    let mut x = client.seed;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    client.seed = x;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

fn effect(client: &mut Client, kind: Kind, delay: f32, duration: f32, at: [f64; 3], to: [f64; 3], size: f32, color: [f32; 3]) {
    let seed = rand(client) * 100.0;
    let start = Instant::now() + std::time::Duration::from_secs_f32(delay.max(0.0));
    // (the shader takes 24 at once: the oldest go first)
    if client.effects.len() >= 48 {
        client.effects.remove(0);
    }
    client.effects.push(Effect { kind, start, duration, at, to, size, color, seed });
}

fn send(host: &mut dyn Host, act: Act, dimension: &str, at: [f64; 3], dir: [f64; 3], level: u8) {
    let message = Use { act, dimension: dimension.to_owned(), at, dir, level };
    // (an error only while joining, or on a server without mod messages: the power is then only seen)
    let _ = host.net_send_to_server(CHANNEL, &message.encode());
}

fn sound(host: &mut dyn Host, sound: &str, at: [f64; 3], volume: f32, pitch: f32, delay: i32) {
    let _ = host.play_local_sound(sound, at, volume, pitch, delay);
}

pub fn tick(host: &mut dyn Host, camera: NativeCamera) {
    DAY_TIME.store(host.day_time(), Ordering::Relaxed);
    let mut client = state();
    client.ticks += 1;
    client.dimension = camera.dimension.clone();
    let now = Instant::now();
    client
        .effects
        .retain(|e| e.age(now) < e.duration + 0.1);
    if !client.lasers {
        return;
    }
    // The beams burn faster the stronger they are: every 6 ticks at power 1, every tick at 5.
    let power = client.laser_power;
    let every = u64::from(7 - power.min(5)).max(1);
    let beam = client.beam;
    let dimension = client.dimension.clone();
    let ticks = client.ticks;
    drop(client);
    if let Some(beam) = beam.filter(|b| b.hit) {
        if ticks % every == 0 {
            let dir = look(&camera);
            send(host, Act::Laser, &dimension, beam.to, dir, power);
        }
        if ticks % 4 == 0 {
            sound(host, "minecraft:block.fire.extinguish", beam.to, 0.5, 1.6, 0);
        }
    }
    if ticks % 30 == 0 {
        sound(host, "minecraft:block.beacon.ambient", camera.position, 0.9, 0.6 + 0.25 * f32::from(power), 0);
    }
}

/// Each frame: where the laser eyes hit.
pub fn frame(host: &mut dyn Host) {
    if !lasers_on() {
        return;
    }
    let Some(camera) = host.camera() else {
        return;
    };
    let dir = look(&camera);
    let hit = host.raycast(96.0);
    let (to, hit) = match hit {
        Some(h) => ([h.x, h.y, h.z], true),
        None => (add(camera.position, dir, 96.0), false),
    };
    let mut client = state();
    let power = client.laser_power;
    client.beam = Some(Beam { from: camera.position, to, hit, power });
}

pub fn key(host: &mut dyn Host, key: &str, pressed: bool) {
    if !pressed {
        return;
    }
    // Right click raises the laser eyes' power while they are on: 1 to 5, then back to 1.
    if key == "key.mouse.right" {
        let mut client = state();
        if client.lasers {
            client.laser_power = client.laser_power % 5 + 1;
            let power = client.laser_power;
            drop(client);
            if let Some(camera) = host.camera() {
                sound(host, "minecraft:block.note_block.chime", camera.position, 0.8, 0.5 + 0.25 * f32::from(power), 0);
                sound(host, "minecraft:block.beacon.power_select", camera.position, 0.6, 0.8 + 0.2 * f32::from(power), 0);
            }
        }
    }
    if SCREEN.load(Ordering::Relaxed) {
        return;
    }
    if host.mod_key(ID, PANEL.0, PANEL.2) == key {
        let mut client = state();
        client.panel = !client.panel;
    }
    for (index, power) in POWERS.iter().enumerate() {
        if host.mod_key(ID, power.key, power.default) == key {
            activate(host, index);
        }
    }
}

fn activate(host: &mut dyn Host, index: usize) {
    let Some(camera) = host.camera() else {
        return;
    };
    let now = Instant::now();
    {
        let mut client = state();
        if client.ready[index].is_some_and(|ready| now < ready) {
            return;
        }
        client.ready[index] =
            Some(now + std::time::Duration::from_secs_f32(POWERS[index].cooldown));
    }
    let eye = camera.position;
    let dir = look(&camera);
    let dimension = camera.dimension.clone();
    match index {
        PUNCH => punch(host, &dimension, eye, dir),
        LASERS => {
            let on = {
                let mut client = state();
                client.lasers = !client.lasers;
                if !client.lasers {
                    client.beam = None;
                }
                client.lasers
            };
            let sound_name = if on { "minecraft:block.beacon.activate" } else { "minecraft:block.beacon.deactivate" };
            sound(host, sound_name, eye, 1.0, 1.4, 0);
        }
        THUNDER => thunder(host, &dimension, eye, dir),
        FROST => frost(host, &dimension, eye),
        GRAVITY => gravity(host, &dimension, eye, dir),
        DASH => dash(host, &dimension, eye, dir),
        _ => {}
    }
}

/// Super Punch: a cone of force from the fist that shatters what is in front.
fn punch(host: &mut dyn Host, dimension: &str, eye: [f64; 3], dir: [f64; 3]) {
    let impact = match host.raycast(5.0) {
        Some(hit) => [hit.x, hit.y, hit.z],
        None => add(eye, dir, 3.5),
    };
    let ground = [impact[0], impact[1] - 0.5, impact[2]];
    let fist = add(add(eye, dir, 0.7), [0.0, -0.25, 0.0], 1.0);
    {
        let mut client = state();
        let c = &mut *client;
        effect(c, Kind::Punch, 0.0, 0.45, fist, dir, 16.0, [1.0, 0.72, 0.35]);
        effect(c, Kind::Shockwave, 0.0, 1.6, ground, [0.0, 1.0, 0.0], 34.0, [1.0, 0.6, 0.25]);
    }
    sound(host, "minecraft:entity.player.attack.knockback", eye, 1.0, 0.6, 0);
    sound(host, "minecraft:entity.generic.explode", impact, 2.0, 0.7, 0);
    sound(host, "minecraft:entity.warden.sonic_boom", impact, 1.5, 0.8, 1);
    sound(host, "minecraft:block.anvil.land", impact, 0.6, 0.5, 2);
    send(host, Act::Punch, dimension, eye, dir, 1);
}

/// Thunder Strike: a ring of lightning around what the player looks at.
fn thunder(host: &mut dyn Host, dimension: &str, eye: [f64; 3], dir: [f64; 3]) {
    let target = match host.raycast(160.0) {
        Some(hit) => [hit.x, hit.y, hit.z],
        None => add(eye, dir, 40.0),
    };
    {
        let mut client = state();
        let c = &mut *client;
        effect(c, Kind::Lightning, 0.0, 0.9, target, [0.0; 3], 140.0, [0.55, 0.75, 1.0]);
        effect(c, Kind::Shockwave, 0.0, 1.3, target, [0.0, 1.0, 0.0], 20.0, [0.45, 0.65, 1.0]);
        for k in 0..6 {
            let angle = k as f64 / 6.0 * std::f64::consts::TAU + f64::from(rand(c)) * 0.6;
            let radius = 5.0 + f64::from(rand(c)) * 4.0;
            let at = [target[0] + angle.cos() * radius, target[1], target[2] + angle.sin() * radius];
            let delay = 0.15 * (k + 1) as f32;
            effect(c, Kind::Lightning, delay, 0.8, at, [0.0; 3], 140.0, [0.6, 0.7, 1.0]);
        }
    }
    sound(host, "minecraft:entity.lightning_bolt.thunder", target, 4.0, 0.8, 0);
    sound(host, "minecraft:entity.lightning_bolt.impact", target, 2.0, 1.0, 0);
    send(host, Act::Thunder, dimension, target, dir, 1);
}

/// Frost Nova: ice spreads out from the player's feet.
fn frost(host: &mut dyn Host, dimension: &str, eye: [f64; 3]) {
    let feet = [eye[0], eye[1] - 1.62, eye[2]];
    {
        let mut client = state();
        let c = &mut *client;
        effect(c, Kind::Frost, 0.0, 3.5, feet, [0.0; 3], 15.0, [0.6, 0.9, 1.0]);
        effect(c, Kind::Shockwave, 0.0, 1.4, feet, [0.0, 1.0, 0.0], 18.0, [0.55, 0.9, 1.0]);
    }
    sound(host, "minecraft:block.glass.break", feet, 1.5, 0.6, 0);
    sound(host, "minecraft:entity.player.hurt_freeze", feet, 1.0, 0.8, 0);
    sound(host, "minecraft:block.amethyst_block.chime", feet, 2.0, 0.5, 3);
    send(host, Act::Frost, dimension, feet, [0.0, 1.0, 0.0], 1);
}

/// Gravity Well: a black hole where the player looks, for five seconds, then it implodes.
fn gravity(host: &mut dyn Host, dimension: &str, eye: [f64; 3], dir: [f64; 3]) {
    let center = match host.raycast(48.0) {
        Some(hit) => add([hit.x, hit.y, hit.z], dir, -3.0),
        None => add(eye, dir, 18.0),
    };
    let center = [center[0], center[1] + 2.5, center[2]];
    {
        let mut client = state();
        let c = &mut *client;
        effect(c, Kind::Gravity, 0.0, 5.0, center, [0.0; 3], 1.6, [1.0, 0.55, 0.2]);
        effect(c, Kind::Shockwave, 4.85, 1.8, center, [0.0, 1.0, 0.0], 26.0, [0.75, 0.4, 1.0]);
    }
    sound(host, "minecraft:block.portal.trigger", center, 2.0, 0.5, 0);
    sound(host, "minecraft:block.beacon.activate", center, 2.0, 0.5, 0);
    sound(host, "minecraft:entity.warden.sonic_boom", center, 3.0, 0.5, 98);
    sound(host, "minecraft:entity.generic.explode", center, 4.0, 0.6, 100);
    send(host, Act::Gravity, dimension, center, dir, 1);
}

/// Hyper Dash: the player shoots forward, leaving light behind.
fn dash(host: &mut dyn Host, dimension: &str, eye: [f64; 3], dir: [f64; 3]) {
    let push = [dir[0] * 3.4, (dir[1] * 3.4 + 0.35).clamp(-4.0, 4.0), dir[2] * 3.4];
    host.push_player([push[0] as f32, push[1] as f32, push[2] as f32]);
    let end = add(eye, dir, 16.0);
    {
        let mut client = state();
        let c = &mut *client;
        c.dash = Some(Instant::now());
        let behind = add(eye, dir, -1.0);
        effect(c, Kind::Dash, 0.0, 1.4, add(behind, [0.0, -0.4, 0.0], 1.0), add(end, [0.0, -0.4, 0.0], 1.0), 0.0, [1.0, 1.0, 1.0]);
        effect(c, Kind::Shockwave, 0.0, 0.9, behind, dir, 7.0, [0.6, 1.0, 0.85]);
    }
    sound(host, "minecraft:entity.breeze.wind_burst", eye, 1.5, 0.7, 0);
    sound(host, "minecraft:item.trident.riptide_3", eye, 1.2, 1.2, 0);
    send(host, Act::Dash, dimension, eye, dir, 1);
}

/// Each effect's push on the camera this frame, added to the other mods'.
pub fn shake(shake: &mut NativeShake) {
    let client = state();
    let now = Instant::now();
    let mut amount = 0.0f32;
    for e in &client.effects {
        let age = e.age(now);
        if age < 0.0 || age > e.duration {
            continue;
        }
        let t = age / e.duration;
        amount += match e.kind {
            Kind::Punch => 3.5 * (1.0 - t).powi(3),
            Kind::Shockwave => e.size / 30.0 * (1.0 - t).powi(3),
            Kind::Lightning => 1.0 * (1.0 - t).powi(2),
            Kind::Gravity => 0.35 + 0.6 * t * t,
            Kind::Dash => 1.2 * (1.0 - t).powi(4),
            Kind::Frost | Kind::Laser => 0.0,
        };
    }
    if client.lasers && client.beam.is_some_and(|b| b.hit) {
        amount += 0.05 * f32::from(client.laser_power);
    }
    if amount <= 0.0 {
        return;
    }
    let amount = amount.min(5.0);
    let t = now.duration_since(*EPOCH.get_or_init(Instant::now)).as_secs_f32();
    let wobble = |f: f32, p: f32| (t * f + p).sin() * 0.6 + (t * f * 2.31 + p * 1.7).sin() * 0.4;
    shake.pitch += amount * wobble(47.0, 0.0);
    shake.yaw += amount * wobble(41.0, 1.3) * 0.8;
    shake.roll += amount * wobble(37.0, 2.1) * 0.5;
}

/// The field of view: a dash widens it, then it eases back.
pub fn fov(degrees: f32) -> f32 {
    let client = state();
    let Some(dash) = client.dash else {
        return degrees;
    };
    let t = dash.elapsed().as_secs_f32();
    if t > 1.2 {
        return degrees;
    }
    let kick = if t < 0.12 { t / 0.12 } else { (1.0 - (t - 0.12) / 1.08).powi(2) };
    (degrees + 28.0 * kick).clamp(1.0, 170.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_matches_vanilla() {
        let mut camera = NativeCamera {
            position: [0.0; 3], eye_height: 1.62, partial_tick: 0.0, yaw: 0.0, pitch: 0.0,
            fov_degrees: 70.0, near: 0.05, far: 512.0, view: [0.0; 16], projection: [0.0; 16],
            dimension: "minecraft:overworld".into(),
        };
        // yaw 0 looks south (+z), 90 west (-x); pitch 90 looks down
        let d = look(&camera);
        assert!((d[2] - 1.0).abs() < 1e-9);
        camera.yaw = 90.0;
        assert!((look(&camera)[0] + 1.0).abs() < 1e-9);
        camera.pitch = 90.0;
        assert!((look(&camera)[1] + 1.0).abs() < 1e-9);
    }

    #[test]
    fn every_power_has_its_own_key() {
        let mut keys: Vec<&str> = POWERS.iter().map(|p| p.default).collect();
        keys.push(PANEL.2);
        let count = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }
}
