//! The two items, on the window's side.
//!
//! - The Storm Scepter (a breeze rod): right click calls the chosen event down wherever the
//!   cursor points, as far as the world is drawn. Hold the flight key (R) while holding it to
//!   choose: scroll, or A and D, then let go.
//! - The Gravity Gun (an echo shard): right click doubles the cube it picks, 1, 2, 4, 8 blocks
//!   across, then back to 1; left click lifts the cube out of the world, left click again
//!   flings it where you look; right click while holding lets it fall.

use crate::client::{self, Client, Effect, Kind, POWERS, FLIGHT};
use crate::flight::{self, V3};
use crate::net::Act;
use crate::weather::{self, EVENTS};
use crate::ID;
use horizon_client::mod_api::{Host, NativeCamera};
use std::time::{Duration, Instant};

pub const SCEPTER: &str = "minecraft:breeze_rod";
pub const GUN: &str = "minecraft:echo_shard";
const SCEPTER_REACH: f32 = 1024.0;
const GUN_REACH: f32 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holding {
    Nothing,
    Scepter,
    Gun,
}

/// An event called down, as the window draws and hears it.
#[derive(Clone, Debug)]
pub struct Storm {
    pub event: usize,
    pub at: V3,
    /// A tsunami's way (towards the caller), and how far it rolls.
    pub dir: V3,
    pub run: f64,
    pub seed: u32,
    pub start: Instant,
    pub seconds: f32,
}

impl Storm {
    pub fn age(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    /// Where it is now: a tornado moves, the rest stay.
    pub fn now_at(&self) -> V3 {
        match weather::ef(self.event) {
            Some(ef) => {
                let off = weather::tornado_offset(self.seed, ef, f64::from(self.age()));
                [self.at[0] + off[0], self.at[1], self.at[2] + off[1]]
            }
            None => self.at,
        }
    }

    /// 0 to 1 to 0 over its life.
    pub fn envelope(&self, rise: f32, fall: f32) -> f32 {
        let t = self.age();
        let s = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        s(t / rise) * s((self.seconds - t) / fall)
    }
}

pub struct Items {
    pub holding: Holding,
    pub selected: usize,
    /// The Storm Scepter's picker, open while the flight key is held.
    pub picker: bool,
    /// Where the scepter points (up to 1024 blocks), and how far that is.
    pub aim: Option<(V3, f64)>,
    pub ready: [Option<Instant>; 13],
    pub storms: Vec<Storm>,
    /// The Gravity Gun's cube size, and the cube it would pick (its lowest corner).
    pub size: u8,
    pub selection: Option<V3>,
    /// The cube the gun holds: its size, since when.
    pub gripping: Option<(u8, Instant)>,
    /// A line shown for a moment (a refused call, a cube grown).
    pub toast: Option<(String, Instant)>,
}

impl Items {
    pub const fn new() -> Self {
        Self {
            holding: Holding::Nothing,
            selected: 0,
            picker: false,
            aim: None,
            ready: [None; 13],
            storms: Vec::new(),
            size: 1,
            selection: None,
            gripping: None,
            toast: None,
        }
    }
}

fn holding(host: &dyn Host) -> Holding {
    match host.held() {
        Some((item, _, _)) if item == SCEPTER => Holding::Scepter,
        Some((item, _, _)) if item == GUN => Holding::Gun,
        _ => Holding::Nothing,
    }
}

fn toast(c: &mut Client, text: impl Into<String>) {
    c.items.toast = Some((text.into(), Instant::now()));
}

/// Each tick: what is held, and the storms' sounds and flashes.
pub fn tick(host: &mut dyn Host, camera: &NativeCamera) {
    let now_holding = holding(host);
    let (let_go, storms, ticks) = {
        let mut c = client::state();
        let was = c.items.holding;
        c.items.holding = now_holding;
        if now_holding != Holding::Scepter {
            c.items.picker = false;
            c.items.aim = None;
        }
        if now_holding != Holding::Gun {
            c.items.selection = None;
        }
        c.items.storms.retain(|s| s.age() < s.seconds + 0.5);
        let let_go = was == Holding::Gun && now_holding != Holding::Gun && c.items.gripping.take().is_some();
        (let_go, c.items.storms.clone(), c.ticks())
    };
    if let_go {
        let dir = client::look(camera);
        client::send(host, Act::Drop, &camera.dimension, camera.position, dir, 1);
    }
    for storm in &storms {
        ambience(host, camera, storm, ticks);
    }
}

/// A storm's ongoing sounds, and its flashes of lightning.
fn ambience(host: &mut dyn Host, camera: &NativeCamera, s: &Storm, ticks: u64) {
    let at = s.now_at();
    let age = s.age();
    let eye = camera.position;
    let far = flight::length(flight::sub(at, eye));
    let env = s.envelope(2.0, 3.0);
    let mut c = client::state();
    match s.event {
        e if weather::ef(e).is_some() => {
            let ef = f32::from(weather::ef(e).unwrap_or(1));
            drop(c);
            if ticks % 20 == 0 && env > 0.05 {
                client::sound(host, "minecraft:item.elytra.flying", at, 3.0 + ef, 0.35 + 0.03 * ef, 0);
                client::sound(host, "minecraft:weather.rain", eye, 0.4 * env, 0.6, 0);
            }
            if ticks % 33 == 0 && env > 0.05 {
                client::sound(host, "minecraft:entity.breeze.wind_burst", at, 2.0 + ef, 0.5, 0);
            }
            if ef >= 4.0 && ticks % 45 == 7 {
                let mut c = client::state();
                let a = client::rand(&mut c) * std::f64::consts::TAU;
                let d = 20.0 + client::rand(&mut c) * 40.0;
                let bolt = [at[0] + a.cos() * d, at[1], at[2] + a.sin() * d];
                client::effect(&mut c, Kind::Lightning, 0.0, 0.7, bolt, [0.0; 3], 140.0, [0.75, 0.8, 1.0]);
                drop(c);
                client::sound(host, "minecraft:entity.lightning_bolt.thunder", bolt, 4.0, 0.8, (far / 17.0) as i32);
            }
        }
        weather::HURRICANE | weather::FLOOD => {
            let hurricane = s.event == weather::HURRICANE;
            let every = if hurricane { 18 } else { 40 };
            let strike = env > 0.3 && ticks % every == 3;
            let mut bolt = at;
            if strike {
                let a = client::rand(&mut c) * std::f64::consts::TAU;
                let d = client::rand(&mut c).sqrt() * if hurricane { 140.0 } else { 50.0 };
                bolt = [at[0] + a.cos() * d, at[1], at[2] + a.sin() * d];
                client::effect(&mut c, Kind::Lightning, 0.0, 0.8, bolt, [0.0; 3], 170.0, [0.7, 0.8, 1.0]);
            }
            if !hurricane && ticks % 30 == 0 && age > 1.5 {
                let a = client::rand(&mut c) * std::f64::consts::TAU;
                let d = client::rand(&mut c).sqrt() * 40.0;
                let level = f64::from(((age - 1.5) / 2.0).floor().min(8.0));
                let ripple = [at[0] + a.cos() * d, at[1] + level + 0.9, at[2] + a.sin() * d];
                client::effect(&mut c, Kind::Shockwave, 0.0, 1.4, ripple, [0.0, 1.0, 0.0], 6.0, [0.5, 0.7, 1.0]);
            }
            drop(c);
            if ticks % 30 == 0 && env > 0.05 && far < 220.0 {
                client::sound(host, "minecraft:weather.rain", eye, 1.0 * env, 0.8, 0);
                client::sound(host, "minecraft:weather.rain.above", eye, 0.6 * env, 0.8, 0);
            }
            if hurricane && ticks % 25 == 0 {
                client::sound(host, "minecraft:item.elytra.flying", eye, 0.8 * env, 0.4, 0);
            }
            if strike {
                let delay = (flight::length(flight::sub(bolt, eye)) / 17.0) as i32;
                client::sound(host, "minecraft:entity.lightning_bolt.thunder", bolt, 6.0, 0.7, delay);
            }
        }
        weather::TSUNAMI => {
            drop(c);
            let front = flight::add(s.at, flight::scale(s.dir, (weather::WAVE_SPEED * f64::from(age)).min(s.run)));
            if ticks % 10 == 0 && age < s.seconds - 2.0 {
                client::sound(host, "minecraft:block.water.ambient", front, 6.0, 0.4, 0);
                client::sound(host, "minecraft:entity.player.splash.high_speed", front, 4.0, 0.5, 0);
            }
            let crash = (s.run / weather::WAVE_SPEED) as f32;
            if (age - crash).abs() < 0.026 {
                client::sound(host, "minecraft:entity.generic.explode", front, 6.0, 0.5, 0);
                client::sound(host, "minecraft:entity.generic.splash", front, 6.0, 0.4, 0);
                let mut c = client::state();
                client::effect(&mut c, Kind::Shockwave, 0.0, 2.0, front, [0.0, 1.0, 0.0], 40.0, [0.7, 0.9, 1.0]);
            }
        }
        weather::EARTHQUAKE => {
            if ticks % 30 == 0 && age < 15.0 {
                client::effect(&mut c, Kind::Shockwave, 0.0, 2.0, at, [0.0, 1.0, 0.0], 90.0, [0.9, 0.7, 0.45]);
            }
            drop(c);
            if ticks % 12 == 0 && age < 15.0 {
                client::sound(host, "minecraft:entity.warden.dig", at, 6.0, 0.5, 0);
                client::sound(host, "minecraft:block.basalt.break", eye, 1.0, 0.5, 0);
            }
            if ticks % 40 == 0 && age < 14.0 {
                client::sound(host, "minecraft:entity.ravager.roar", at, 6.0, 0.3, 0);
            }
        }
        weather::VOLCANO => {
            let erupting = f64::from(age) > weather::VOLCANO_RISE + 0.5 && age < s.seconds - 2.0;
            let top = [at[0], at[1] + weather::VOLCANO_HEIGHT * 0.8, at[2]];
            if erupting && ticks % 40 == 11 {
                let a = client::rand(&mut c) * std::f64::consts::TAU;
                let bolt = [at[0] + a.cos() * 25.0, at[1] + 60.0, at[2] + a.sin() * 25.0];
                client::effect(&mut c, Kind::Lightning, 0.0, 0.6, bolt, [0.0; 3], 90.0, [1.0, 0.75, 0.6]);
            }
            drop(c);
            if f64::from(age) < weather::VOLCANO_RISE && ticks % 10 == 0 {
                client::sound(host, "minecraft:entity.warden.emerge", at, 6.0, 0.5, 0);
                client::sound(host, "minecraft:block.stone.break", at, 4.0, 0.5, 0);
            }
            if erupting && ticks % 9 == 0 {
                client::sound(host, "minecraft:block.lava.pop", top, 5.0, 0.6, 0);
                client::sound(host, "minecraft:entity.blaze.shoot", top, 4.0, 0.5, 0);
            }
            if erupting && ticks % 30 == 0 {
                client::sound(host, "minecraft:entity.generic.explode", top, 5.0, 0.4, 0);
            }
        }
        weather::SOLAR_FLARE => {
            drop(c);
            if ticks % 15 == 0 && age < 7.0 {
                client::sound(host, "minecraft:block.fire.ambient", at, 6.0, 0.6, 0);
                client::sound(host, "minecraft:block.beacon.ambient", at, 4.0, 0.5, 0);
            }
        }
        _ => {}
    }
}

/// Each frame: where the scepter points, and which cube the gun would pick.
pub fn frame(host: &mut dyn Host) {
    let (holding, gripping, size) = {
        let c = client::state();
        (c.items.holding, c.items.gripping.is_some(), c.items.size)
    };
    match holding {
        Holding::Scepter => {
            let eye = host.camera().map(|c| c.position);
            let aim = host.raycast(SCEPTER_REACH).map(|hit| [hit.x, hit.y, hit.z]);
            let aim = aim.zip(eye).map(|(at, eye)| (at, flight::length(flight::sub(at, eye))));
            client::state().items.aim = aim;
        }
        Holding::Gun if !gripping => {
            let selection = host.raycast(GUN_REACH).map(|hit| cube(&hit, size));
            client::state().items.selection = selection;
        }
        _ => {}
    }
}

/// The lowest corner of the cube `size` blocks across that the gun picks at `hit`: the block hit
/// and the ones behind it, centred across the face.
fn cube(hit: &horizon_client::mod_api::NativeHit, size: u8) -> V3 {
    let n = f64::from(size);
    let block = [f64::from(hit.block_x), f64::from(hit.block_y), f64::from(hit.block_z)];
    // which face: the axis where the hit point sits on the block's edge
    let local = [hit.x - block[0], hit.y - block[1], hit.z - block[2]];
    let mut corner = [0.0; 3];
    let mut face = 3;
    let mut best = 0.02;
    for i in 0..3 {
        for (edge, out) in [(local[i], -1.0), (1.0 - local[i], 1.0)] {
            if edge.abs() < best {
                best = edge.abs();
                face = i * 2 + usize::from(out > 0.0);
            }
        }
    }
    for i in 0..3 {
        let centred = block[i] - ((n - 1.0) / 2.0).floor();
        corner[i] = centred;
        if face / 2 == i {
            // reach into the block from the face the ray came through
            corner[i] = if face % 2 == 1 { block[i] - (n - 1.0) } else { block[i] };
        }
    }
    corner
}

/// The keys while an item is held; true when the key is the item's.
pub fn key(host: &mut dyn Host, key: &str, pressed: bool) -> bool {
    let held = holding(host);
    let flight_key = host.mod_key(ID, POWERS[FLIGHT].key, POWERS[FLIGHT].default);
    let picker = client::state().items.picker;
    if key == flight_key {
        if held == Holding::Scepter || picker {
            let mut c = client::state();
            c.items.picker = pressed && held == Holding::Scepter;
            drop(c);
            if pressed {
                if let Some(camera) = host.camera() {
                    client::sound(host, "minecraft:ui.button.click", camera.position, 0.5, 1.4, 0);
                }
            }
            return true;
        }
        return false;
    }
    if !pressed || client::screen_is_open() {
        return false;
    }
    if picker {
        let step = match key {
            "key.keyboard.a" | "key.keyboard.left" | "key.keyboard.w" | "key.keyboard.up" => Some(EVENTS.len() - 1),
            "key.keyboard.d" | "key.keyboard.right" | "key.keyboard.s" | "key.keyboard.down" => Some(1),
            _ => None,
        };
        if let Some(step) = step {
            choose(host, step);
            return true;
        }
    }
    match (held, key) {
        (Holding::Scepter, "key.mouse.right") => {
            summon(host);
            true
        }
        (Holding::Gun, "key.mouse.right") => {
            gun_right(host);
            true
        }
        (Holding::Gun, "key.mouse.left") => {
            gun_left(host);
            true
        }
        _ => false,
    }
}

/// The mouse wheel while the picker is open; true when it chose.
pub fn scroll(host: &mut dyn Host, dy: f64) -> bool {
    if !client::state().items.picker || dy == 0.0 {
        return false;
    }
    choose(host, if dy > 0.0 { EVENTS.len() - 1 } else { 1 });
    true
}

fn choose(host: &mut dyn Host, step: usize) {
    let selected = {
        let mut c = client::state();
        c.items.selected = (c.items.selected + step) % EVENTS.len();
        c.items.selected
    };
    if let Some(camera) = host.camera() {
        client::sound(host, "minecraft:block.note_block.hat", camera.position, 0.6, 0.8 + 0.08 * selected as f32, 0);
    }
}

/// Whether the use and attack buttons belong to a held item now (so the game doesn't use them).
pub fn claims(button: i32) -> bool {
    let c = client::state();
    match c.items.holding {
        Holding::Scepter => button == 1,
        Holding::Gun => button == 0 || button == 1,
        Holding::Nothing => false,
    }
}

// ---- the Storm Scepter --------------------------------------------------------------------------

fn summon(host: &mut dyn Host) {
    let Some(camera) = host.camera() else {
        return;
    };
    let eye = camera.position;
    let (event, aim) = {
        let c = client::state();
        (c.items.selected, c.items.aim)
    };
    let Some((at, distance)) = aim.or_else(|| {
        host.raycast(SCEPTER_REACH).map(|h| ([h.x, h.y, h.z], flight::length(flight::sub([h.x, h.y, h.z], eye))))
    }) else {
        client::sound(host, "minecraft:block.dispenser.fail", eye, 0.8, 1.2, 0);
        toast(&mut client::state(), "No ground there");
        return;
    };
    {
        let mut c = client::state();
        let now = Instant::now();
        if let Some(left) = c.items.ready[event].filter(|r| now < *r).map(|r| r - now) {
            toast(&mut c, format!("{} again in {:.1}s", EVENTS[event].label, left.as_secs_f32()));
            drop(c);
            client::sound(host, "minecraft:block.dispenser.fail", eye, 0.6, 1.0, 0);
            return;
        }
        c.items.ready[event] = Some(now + Duration::from_secs_f32(EVENTS[event].cooldown));
    }
    let toward = {
        let d = [eye[0] - at[0], 0.0, eye[2] - at[2]];
        let l = d[0].hypot(d[2]);
        if l > 1e-3 { [d[0] / l, 0.0, d[2] / l] } else { [1.0, 0.0, 0.0] }
    };
    let flat = (eye[0] - at[0]).hypot(eye[2] - at[2]);
    let storm = Storm {
        event,
        at,
        dir: toward,
        run: weather::wave_run(flat),
        seed: weather::seed_of(at, event),
        start: Instant::now(),
        seconds: EVENTS[event].seconds,
    };
    begin(host, &storm, eye, distance);
    client::state().items.storms.push(storm);
    client::send_value(host, Act::Weather, &camera.dimension, at, toward, event as u8, flat);
    client::sound(host, "minecraft:entity.breeze.charge", eye, 1.0, 0.6, 0);
    client::sound(host, "minecraft:block.beacon.activate", eye, 0.8, 0.7, 0);
}

/// What an event looks and sounds like when it starts.
fn begin(host: &mut dyn Host, s: &Storm, eye: V3, distance: f64) {
    let at = s.at;
    let up = [0.0, 1.0, 0.0];
    let delay = (distance / 17.0) as i32;
    let mut c = client::state();
    let c = &mut *c;
    let add = |c: &mut Client, e: Effect| c.push_effect(e);
    let base = |kind: Kind, delay: f32, duration: f32, at: V3, to: V3, size: f32, color: [f32; 3]| Effect::new(kind, delay, duration, at, to, size, color);
    match s.event {
        e if weather::ef(e).is_some() => {
            let ef = weather::ef(e).unwrap_or(1);
            let r = weather::tornado_radius(ef) as f32;
            let height = 90.0 + 18.0 * f32::from(ef);
            let mut t = base(Kind::Tornado, 0.0, s.seconds, at, [f64::from(height), f64::from(r * 3.2 + 10.0), f64::from(ef)], r, [0.42, 0.43, 0.45]);
            t.path = s.seed;
            add(c, t);
            add(c, base(Kind::Hurricane, 0.0, s.seconds, at, [f64::from(height), 16.0, 0.0], (r * 9.0).max(70.0), [0.3, 0.31, 0.34]));
            client::sound(host, "minecraft:entity.breeze.wind_burst", at, 6.0, 0.4, 0);
            client::sound(host, "minecraft:entity.lightning_bolt.thunder", at, 4.0, 0.6, delay);
        }
        weather::HURRICANE => {
            let mut h = base(Kind::Hurricane, 0.0, s.seconds, at, [75.0, 34.0, 16.0], 150.0, [0.34, 0.36, 0.4]);
            h.q_w = 1.0;
            add(c, h);
            client::sound(host, "minecraft:entity.breeze.wind_burst", eye, 2.0, 0.3, 0);
            client::sound(host, "minecraft:entity.lightning_bolt.thunder", at, 8.0, 0.5, delay);
        }
        weather::TSUNAMI => {
            let mut w = base(Kind::Tsunami, 0.0, s.seconds, at, s.dir, s.run as f32, [0.05, 0.32, 0.38]);
            w.c_w = weather::WAVE_WIDTH as f32;
            add(c, w);
            add(c, base(Kind::Shockwave, 0.0, 2.5, at, up, 60.0, [0.6, 0.85, 1.0]));
            client::sound(host, "minecraft:ambient.underwater.enter", eye, 1.0, 0.5, 0);
            client::sound(host, "minecraft:entity.generic.splash", at, 8.0, 0.3, delay);
        }
        weather::FLOOD => {
            add(c, base(Kind::Hurricane, 0.0, s.seconds, at, [55.0, 22.0, 0.0], 80.0, [0.3, 0.32, 0.36]));
            client::sound(host, "minecraft:weather.rain.above", eye, 1.0, 0.7, 0);
            client::sound(host, "minecraft:entity.lightning_bolt.thunder", at, 5.0, 0.7, delay);
        }
        weather::SOLAR_FLARE => {
            add(c, base(Kind::Flare, 0.0, s.seconds, at, up, 12.0, [1.0, 0.75, 0.3]));
            add(c, base(Kind::Orb, 0.0, 2.5, at, up, 60.0, [1.0, 0.85, 0.5]));
            add(c, base(Kind::Shockwave, 0.6, 3.5, at, up, 80.0, [1.0, 0.6, 0.2]));
            client::sound(host, "minecraft:block.beacon.activate", at, 8.0, 0.5, 0);
            client::sound(host, "minecraft:entity.blaze.shoot", at, 8.0, 0.4, delay);
            client::sound(host, "minecraft:item.firecharge.use", eye, 1.0, 0.5, delay);
        }
        weather::NUKE => {
            add(c, base(Kind::Orb, 0.0, 1.6, at, up, 120.0, [1.0, 0.95, 0.85]));
            add(c, base(Kind::Orb, 0.05, 9.0, flight::add(at, [0.0, 16.0, 0.0]), up, 40.0, [1.0, 0.5, 0.15]));
            add(c, base(Kind::Shockwave, 0.1, 5.0, at, up, 220.0, [1.0, 0.65, 0.3]));
            add(c, base(Kind::Shockwave, 0.3, 2.2, flight::add(at, [0.0, 10.0, 0.0]), up, 90.0, [1.0, 1.0, 1.0]));
            add(c, base(Kind::Mushroom, 0.0, s.seconds, at, up, 42.0, [0.42, 0.37, 0.33]));
            client::sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.25, delay);
            client::sound(host, "minecraft:entity.warden.sonic_boom", eye, 1.0, 0.3, delay);
            client::sound(host, "minecraft:entity.lightning_bolt.thunder", eye, 1.0, 0.3, delay + 3);
            client::sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.2, delay + 14);
            client::sound(host, "minecraft:entity.wither.spawn", eye, 0.6, 0.3, delay + 20);
        }
        weather::METEOR => {
            let from = weather::meteor_from(s.seed);
            let fall = weather::METEOR_SECONDS as f32;
            add(c, base(Kind::Meteor, 0.0, fall, at, from, 3.0, [1.0, 0.55, 0.2]));
            add(c, base(Kind::Orb, fall, 2.2, at, up, 55.0, [1.0, 0.7, 0.35]));
            add(c, base(Kind::Shockwave, fall, 3.0, at, up, 90.0, [1.0, 0.55, 0.25]));
            let mut plume = base(Kind::Smoke, fall, 18.0, at, [70.0, 26.0, 3.0], 10.0, [0.32, 0.28, 0.26]);
            plume.q_w = 0.6;
            plume.c_w = 0.8;
            add(c, plume);
            let land = (fall * 20.0) as i32;
            let start = flight::add(at, flight::scale(from, weather::METEOR_PATH * 0.6));
            client::sound(host, "minecraft:entity.firework_rocket.large_blast_far", start, 8.0, 0.4, 0);
            client::sound(host, "minecraft:entity.blaze.shoot", eye, 1.0, 0.3, 10);
            client::sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.35, land + delay);
            client::sound(host, "minecraft:entity.warden.sonic_boom", eye, 1.0, 0.45, land + delay);
            client::sound(host, "minecraft:entity.lightning_bolt.thunder", eye, 1.0, 0.4, land + delay + 3);
        }
        weather::EARTHQUAKE => {
            for (i, (a, length)) in weather::faults(s.seed).into_iter().enumerate() {
                for k in 0..3 {
                    let along = length * (0.2 + 0.3 * k as f64);
                    let p = weather::fault_point(s.seed, i, a, along);
                    let spot = [at[0] + p[0], at[1], at[2] + p[1]];
                    let delay = (along / length * 7.0) as f32;
                    let mut dust = base(Kind::Smoke, delay, 8.0, spot, [20.0, 16.0, 2.0], 9.0, [0.5, 0.42, 0.33]);
                    dust.q_w = if along < length * 0.45 { 0.7 } else { 0.0 };
                    dust.c_w = 0.55;
                    add(c, dust);
                }
            }
            add(c, base(Kind::Shockwave, 0.0, 2.5, at, up, 100.0, [0.9, 0.7, 0.45]));
            client::sound(host, "minecraft:entity.warden.emerge", at, 8.0, 0.4, 0);
            client::sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.3, delay);
        }
        weather::VOLCANO => {
            let rise = weather::VOLCANO_RISE as f32;
            let top = flight::add(at, [0.0, weather::VOLCANO_HEIGHT * 0.82, 0.0]);
            let mut plume = base(Kind::Smoke, rise, s.seconds - rise, top, [190.0, 70.0, 14.0], 7.0, [0.17, 0.15, 0.15]);
            plume.q_w = 1.0;
            plume.c_w = 1.0;
            add(c, plume);
            add(c, base(Kind::Orb, rise, 3.0, top, up, 30.0, [1.0, 0.45, 0.12]));
            add(c, base(Kind::Shockwave, rise, 3.0, top, up, 120.0, [1.0, 0.45, 0.15]));
            add(c, base(Kind::Shockwave, 0.0, 2.0, at, up, 60.0, [0.9, 0.5, 0.25]));
            client::sound(host, "minecraft:entity.warden.emerge", at, 8.0, 0.4, 0);
            client::sound(host, "minecraft:entity.generic.explode", eye, 1.0, 0.3, delay + (rise * 20.0) as i32);
            client::sound(host, "minecraft:entity.ender_dragon.growl", top, 8.0, 0.4, (rise * 20.0) as i32 + 4);
        }
        _ => {}
    }
}

/// How much the storms shake the camera at `eye`.
pub fn shake(c: &Client, eye: V3) -> f32 {
    let mut amount = 0.0f32;
    for s in &c.items.storms {
        let at = s.now_at();
        let d = flight::length(flight::sub(at, eye)) as f32;
        let age = s.age();
        let near = |reach: f32| (1.0 - d / reach).max(0.0);
        amount += match s.event {
            e if weather::ef(e).is_some() => {
                let r = weather::tornado_radius(weather::ef(e).unwrap_or(1)) as f32;
                1.6 * near(r * 8.0 + 30.0) * s.envelope(2.0, 3.0)
            }
            weather::HURRICANE => 0.25 * near(220.0) * s.envelope(4.0, 5.0),
            weather::EARTHQUAKE if age < 15.0 => 4.0 * near(170.0).max(0.25) * s.envelope(0.3, 1.5),
            weather::VOLCANO => {
                let rising = f64::from(age) < weather::VOLCANO_RISE;
                (if rising { 2.2 } else { 0.8 }) * near(160.0)
            }
            weather::TSUNAMI => {
                let front = flight::add(s.at, flight::scale(s.dir, (weather::WAVE_SPEED * f64::from(age)).min(s.run)));
                let d = flight::length(flight::sub(front, eye)) as f32;
                1.2 * (1.0 - d / 90.0).max(0.0)
            }
            weather::NUKE if age < 6.0 => 2.5 * near(260.0) * (1.0 - age / 6.0),
            weather::METEOR => {
                let since = age - weather::METEOR_SECONDS as f32;
                if (0.0..2.5).contains(&since) { 3.5 * near(160.0) * (1.0 - since / 2.5) } else { 0.0 }
            }
            _ => 0.0,
        };
    }
    amount
}

/// What the storms do to the air at `eye`: rain, darkness, heat shimmer, a red-orange cast.
pub fn atmosphere(c: &Client, eye: V3) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for s in &c.items.storms {
        let at = s.now_at();
        let d = ((at[0] - eye[0]).hypot(at[2] - eye[2])) as f32;
        let age = s.age();
        let near = |reach: f32| (1.0 - d / reach).clamp(0.0, 1.0);
        let (rain, dark, heat, tint) = match s.event {
            e if weather::ef(e).is_some() => {
                let k = near(260.0) * s.envelope(2.0, 3.0);
                (0.55 * k, 0.4 * k, 0.0, 0.0)
            }
            weather::HURRICANE => {
                let k = near(300.0) * s.envelope(4.0, 5.0);
                let eye_calm = if d < 16.0 { 0.25 } else { 1.0 };
                (k * eye_calm, 0.55 * k, 0.0, 0.0)
            }
            weather::FLOOD => {
                let k = near(180.0) * s.envelope(1.5, 3.0);
                (0.9 * k, 0.4 * k, 0.0, 0.0)
            }
            weather::SOLAR_FLARE => {
                let k = near(260.0) * s.envelope(0.5, 2.5);
                (0.0, 0.0, k, 0.8 * k)
            }
            weather::NUKE => {
                let k = near(320.0) * (1.0 - age / 14.0).max(0.0);
                (0.0, 0.25 * near(320.0) * s.envelope(6.0, 10.0), 0.7 * k, 0.6 * k)
            }
            weather::VOLCANO if f64::from(age) > weather::VOLCANO_RISE => {
                let k = near(220.0) * s.envelope(1.0, 3.0);
                (0.0, 0.35 * k, 0.35 * k, 0.45 * k)
            }
            _ => (0.0, 0.0, 0.0, 0.0),
        };
        out[0] = out[0].max(rain);
        out[1] = out[1].max(dark);
        out[2] = out[2].max(heat);
        out[3] = out[3].max(tint);
    }
    out
}

// ---- the Gravity Gun ----------------------------------------------------------------------------

fn gun_right(host: &mut dyn Host) {
    let Some(camera) = host.camera() else {
        return;
    };
    let gripping = client::state().items.gripping.is_some();
    if gripping {
        client::state().items.gripping = None;
        client::send(host, Act::Drop, &camera.dimension, camera.position, client::look(&camera), 1);
        client::sound(host, "minecraft:block.beacon.deactivate", camera.position, 0.7, 1.6, 0);
        return;
    }
    let size = {
        let mut c = client::state();
        c.items.size = if c.items.size >= 8 { 1 } else { c.items.size * 2 };
        let size = c.items.size;
        toast(&mut c, format!("Gravity Gun: {size}x{size}x{size}"));
        size
    };
    client::sound(host, "minecraft:block.note_block.chime", camera.position, 0.8, 0.6 + 0.25 * f32::from(size.trailing_zeros() as u8), 0);
    client::sound(host, "minecraft:block.beacon.power_select", camera.position, 0.5, 1.0 + 0.2 * f32::from(size.trailing_zeros() as u8), 0);
}

fn gun_left(host: &mut dyn Host) {
    let Some(camera) = host.camera() else {
        return;
    };
    let eye = camera.position;
    let dir = client::look(&camera);
    let (gripping, selection, size) = {
        let c = client::state();
        (c.items.gripping, c.items.selection, c.items.size)
    };
    if let Some((size, _)) = gripping {
        client::state().items.gripping = None;
        client::send(host, Act::Fling, &camera.dimension, eye, dir, 1);
        let hold = flight::add(eye, flight::scale(dir, crate::server::hold_distance(size)));
        {
            let mut c = client::state();
            let c = &mut *c;
            client::effect(c, Kind::Punch, 0.0, 0.5, flight::add(eye, flight::scale(dir, 0.8)), dir, 10.0 + f32::from(size) * 3.0, [1.0, 0.6, 0.2]);
            client::effect(c, Kind::Shockwave, 0.0, 0.9, hold, dir, 6.0 + f32::from(size) * 2.0, [1.0, 0.6, 0.2]);
        }
        client::sound(host, "minecraft:entity.warden.sonic_boom", eye, 0.8, 1.5, 0);
        client::sound(host, "minecraft:entity.breeze.shoot", eye, 1.0, 0.7, 0);
        return;
    }
    let Some(corner) = selection else {
        client::sound(host, "minecraft:block.dispenser.fail", eye, 0.6, 1.4, 0);
        return;
    };
    client::state().items.gripping = Some((size, Instant::now()));
    client::send(host, Act::Grab, &camera.dimension, corner, dir, size);
    let n = f64::from(size);
    let middle = flight::add(corner, [n / 2.0, n / 2.0, n / 2.0]);
    {
        let mut c = client::state();
        client::effect(&mut c, Kind::Shockwave, 0.0, 0.8, middle, [0.0, 1.0, 0.0], 3.0 + f32::from(size) * 2.0, [1.0, 0.6, 0.2]);
    }
    client::sound(host, "minecraft:entity.enderman.teleport", middle, 1.0, 0.6, 0);
    client::sound(host, "minecraft:block.beacon.activate", eye, 0.7, 1.8, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::mod_api::NativeHit;

    fn hit(block: [i32; 3], at: [f64; 3]) -> NativeHit {
        NativeHit { block_x: block[0], block_y: block[1], block_z: block[2], x: at[0], y: at[1], z: at[2], block: "minecraft:stone".into() }
    }

    #[test]
    fn the_cube_goes_into_the_face_it_was_picked_on() {
        // looking down at the top of block (10, 64, 10)
        let top = hit([10, 64, 10], [10.5, 65.0, 10.5]);
        assert_eq!(cube(&top, 1), [10.0, 64.0, 10.0]);
        assert_eq!(cube(&top, 4), [9.0, 61.0, 9.0]);
        // the west face (low x): the cube reaches east, into the hill
        let west = hit([10, 64, 10], [10.0, 64.5, 10.5]);
        assert_eq!(cube(&west, 2), [10.0, 64.0, 10.0]);
    }
}
