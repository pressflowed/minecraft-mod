//! The window's half: keys, cooldowns, the effects being drawn, Superflight, camera feel,
//! sounds, and what each power asks of the server.

use crate::flight::{self, V3};
use crate::net::{Act, CHANNEL, Use};
use crate::ID;
use crate::host::{Host, NativeCamera, NativeShake};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub struct Power {
    pub key: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    pub cooldown: f32,
    pub color: u32,
    pub mega: bool,
}

pub const PUNCH: usize = 0;
pub const LASERS: usize = 1;
pub const THUNDER: usize = 2;
pub const FROST: usize = 3;
pub const GRAVITY: usize = 4;
pub const DASH: usize = 5;
pub const FLIGHT: usize = 6;
pub const TITAN: usize = 7;
pub const OMEGA: usize = 8;
pub const WRATH: usize = 9;
pub const ZERO: usize = 10;
pub const SINGULARITY: usize = 11;
pub const CATACLYSM: usize = 12;

pub const POWERS: [Power; 13] = [
    Power { key: "punch", label: "Super Punch", default: "key.keyboard.z", cooldown: 1.2, color: 0xFFB347, mega: false },
    Power { key: "lasers", label: "Laser Eyes", default: "key.keyboard.g", cooldown: 0.25, color: 0xFF3B3B, mega: false },
    Power { key: "thunder", label: "Thunder Strike", default: "key.keyboard.v", cooldown: 3.0, color: 0x7FB8FF, mega: false },
    Power { key: "frost", label: "Frost Nova", default: "key.keyboard.b", cooldown: 4.0, color: 0x9FF3FF, mega: false },
    Power { key: "gravity", label: "Gravity Well", default: "key.keyboard.n", cooldown: 8.0, color: 0xC17BFF, mega: false },
    Power { key: "dash", label: "Hyper Dash", default: "key.keyboard.j", cooldown: 1.0, color: 0x7CFFB2, mega: false },
    Power { key: "flight", label: "Superflight", default: "key.keyboard.r", cooldown: 0.3, color: 0xFFFFFF, mega: false },
    Power { key: "titan_fist", label: "Titan Fist", default: "key.keyboard.y", cooldown: 8.0, color: 0xFF8A2A, mega: true },
    Power { key: "omega_beam", label: "Omega Beam", default: "key.keyboard.u", cooldown: 10.0, color: 0xFF2A6A, mega: true },
    Power { key: "wrath", label: "Wrath of the Gods", default: "key.keyboard.i", cooldown: 12.0, color: 0x9AC8FF, mega: true },
    Power { key: "absolute_zero", label: "Absolute Zero", default: "key.keyboard.o", cooldown: 14.0, color: 0xBFF8FF, mega: true },
    Power { key: "singularity", label: "Singularity", default: "key.keyboard.k", cooldown: 20.0, color: 0xB060FF, mega: true },
    Power { key: "cataclysm", label: "CATACLYSM (hold)", default: "key.keyboard.m", cooldown: 30.0, color: 0xFF4020, mega: true },
];

pub const PANEL: (&str, &str, &str) = ("panel", "Powers panel", "key.keyboard.h");
pub const LOOP: (&str, &str, &str) = ("loop", "Superflight: loop", "key.keyboard.x");

/// Ticks the Cataclysm's key must be held.
pub const CHARGE_TICKS: u32 = 40;
/// The Cataclysm's crater: 20 chunks each way, 40 across.
pub const CATACLYSM_RADIUS: f64 = 320.0;

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
    Orb = 8,
    Tornado = 9,
    Hurricane = 10,
    Tsunami = 11,
    Mushroom = 12,
    Smoke = 13,
    Flare = 14,
    Meteor = 15,
    Box = 16,
    Tether = 17,
    Marker = 18,
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub kind: Kind,
    pub start: Instant,
    pub duration: f32,
    pub at: V3,
    pub to: V3,
    pub size: f32,
    pub color: [f32; 3],
    pub seed: f32,
    /// A tornado's path seed (0 for everything else).
    pub path: u32,
    /// What goes in the shader's `q.w` and `c.w` (0 and 1 unless an effect says).
    pub q_w: f32,
    pub c_w: f32,
}

impl Effect {
    #[allow(clippy::too_many_arguments)]
    pub fn new(kind: Kind, delay: f32, duration: f32, at: V3, to: V3, size: f32, color: [f32; 3]) -> Self {
        let start = Instant::now() + Duration::from_secs_f32(delay.max(0.0));
        let seed = ((at[0] * 0.731 + at[2] * 1.37).rem_euclid(97.0)) as f32;
        Self { kind, start, duration, at, to, size, color, seed, path: 0, q_w: 0.0, c_w: 1.0 }
    }

    pub fn age(&self, now: Instant) -> f32 {
        if now >= self.start {
            now.duration_since(self.start).as_secs_f32()
        } else {
            -self.start.duration_since(now).as_secs_f32()
        }
    }
}

/// The laser eyes (power 1 to 5) or the Omega Beam (power 12) this frame.
#[derive(Clone, Copy, Debug)]
pub struct Beam {
    pub from: V3,
    pub to: V3,
    pub hit: bool,
    pub power: u8,
}

/// Superflight: the visual roll and pitch the camera adds, and the manoeuvres in progress.
///
/// Everything the camera shows is kept for this tick and the last, and drawn between the two by
/// the frame's partial tick, so it is smooth at any frame rate. The flight path itself has
/// momentum: it swings round toward the view instead of snapping to it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flying {
    /// Degrees, right wing down positive: the bank from A/D and from turning (now, last tick).
    pub bank: f64,
    pub prev_bank: f64,
    /// A barrel roll: ticks into it (0 when none, now and last tick), and which way.
    pub barrel: f64,
    pub prev_barrel: f64,
    pub barrel_dir: f64,
    /// A loop: ticks into it (0 when none, now and last tick).
    pub looping: f64,
    pub prev_looping: f64,
    pub forward: f64,
    pub strafe: f64,
    pub lift: f64,
    pub boost: bool,
    /// What was pushed last tick, to work out this tick's push.
    pub velocity: V3,
    /// Where the flight is heading (a unit vector; zero before the first tick).
    pub dir: V3,
    pub speed: f64,
    pub prev_speed: f64,
}

/// How long a barrel roll and a loop take, in ticks.
pub const BARREL_TICKS: f64 = 16.0;
pub const LOOP_TICKS: f64 = 30.0;

/// Eases a manoeuvre in and out (0 to 1).
fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Degrees turned `ticks` into a manoeuvre `total` ticks long.
fn turned(ticks: f64, total: f64) -> f64 {
    if ticks <= 0.0 { 0.0 } else { 360.0 * ease(ticks / total) }
}

/// A manoeuvre's degrees between last tick and now: one that just ended is drawn to its end.
fn between(prev: f64, now: f64, total: f64, partial: f64) -> f64 {
    if prev <= 0.0 && now <= 0.0 {
        return 0.0;
    }
    let now = if now <= 0.0 { total } else { now };
    turned(prev + (now - prev) * partial, total)
}

impl Flying {
    /// The camera's roll this frame: the bank and any barrel roll.
    pub fn roll(&self, partial: f64) -> f64 {
        let bank = self.prev_bank + (self.bank - self.prev_bank) * partial;
        bank + between(self.prev_barrel, self.barrel, BARREL_TICKS, partial) * self.barrel_dir
    }

    /// How far round a loop the camera is this frame, in degrees.
    pub fn loop_angle(&self, partial: f64) -> f64 {
        between(self.prev_looping, self.looping, LOOP_TICKS, partial)
    }

    /// The speed this frame, in blocks a tick.
    pub fn speed_at(&self, partial: f64) -> f64 {
        self.prev_speed + (self.speed - self.prev_speed) * partial
    }
}

pub struct Client {
    pub panel: bool,
    pub lasers: bool,
    pub laser_power: u8,
    pub beam: Option<Beam>,
    pub omega_until: Option<Instant>,
    pub omega: Option<Beam>,
    pub ready: [Option<Instant>; 13],
    pub effects: Vec<Effect>,
    pub dash: Option<Instant>,
    pub dimension: String,
    pub flying: Option<Flying>,
    /// The Cataclysm's key held, and for how many ticks; where it is aimed.
    pub charge_held: bool,
    pub charge: u32,
    pub charge_at: Option<V3>,
    pub tapped: [Option<Instant>; 2],
    /// The Storm Scepter and the Gravity Gun.
    pub items: crate::items::Items,
    /// Where the camera was last tick.
    pub eye: V3,
    ticks: u64,
    seed: u32,
}

impl Client {
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn push_effect(&mut self, e: Effect) {
        if self.effects.len() >= 160 {
            self.effects.remove(0);
        }
        self.effects.push(e);
    }
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
    flying: None,
    charge_held: false,
    charge: 0,
    charge_at: None,
    tapped: [None, None],
    items: crate::items::Items::new(),
    eye: [0.0; 3],
    ticks: 0,
    seed: 0x9E37_79B9,
});

static EPOCH: OnceLock<Instant> = OnceLock::new();
static SCREEN: AtomicBool = AtomicBool::new(false);
pub static DAY_TIME: AtomicI64 = AtomicI64::new(6000);
/// No fall damage for the player until then (while flying, and a while after).
static SAFE_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

pub fn state() -> MutexGuard<'static, Client> {
    CLIENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn lasers_on() -> bool {
    state().lasers
}

pub fn screen_open(open: bool) {
    SCREEN.store(open, Ordering::Relaxed);
}

pub fn screen_is_open() -> bool {
    SCREEN.load(Ordering::Relaxed)
}

/// Whether a fall should hurt the player now (single player: the client and the server share it).
pub fn fall_protected() -> bool {
    SAFE_UNTIL
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_some_and(|until| Instant::now() < until)
}

pub(crate) fn protect_falls(seconds: f32) {
    let until = Instant::now() + Duration::from_secs_f32(seconds);
    let mut safe = SAFE_UNTIL.lock().unwrap_or_else(|p| p.into_inner());
    if safe.is_none_or(|s| s < until) {
        *safe = Some(until);
    }
}

/// Every key the mod registers: (id, label, default key).
pub fn all_keys() -> Vec<(&'static str, &'static str, &'static str)> {
    let mut keys: Vec<_> = POWERS.iter().map(|p| (p.key, p.label, p.default)).collect();
    keys.extend([PANEL, LOOP]);
    keys.extend(crate::jet::KEYS);
    keys
}

/// Which of the mod's keys were held at the last poll.
static HELD: Mutex<Vec<bool>> = Mutex::new(Vec::new());

/// Each tick: the mod's own keys, as presses and releases under `superpowers:<key>` (a tap
/// shorter than a tick still counts, from its click).
pub fn poll_keys(host: &mut Host<'_>) {
    let keys = all_keys();
    let mut events = Vec::new();
    {
        let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
        held.resize(keys.len(), false);
        for (i, (key, _, _)) in keys.iter().enumerate() {
            let mut clicks = 0;
            while clicks < 16 && host.consume_click(key) {
                clicks += 1;
            }
            let down = host.is_down(key);
            let name = format!("{ID}:{key}");
            match (held[i], down) {
                (false, true) => events.push((name, true)),
                (true, false) => events.push((name, false)),
                (false, false) if clicks > 0 => {
                    events.push((name.clone(), true));
                    events.push((name, false));
                }
                _ => {}
            }
            held[i] = down;
        }
    }
    for (name, pressed) in events {
        key(host, &name, pressed);
    }
}

pub fn key_label(host: &Host<'_>, key: &str, default: &str) -> String {
    let bound = host.bound_key(ID, key, default);
    match bound.as_str() {
        "key.mouse.left" => "LMB".to_owned(),
        "key.mouse.right" => "RMB".to_owned(),
        "key.mouse.middle" => "MMB".to_owned(),
        _ => bound.rsplit('.').next().unwrap_or("?").to_ascii_uppercase(),
    }
}

pub fn look(camera: &NativeCamera) -> V3 {
    flight::look(f64::from(camera.yaw), f64::from(camera.pitch))
}

pub(crate) fn rand(c: &mut Client) -> f64 {
    let mut x = c.seed;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    c.seed = x;
    f64::from(x >> 8) / f64::from(1u32 << 24)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn effect(c: &mut Client, kind: Kind, delay: f32, duration: f32, at: V3, to: V3, size: f32, color: [f32; 3]) {
    let mut e = Effect::new(kind, delay, duration, at, to, size, color);
    e.seed = rand(c) as f32 * 100.0;
    c.push_effect(e);
}

pub(crate) fn send(host: &mut Host<'_>, act: Act, dimension: &str, at: V3, dir: V3, level: u8) {
    send_value(host, act, dimension, at, dir, level, 0.0);
}

pub(crate) fn send_value(host: &mut Host<'_>, act: Act, dimension: &str, at: V3, dir: V3, level: u8, value: f64) {
    let message = Use { act, dimension: dimension.to_owned(), at, dir, level, value };
    let _ = host.net_send_to_server(CHANNEL, &message.encode());
}

pub(crate) fn sound(host: &mut Host<'_>, name: &str, at: V3, volume: f32, pitch: f32, delay: i32) {
    let _ = host.play_local_sound(name, at, volume, pitch, delay.clamp(0, 1200));
}

fn target(host: &Host<'_>, eye: V3, dir: V3, reach: f32, otherwise: f64) -> (V3, bool) {
    match host.raycast(reach) {
        Some(hit) => ([hit.x, hit.y, hit.z], true),
        None => (flight::add(eye, flight::scale(dir, otherwise)), false),
    }
}

pub fn tick(host: &mut Host<'_>, camera: NativeCamera) {
    DAY_TIME.store(host.day_time(), Ordering::Relaxed);
    let (lasers, power, beam, omega, dimension, ticks) = {
        let mut c = state();
        c.ticks += 1;
        c.dimension = camera.dimension.clone();
        c.eye = camera.position;
        let now = Instant::now();
        c.effects.retain(|e| e.age(now) < e.duration + 0.1);
        let omega = c.omega_until.is_some_and(|t| now < t);
        if !omega {
            c.omega_until = None;
            c.omega = None;
        }
        (c.lasers, c.laser_power, c.beam, omega, c.dimension.clone(), c.ticks)
    };
    let eye = camera.position;
    let dir = look(&camera);
    if lasers {
        let every = u64::from(7 - power.min(5)).max(1);
        if let Some(beam) = beam.filter(|b| b.hit) {
            if ticks % every == 0 {
                send(host, Act::Laser, &dimension, beam.to, dir, power);
            }
            if ticks % 4 == 0 {
                sound(host, "minecraft:block.fire.extinguish", beam.to, 0.5, 1.6, 0);
            }
        }
        if ticks % 30 == 0 {
            sound(host, "minecraft:block.beacon.ambient", eye, 0.9, 0.6 + 0.25 * f32::from(power), 0);
        }
    }
    if omega {
        if ticks % 2 == 0 {
            send(host, Act::Omega, &dimension, eye, dir, 5);
        }
        if ticks % 6 == 0 {
            let (to, _) = target(host, eye, dir, 256.0, 256.0);
            sound(host, "minecraft:entity.generic.explode", to, 2.0, 0.8, 0);
            let mut c = state();
            effect(&mut c, Kind::Shockwave, 0.0, 0.8, to, flight::scale(dir, -1.0), 10.0, [1.0, 0.3, 0.5]);
        }
        if ticks % 12 == 0 {
            sound(host, "minecraft:block.beacon.ambient", eye, 2.0, 0.5, 0);
        }
    }
    charge_tick(host, &camera);
    flight_tick(host, &camera);
    crate::items::tick(host, &camera);
    crate::jet::tick(host, &camera);
}

/// Each frame: where the beams hit.
pub fn frame(host: &mut Host<'_>) {
    crate::items::frame(host);
    crate::jet::frame(host);
    let (lasers, omega) = {
        let c = state();
        (c.lasers, c.omega_until.is_some())
    };
    if !lasers && !omega {
        return;
    }
    let Some(camera) = host.camera() else {
        return;
    };
    let dir = look(&camera);
    let mut c = state();
    if lasers {
        let (to, hit) = target(host, camera.position, dir, 96.0, 96.0);
        let power = c.laser_power;
        c.beam = Some(Beam { from: camera.position, to, hit, power });
    }
    if omega {
        let (to, hit) = target(host, camera.position, dir, 256.0, 256.0);
        c.omega = Some(Beam { from: camera.position, to, hit, power: 12 });
    }
}

/// A key went down or up; true when a jet or a held item took it.
pub fn key(host: &mut Host<'_>, key: &str, pressed: bool) -> bool {
    // a jet takes its keys first, then a held Storm Scepter or Gravity Gun (R opens the
    // scepter's picker)
    if crate::jet::key(host, key, pressed) || crate::items::key(host, key, pressed) {
        return true;
    }
    // The Cataclysm charges while its key is held.
    if host.mod_key(ID, POWERS[CATACLYSM].key, POWERS[CATACLYSM].default) == key {
        let mut c = state();
        c.charge_held = pressed && !SCREEN.load(Ordering::Relaxed);
        if !pressed {
            c.charge = 0;
            c.charge_at = None;
        }
        return false;
    }
    if !pressed {
        return false;
    }
    // Right click raises the laser eyes' power while they are on: 1 to 5, then back to 1.
    if key == "key.mouse.right" {
        let mut c = state();
        if c.lasers {
            c.laser_power = c.laser_power % 5 + 1;
            let power = c.laser_power;
            drop(c);
            if let Some(camera) = host.camera() {
                sound(host, "minecraft:block.note_block.chime", camera.position, 0.8, 0.5 + 0.25 * f32::from(power), 0);
                sound(host, "minecraft:block.beacon.power_select", camera.position, 0.6, 0.8 + 0.2 * f32::from(power), 0);
            }
        }
    }
    if SCREEN.load(Ordering::Relaxed) {
        return false;
    }
    // A double tap of A or D while flying: a barrel roll that way.
    for (side, name) in [(0usize, "key.keyboard.a"), (1, "key.keyboard.d")] {
        if key == name {
            let mut c = state();
            if c.flying.is_none() {
                break;
            }
            let now = Instant::now();
            let double = c.tapped[side].is_some_and(|t| now.duration_since(t) < Duration::from_millis(280));
            c.tapped[side] = if double { None } else { Some(now) };
            if double {
                if let Some(f) = c.flying.as_mut() {
                    if f.barrel == 0.0 {
                        f.barrel_dir = if side == 1 { 1.0 } else { -1.0 };
                        f.barrel = 0.001;
                        f.prev_barrel = 0.0;
                    }
                }
            }
        }
    }
    if host.mod_key(ID, PANEL.0, PANEL.2) == key {
        let mut c = state();
        c.panel = !c.panel;
    }
    if host.mod_key(ID, LOOP.0, LOOP.2) == key {
        let mut c = state();
        if let Some(f) = c.flying.as_mut() {
            if f.looping == 0.0 {
                f.looping = 0.001;
                f.prev_looping = 0.0;
            }
        }
    }
    for (index, power) in POWERS.iter().enumerate() {
        if index != CATACLYSM && host.mod_key(ID, power.key, power.default) == key {
            activate(host, index);
        }
    }
    false
}

fn ready(index: usize) -> bool {
    let mut c = state();
    let now = Instant::now();
    if c.ready[index].is_some_and(|r| now < r) {
        return false;
    }
    c.ready[index] = Some(now + Duration::from_secs_f32(POWERS[index].cooldown));
    true
}

fn activate(host: &mut Host<'_>, index: usize) {
    let Some(camera) = host.camera() else {
        return;
    };
    if !ready(index) {
        return;
    }
    let eye = camera.position;
    let dir = look(&camera);
    let dimension = camera.dimension.clone();
    match index {
        PUNCH => punch(host, &dimension, eye, dir, 1.0),
        TITAN => punch(host, &dimension, eye, dir, 5.0),
        LASERS => {
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
        OMEGA => {
            state().omega_until = Some(Instant::now() + Duration::from_secs(4));
            sound(host, "minecraft:block.beacon.activate", eye, 2.0, 0.5, 0);
            sound(host, "minecraft:entity.warden.sonic_charge", eye, 2.0, 0.7, 0);
            sound(host, "minecraft:entity.warden.sonic_boom", eye, 2.0, 0.5, 10);
        }
        THUNDER => thunder(host, &dimension, eye, dir, false),
        WRATH => thunder(host, &dimension, eye, dir, true),
        FROST => frost(host, &dimension, eye, false),
        ZERO => frost(host, &dimension, eye, true),
        GRAVITY => gravity(host, &dimension, eye, dir, false),
        SINGULARITY => gravity(host, &dimension, eye, dir, true),
        DASH => dash(host, &dimension, eye, dir),
        FLIGHT => toggle_flight(host, eye),
        _ => {}
    }
}

/// Super Punch, and Titan Fist (`k` 5).
fn punch(host: &mut Host<'_>, dimension: &str, eye: V3, dir: V3, k: f32) {
    let (impact, _) = target(host, eye, dir, 5.0 * k, 3.5);
    let ground = [impact[0], impact[1] - 0.5, impact[2]];
    let fist = flight::add(flight::add(eye, flight::scale(dir, 0.7)), [0.0, -0.25, 0.0]);
    {
        let mut c = state();
        let c = &mut *c;
        effect(c, Kind::Punch, 0.0, 0.45 * k.sqrt(), fist, dir, 16.0 * k, [1.0, 0.72, 0.35]);
        effect(c, Kind::Shockwave, 0.0, 1.6 * k.sqrt(), ground, [0.0, 1.0, 0.0], 34.0 * k, [1.0, 0.6, 0.25]);
        if k > 1.0 {
            effect(c, Kind::Shockwave, 0.25, 2.4, flight::add(eye, flight::scale(dir, 30.0)), dir, 50.0, [1.0, 0.8, 0.5]);
            effect(c, Kind::Orb, 0.1, 1.2, flight::add(eye, flight::scale(dir, 30.0)), dir, 14.0, [1.0, 0.6, 0.25]);
        }
    }
    sound(host, "minecraft:entity.player.attack.knockback", eye, 1.0, 0.6, 0);
    sound(host, "minecraft:entity.generic.explode", impact, 2.0 * k, 0.7 / k.sqrt(), 0);
    sound(host, "minecraft:entity.warden.sonic_boom", impact, 1.5 * k, 0.8 / k.sqrt(), 1);
    if k > 1.0 {
        sound(host, "minecraft:entity.lightning_bolt.thunder", impact, 4.0, 0.5, 4);
    }
    send(host, if k > 1.0 { Act::TitanFist } else { Act::Punch }, dimension, eye, dir, 1);
}

/// Where the bolts of a lightning power fall: the same on the client and the server, from one seed.
pub fn bolts(seed: u8, count: usize, radius: f64, over_ticks: u32) -> Vec<([f64; 2], u32)> {
    let mut x: u32 = 0x9E37_79B9 ^ (u32::from(seed) << 8 | 0x5A);
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        f64::from(x >> 8) / f64::from(1u32 << 24)
    };
    (0..count)
        .map(|i| {
            if i == 0 {
                return ([0.0, 0.0], 0);
            }
            let angle = next() * std::f64::consts::TAU;
            let r = radius * next().sqrt();
            let tick = (next() * f64::from(over_ticks)) as u32;
            ([angle.cos() * r, angle.sin() * r], tick)
        })
        .collect()
}

/// Thunder Strike, or Wrath of the Gods.
fn thunder(host: &mut Host<'_>, dimension: &str, eye: V3, dir: V3, wrath: bool) {
    let (center, _) = target(host, eye, dir, 192.0, 48.0);
    let seed = (rand(&mut state()) * 255.0) as u8;
    let (count, radius, ticks) = if wrath { (36, 46.0, 70) } else { (7, 8.0, 18) };
    {
        let mut c = state();
        let c = &mut *c;
        for (offset, tick) in bolts(seed, count, radius, ticks) {
            let at = [center[0] + offset[0], center[1], center[2] + offset[1]];
            let delay = tick as f32 / 20.0;
            effect(c, Kind::Lightning, delay, 0.9, at, [0.0; 3], 160.0, [0.55, 0.72, 1.0]);
        }
        effect(c, Kind::Shockwave, 0.0, 1.4, center, [0.0, 1.0, 0.0], if wrath { 90.0 } else { 20.0 }, [0.45, 0.65, 1.0]);
        if wrath {
            effect(c, Kind::Shockwave, 0.5, 3.0, center, [0.0, 1.0, 0.0], 60.0, [0.5, 0.6, 1.0]);
        }
    }
    sound(host, "minecraft:entity.lightning_bolt.thunder", center, if wrath { 10.0 } else { 4.0 }, 0.8, 0);
    if wrath {
        for k in 1..6 {
            sound(host, "minecraft:entity.lightning_bolt.thunder", center, 8.0, 0.6 + 0.1 * k as f32, k * 12);
        }
    }
    send(host, if wrath { Act::Wrath } else { Act::Thunder }, dimension, center, dir, seed);
}

/// Frost Nova, or Absolute Zero.
fn frost(host: &mut Host<'_>, dimension: &str, eye: V3, absolute: bool) {
    let feet = [eye[0], eye[1] - 1.62, eye[2]];
    let r = if absolute { 70.0 } else { 15.0 };
    {
        let mut c = state();
        let c = &mut *c;
        effect(c, Kind::Frost, 0.0, if absolute { 8.0 } else { 3.5 }, feet, [0.0; 3], r, [0.6, 0.9, 1.0]);
        effect(c, Kind::Shockwave, 0.0, if absolute { 3.0 } else { 1.4 }, feet, [0.0, 1.0, 0.0], r * 1.2, [0.55, 0.9, 1.0]);
        if absolute {
            effect(c, Kind::Shockwave, 0.6, 3.0, feet, [0.0, 1.0, 0.0], 110.0, [0.8, 0.95, 1.0]);
        }
    }
    sound(host, "minecraft:block.glass.break", feet, 1.5, 0.6, 0);
    sound(host, "minecraft:entity.player.hurt_freeze", feet, 1.0, 0.8, 0);
    sound(host, "minecraft:block.amethyst_block.chime", feet, 2.0, 0.5, 3);
    if absolute {
        sound(host, "minecraft:entity.elder_guardian.curse", feet, 1.0, 0.5, 0);
    }
    send(host, if absolute { Act::AbsoluteZero } else { Act::Frost }, dimension, feet, [0.0, 1.0, 0.0], 1);
}

/// Gravity Well, or the Singularity.
fn gravity(host: &mut Host<'_>, dimension: &str, eye: V3, dir: V3, singularity: bool) {
    let (reach, back, up) = if singularity { (160.0, 20.0, 18.0) } else { (48.0, 3.0, 2.5) };
    let (hit, found) = target(host, eye, dir, reach, if singularity { 70.0 } else { 18.0 });
    let center = if found { flight::add(hit, flight::scale(dir, -back)) } else { hit };
    let center = [center[0], center[1] + up, center[2]];
    let (rs, dur, wave) = if singularity { (7.0, 8.0, 130.0) } else { (1.6, 5.0, 26.0) };
    {
        let mut c = state();
        let c = &mut *c;
        effect(c, Kind::Gravity, 0.0, dur, center, [0.0; 3], rs, [1.0, 0.55, 0.2]);
        effect(c, Kind::Shockwave, dur - 0.15, 1.8, center, [0.0, 1.0, 0.0], wave, [0.75, 0.4, 1.0]);
        effect(c, Kind::Orb, dur - 0.15, 0.8, center, [0.0; 3], rs * 3.0, [0.9, 0.7, 1.0]);
    }
    let end = (dur * 20.0) as i32;
    sound(host, "minecraft:block.portal.trigger", center, if singularity { 6.0 } else { 2.0 }, 0.5, 0);
    sound(host, "minecraft:block.beacon.activate", center, 2.0, 0.5, 0);
    sound(host, "minecraft:entity.warden.sonic_boom", center, if singularity { 8.0 } else { 3.0 }, 0.5, end - 2);
    sound(host, "minecraft:entity.generic.explode", center, if singularity { 10.0 } else { 4.0 }, 0.5, end);
    send(host, if singularity { Act::Singularity } else { Act::Gravity }, dimension, center, dir, 1);
}

/// Hyper Dash.
fn dash(host: &mut Host<'_>, dimension: &str, eye: V3, dir: V3) {
    let push = [dir[0] * 3.4, (dir[1] * 3.4 + 0.35).clamp(-4.0, 4.0), dir[2] * 3.4];
    host.push_player([push[0] as f32, push[1] as f32, push[2] as f32]);
    let end = flight::add(eye, flight::scale(dir, 16.0));
    {
        let mut c = state();
        let c = &mut *c;
        c.dash = Some(Instant::now());
        let behind = flight::add(eye, flight::scale(dir, -1.0));
        let low = [0.0, -0.4, 0.0];
        effect(c, Kind::Dash, 0.0, 1.4, flight::add(behind, low), flight::add(end, low), 0.0, [1.0, 1.0, 1.0]);
        effect(c, Kind::Shockwave, 0.0, 0.9, behind, dir, 7.0, [0.6, 1.0, 0.85]);
    }
    protect_falls(4.0);
    sound(host, "minecraft:entity.breeze.wind_burst", eye, 1.5, 0.7, 0);
    sound(host, "minecraft:item.trident.riptide_3", eye, 1.2, 1.2, 0);
    send(host, Act::Dash, dimension, eye, dir, 1);
}

/// The Cataclysm: hold its key for two seconds; letting go calls it off.
fn charge_tick(host: &mut Host<'_>, camera: &NativeCamera) {
    let (held, ready_at) = {
        let c = state();
        (c.charge_held, c.ready[CATACLYSM])
    };
    if !held || ready_at.is_some_and(|r| Instant::now() < r) {
        return;
    }
    let eye = camera.position;
    let dir = look(camera);
    let (charge, aim) = {
        let mut c = state();
        c.charge += 1;
        (c.charge, c.charge_at)
    };
    let center = match aim {
        Some(at) => at,
        None => {
            let (hit, _) = target(host, eye, dir, 1024.0, 200.0);
            let mut c = state();
            c.charge_at = Some(hit);
            effect(&mut c, Kind::Orb, 0.0, CHARGE_TICKS as f32 / 20.0 + 0.3, hit, [0.0; 3], 30.0, [1.0, 0.35, 0.1]);
            effect(&mut c, Kind::Shockwave, 0.0, CHARGE_TICKS as f32 / 20.0, hit, [0.0, 1.0, 0.0], 60.0, [1.0, 0.35, 0.1]);
            hit
        }
    };
    if charge % 10 == 1 {
        sound(host, "minecraft:block.beacon.ambient", eye, 2.0, 0.5 + charge as f32 / 40.0, 0);
        sound(host, "minecraft:entity.warden.heartbeat", center, 8.0, 0.6, 0);
    }
    if charge < CHARGE_TICKS || !ready(CATACLYSM) {
        return;
    }
    {
        let mut c = state();
        c.charge = 0;
        c.charge_at = None;
        c.charge_held = false;
    }
    cataclysm(host, &camera.dimension, eye, center);
}

fn cataclysm(host: &mut Host<'_>, dimension: &str, eye: V3, center: V3) {
    let r = CATACLYSM_RADIUS as f32;
    {
        let mut c = state();
        let c = &mut *c;
        let up = [0.0, 1.0, 0.0];
        effect(c, Kind::Orb, 0.0, 6.0, center, up, 90.0, [1.0, 0.7, 0.35]);
        effect(c, Kind::Orb, 0.4, 7.0, flight::add(center, [0.0, 120.0, 0.0]), up, 110.0, [1.0, 0.45, 0.15]);
        effect(c, Kind::Shockwave, 0.0, 4.5, center, up, r, [1.0, 0.5, 0.15]);
        effect(c, Kind::Shockwave, 0.8, 5.0, center, up, r * 1.35, [1.0, 0.3, 0.1]);
        effect(c, Kind::Shockwave, 1.6, 5.0, flight::add(center, [0.0, 60.0, 0.0]), up, r * 0.8, [1.0, 0.85, 0.6]);
        for k in 0..12 {
            let a = k as f64 / 12.0 * std::f64::consts::TAU + rand(c) * 0.3;
            let d = 60.0 + rand(c) * (f64::from(r) - 80.0);
            let at = [center[0] + a.cos() * d, center[1], center[2] + a.sin() * d];
            let delay = 0.2 + rand(c) as f32 * 3.0;
            effect(c, Kind::Lightning, delay, 0.8, at, [0.0; 3], 220.0, [1.0, 0.6, 0.35]);
        }
    }
    protect_falls(30.0);
    // the blast is heard when its shock arrives: sound covers about 17 blocks a tick
    let distance = flight::length(flight::sub(center, eye));
    let delay = (distance / 17.0) as i32;
    sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.3, delay);
    sound(host, "minecraft:entity.warden.sonic_boom", eye, 1.0, 0.4, delay);
    sound(host, "minecraft:entity.lightning_bolt.thunder", eye, 1.0, 0.4, delay + 4);
    sound(host, "minecraft:entity.ender_dragon.growl", eye, 1.0, 0.5, delay + 10);
    sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.25, delay + 20);
    let dir = flight::normalize(flight::sub(center, eye));
    send(host, Act::Cataclysm, dimension, center, dir, 1);
}

fn toggle_flight(host: &mut Host<'_>, eye: V3) {
    let on = {
        let mut c = state();
        if c.flying.take().is_some() {
            false
        } else {
            c.flying = Some(Flying::default());
            true
        }
    };
    protect_falls(10.0);
    if on {
        sound(host, "minecraft:item.elytra.flying", eye, 0.8, 1.2, 0);
        sound(host, "minecraft:entity.breeze.wind_burst", eye, 1.0, 1.4, 0);
        let mut c = state();
        effect(&mut c, Kind::Shockwave, 0.0, 0.8, [eye[0], eye[1] - 1.6, eye[2]], [0.0, 1.0, 0.0], 9.0, [0.8, 0.95, 1.0]);
    } else {
        sound(host, "minecraft:entity.player.attack.sweep", eye, 1.0, 0.7, 0);
    }
}

/// Superflight, each tick: fly where the player looks (the camera's view), the flight path
/// swinging round to it with some momentum. A loop carries the flight up and over; a barrel roll
/// swings it round the line of flight.
fn flight_tick(host: &mut Host<'_>, camera: &NativeCamera) {
    let actual = host.player_velocity();
    let push = {
        let mut c = state();
        let Some(f) = c.flying.as_mut() else {
            return;
        };
        f.prev_bank = f.bank;
        f.prev_barrel = f.barrel;
        f.prev_looping = f.looping;
        f.prev_speed = f.speed;
        let look = look(camera);
        let right = flight::normalize(flight::cross(look, [0.0, 1.0, 0.0]));
        let mut heading = look;
        if f.looping > 0.0 {
            f.looping += 1.0;
            // the line of flight pitches up and over with the camera
            heading = flight::rotate(look, right, turned(f.looping, LOOP_TICKS).to_radians());
            if f.looping >= LOOP_TICKS {
                f.looping = 0.0;
            }
        }
        let mut sideways = [0.0; 3];
        if f.barrel > 0.0 {
            f.barrel += 1.0;
            let a = turned(f.barrel, BARREL_TICKS).to_radians();
            // round the line of flight: a corkscrew
            let up = flight::cross(right, look);
            sideways = flight::add(flight::scale(right, a.sin() * 0.6 * f.barrel_dir), flight::scale(up, (1.0 - a.cos()) * 0.3));
            if f.barrel >= BARREL_TICKS {
                f.barrel = 0.0;
            }
        }
        // momentum: the path turns toward the view, harder in a loop, softer at full boost
        let was = if flight::length(f.dir) < 0.5 { look } else { f.dir };
        let grip = if f.looping > 0.0 { 0.7 } else if f.boost { 0.2 } else { 0.32 };
        let dir = flight::normalize(flight::add(was, flight::scale(flight::sub(heading, was), grip)));
        f.dir = dir;
        // bank into the turn the path makes, and with A/D
        let yaw_of = |d: V3| (-d[0]).atan2(d[2]).to_degrees();
        let turn = ((yaw_of(dir) - yaw_of(was) + 180.0).rem_euclid(360.0)) - 180.0;
        let turn = if dir[1].abs() > 0.97 || was[1].abs() > 0.97 { 0.0 } else { turn };
        let bank_target = (turn * 4.0).clamp(-55.0, 55.0) + f.strafe * 35.0;
        f.bank += (bank_target - f.bank) * 0.18;
        let speed = if f.forward < 0.0 {
            0.15
        } else if f.boost {
            3.0
        } else {
            1.1 * (0.6 + 0.4 * f.forward.max(0.0))
        };
        f.speed += (speed - f.speed) * 0.12;
        let target = flight::add(
            flight::add(flight::scale(dir, f.speed), flight::scale(right, f.strafe * 0.3)),
            flight::add([0.0, f.lift * 0.6, 0.0], sideways),
        );
        // The game moves the player itself, with drag and gravity after our push: aim the
        // velocity it has now (after a wall or the ground stopped it, too) at the new one.
        let now = actual.unwrap_or([f.velocity[0] * 0.91, (f.velocity[1] - 0.08) * 0.98, f.velocity[2] * 0.91]);
        let push = flight::sub(target, now).map(|v| v.clamp(-4.0, 4.0));
        f.velocity = target;
        push
    };
    protect_falls(10.0);
    host.push_player([push[0] as f32, push[1] as f32, push[2] as f32]);
    if state().ticks % 20 == 0 {
        let speed = state().flying.map_or(0.0, |f| f.speed) as f32;
        sound(host, "minecraft:item.elytra.flying", camera.position, (0.2 + speed * 0.25).min(1.0), 0.8 + speed * 0.15, 0);
    }
}

/// The movement keys while flying steer the flight, not the feet.
pub fn movement(forward: &mut f32, strafe: &mut f32, jump: &mut bool, sneak: &mut bool, sprinting: bool) {
    if crate::jet::movement(forward, strafe, jump, sneak, sprinting) {
        return;
    }
    let mut c = state();
    if c.items.picker {
        // A and D choose an event while the Storm Scepter's picker is open
        *forward = 0.0;
        *strafe = 0.0;
    }
    let Some(f) = c.flying.as_mut() else {
        return;
    };
    f.forward = f64::from(*forward).clamp(-1.0, 1.0);
    // vanilla's strafe is positive to the left
    f.strafe = -f64::from(*strafe).clamp(-1.0, 1.0);
    f.lift = f64::from(u8::from(*jump)) - f64::from(u8::from(*sneak));
    f.boost = sprinting && *forward > 0.0;
    *forward = 0.0;
    *strafe = 0.0;
    *jump = false;
    *sneak = false;
}

/// Each effect's push on the camera, plus Superflight's bank, barrel roll and loop, drawn
/// `partial` of the way from last tick to this one.
pub fn shake(shake: &mut NativeShake, partial: f32) {
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
            Kind::Punch => 3.0 * (e.size / 16.0).sqrt() * (1.0 - t).powi(3),
            Kind::Shockwave => (e.size / 30.0).min(4.0) * (1.0 - t).powi(3),
            Kind::Lightning => 0.6 * (1.0 - t).powi(2),
            Kind::Gravity => 0.3 + e.size * 0.08 + 0.6 * t * t,
            Kind::Dash => 1.2 * (1.0 - t).powi(4),
            Kind::Orb => if e.size > 50.0 { 3.0 * (1.0 - t).powi(2) } else { 0.0 },
            Kind::Frost | Kind::Laser => 0.0,
            _ => 0.0,
        };
    }
    amount += crate::items::shake(&c, c.eye);
    drop(c);
    crate::jet::shake(shake);
    let c = state();
    if c.lasers && c.beam.is_some_and(|b| b.hit) {
        amount += 0.05 * f32::from(c.laser_power);
    }
    if c.omega.is_some() {
        amount += 1.4;
    }
    if c.charge > 0 {
        amount += 0.4 + 1.2 * c.charge as f32 / CHARGE_TICKS as f32;
    }
    // the roll: the shake's share of it (the picture turns the rest, see `gfx::split_roll`)
    shake.roll += crate::gfx::split_roll(roll(partial)).0;
    if let Some(f) = c.flying {
        if f.boost {
            amount += 0.15;
        }
    }
    if amount > 0.0 {
        let amount = amount.min(6.0);
        let t = now.duration_since(*EPOCH.get_or_init(Instant::now)).as_secs_f32();
        let wobble = |freq: f32, p: f32| (t * freq + p).sin() * 0.6 + (t * freq * 2.31 + p * 1.7).sin() * 0.4;
        shake.pitch += amount * wobble(47.0, 0.0);
        shake.yaw += amount * wobble(41.0, 1.3) * 0.8;
        shake.roll += amount * wobble(37.0, 2.1) * 0.5;
    }
}

/// The camera's whole roll this frame, degrees (right wing down positive): Superflight's bank
/// and barrel rolls, or a jet's.
pub fn roll(partial: f32) -> f32 {
    if let Some(r) = crate::jet::roll(partial) {
        return r;
    }
    let partial = f64::from(partial.clamp(0.0, 1.0));
    state().flying.map_or(0.0, |f| f.roll(partial) as f32)
}

/// Superflight's loop: the camera pitches up, over and round (negative pitch looks up). The
/// camera's pitch has no limit, so the loop is a whole one.
pub fn camera_pose(partial: f32, pose: &mut ([f64; 3], f32, f32)) -> bool {
    let partial = f64::from(partial.clamp(0.0, 1.0));
    let Some(angle) = state().flying.map(|f| f.loop_angle(partial)).filter(|a| *a != 0.0) else {
        return false;
    };
    pose.2 -= angle as f32;
    true
}

/// The field of view: a dash and flying fast widen it.
pub fn fov(degrees: f32) -> f32 {
    let degrees = crate::jet::fov(degrees);
    let c = state();
    let mut out = degrees;
    if let Some(dash) = c.dash {
        let t = dash.elapsed().as_secs_f32();
        if t < 1.2 {
            let kick = if t < 0.12 { t / 0.12 } else { (1.0 - (t - 0.12) / 1.08).powi(2) };
            out += 28.0 * kick;
        }
    }
    if let Some(f) = c.flying {
        let partial = f64::from(crate::jet::partial());
        out += (f.speed_at(partial) as f32 * 6.0).min(22.0);
    }
    out.clamp(1.0, 170.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_power_has_its_own_key() {
        let mut keys: Vec<&str> = POWERS.iter().map(|p| p.default).collect();
        keys.push(PANEL.2);
        keys.push(LOOP.2);
        keys.extend(crate::jet::KEYS.iter().map(|k| k.2));
        let count = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), count);
        assert!(CATACLYSM_RADIUS * 2.0 / 16.0 >= 38.0);
    }

    #[test]
    fn superflight_rolls_and_loops_glide_between_ticks() {
        // a barrel roll and a loop, ticked as `flight_tick` ticks them and drawn at 8 frames a
        // tick: every frame moves on a little, none jumps
        for (total, step) in [(BARREL_TICKS, 0usize), (LOOP_TICKS, 1)] {
            let mut f = Flying::default();
            let mut ticks = 0.001;
            let mut last = 0.0f64;
            let mut biggest = 0.0f64;
            for _ in 0..(total as usize + 3) {
                let prev = ticks;
                if ticks > 0.0 {
                    ticks += 1.0;
                    if ticks >= total {
                        ticks = 0.0;
                    }
                }
                if step == 0 {
                    (f.prev_barrel, f.barrel, f.barrel_dir) = (prev, ticks, 1.0);
                } else {
                    (f.prev_looping, f.looping) = (prev, ticks);
                }
                for frame in 0..8 {
                    let partial = f64::from(frame) / 8.0;
                    let angle = if step == 0 { f.roll(partial) } else { f.loop_angle(partial) };
                    // 360 and 0 are the same turn
                    let delta = (angle - last + 180.0).rem_euclid(360.0) - 180.0;
                    biggest = biggest.max(delta.abs());
                    last = angle;
                }
            }
            assert!(biggest < 360.0 / total * 0.35, "a frame jumped {biggest:.1} degrees");
        }
    }

    #[test]
    fn bolts_are_the_same_both_sides() {
        assert_eq!(bolts(7, 36, 46.0, 70), bolts(7, 36, 46.0, 70));
        assert!(bolts(7, 36, 46.0, 70).iter().all(|(o, t)| o[0].hypot(o[1]) <= 46.0 && *t < 70));
    }
}
