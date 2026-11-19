//! Jets, on the window's side: getting in and choosing one, the controls in both modes, the
//! flight model stepped five times a tick against the real terrain, keeping the player in the
//! seat, the weapons, the camera (cockpit or chase), the effects and sounds, and what the
//! instruments, the heads-up display and the screens are fed.

use crate::client::{self, Kind};
use crate::flight;
use crate::gfx::{self, Draw};
use crate::jet_mesh;
use crate::jet_net::{self, Down, Shot, Up};
use crate::jet_sim::{self, Crash, Fcs, Ground, Mode, Pilot, Q, Sim, V, part, specs, v};
use crate::ID;
use crate::host::{Host, NativeCamera, NativeShake};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

pub const JET_KEY: (&str, &str, &str) = ("jet", "Jets: get in/out (hold to choose)", "key.keyboard.p");
pub const COBRA_KEY: (&str, &str, &str) = ("cobra", "Jet: Cobra (Talon)", "key.keyboard.c");
pub const MODE_KEY: (&str, &str, &str) = ("jet_mode", "Jet: mouse aim / direct", "key.keyboard.semicolon");
pub const ASSIST_KEY: (&str, &str, &str) = ("jet_assist", "Jet: flight assists", "key.keyboard.apostrophe");
pub const KEYS: [(&str, &str, &str); 4] = [JET_KEY, COBRA_KEY, MODE_KEY, ASSIST_KEY];

/// Ticks the jet key is held before the picker opens.
const HOLD_TICKS: u32 = 6;
const STEPS: usize = 5;
const MAX_ROUNDS: u32 = 500;
const MUZZLE_SPEED: f64 = 420.0;

#[derive(Clone, Copy, Debug, Default)]
struct Inputs {
    forward: f64,
    strafe: f64,
    jump: bool,
    sneak: bool,
    sprint: bool,
    fire: bool,
    cobra: bool,
}

pub struct Jet {
    pub sim: Sim,
    prev: (V, Q),
    fcs: Fcs,
    pub mode: Mode,
    pub assist: bool,
    pub throttle: f64,
    inp: Inputs,
    frozen: Option<V>,
    moved: V,
    pub rounds: u32,
    pub missiles: u32,
    pub bombs: u32,
    reload_missile: u32,
    reload_bomb: u32,
    carpet: u32,
    bay_until: u64,
    /// The lock: 0 none, 1 locking, 2 locked, and where.
    pub lock: (u8, V),
    pub radar: f64,
    /// Where a bomb would land, from the jet.
    pub impact: Option<V>,
    pub pull_up: bool,
    trails: [VecDeque<(V, Instant)>; 3],
    mach_was: f64,
    last_boom: u64,
    gear_was: f64,
    pub stick: (f64, f64),
    ticks: u64,
}

struct Tracer {
    from: V,
    vel: V,
    start: Instant,
    life: f32,
}

pub struct Jets {
    pub jet: Option<Jet>,
    pub selected: usize,
    pub picker: bool,
    held: Option<u32>,
    detached: Option<Instant>,
    pub chase: f32,
    third: bool,
    partial: f32,
    look: V,
    camera: [f64; 3],
    you: Option<(V, u64)>,
    shots: Vec<(Shot, Instant)>,
    smoke: Vec<(u32, VecDeque<(V, Instant)>)>,
    tracers: Vec<Tracer>,
    ticks: u64,
    dimension: String,
}

static JETS: Mutex<Jets> = Mutex::new(Jets {
    jet: None,
    selected: 0,
    picker: false,
    held: None,
    detached: None,
    chase: 4.0,
    third: false,
    partial: 0.0,
    look: V { x: 0.0, y: 0.0, z: 1.0 },
    camera: [0.0; 3],
    you: None,
    shots: Vec::new(),
    smoke: Vec::new(),
    tracers: Vec::new(),
    ticks: 0,
    dimension: String::new(),
});

pub fn state() -> MutexGuard<'static, Jets> {
    JETS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Whether the camera is behind the player (F5), as the camera hook last said.
fn third_person(j: &Jets) -> bool {
    j.third
}

fn send(host: &mut Host<'_>, up: Up) {
    let _ = host.net_send_to_server(jet_net::CHANNEL, &up.encode());
}

fn vec(a: [f64; 3]) -> V {
    V::from(a)
}

// ---- the world under the jet ------------------------------------------------------------------------

/// The terrain as the flight model asks about it, read from the window's copy of the world.
struct WorldGround<'a, 'h> {
    host: &'a Host<'h>,
    cache: HashMap<[i32; 3], (Vec<[f64; 6]>, bool)>,
}

impl<'a, 'h> WorldGround<'a, 'h> {
    fn new(host: &'a Host<'h>) -> Self {
        Self { host, cache: HashMap::new() }
    }

    fn cell(&mut self, p: [i32; 3]) -> &(Vec<[f64; 6]>, bool) {
        let host = self.host;
        self.cache.entry(p).or_insert_with(|| {
            let boxes = host.block_collision(p);
            let water = boxes.is_empty() && host.block(p).contains("water");
            (boxes, water)
        })
    }

    /// How far down the ground is from `p`, up to `reach`.
    fn radar(&mut self, p: V, reach: i32) -> f64 {
        let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
        let top = p.y.floor() as i32;
        for y in (top - reach..=top).rev() {
            let (boxes, water) = self.cell([x, y, z]).clone();
            if water {
                return p.y - (f64::from(y) + 0.88);
            }
            if let Some(h) = boxes.iter().map(|b| b[4]).reduce(f64::max) {
                return p.y - (f64::from(y) + h);
            }
        }
        f64::from(reach)
    }
}

impl Ground for WorldGround<'_, '_> {
    fn height(&mut self, x: f64, y: f64, z: f64) -> Option<f64> {
        let (bx, bz) = (x.floor() as i32, z.floor() as i32);
        let top = y.floor() as i32;
        for by in (top - 5..=top).rev() {
            let (boxes, water) = self.cell([bx, by, bz]).clone();
            if water {
                return Some(f64::from(by) + 0.88);
            }
            if let Some(h) = boxes.iter().map(|b| b[4]).reduce(f64::max) {
                return Some(f64::from(by) + h);
            }
        }
        None
    }

    fn solid(&mut self, x: f64, y: f64, z: f64) -> bool {
        let p = [x.floor() as i32, y.floor() as i32, z.floor() as i32];
        let (lx, ly, lz) = (x - f64::from(p[0]), y - f64::from(p[1]), z - f64::from(p[2]));
        let (boxes, water) = self.cell(p).clone();
        water || boxes.iter().any(|b| lx >= b[0] && lx <= b[3] && ly >= b[1] && ly <= b[4] && lz >= b[2] && lz <= b[5])
    }
}

// ---- keys ---------------------------------------------------------------------------------------

/// The keys: the jet key always, the rest while flying. True when the key was the jets'.
pub fn key(host: &mut Host<'_>, key: &str, pressed: bool) -> bool {
    let bound = |k: (&str, &str, &str)| host.mod_key(ID, k.0, k.2);
    let (jet_key, cobra, mode, assist) = (bound(JET_KEY), bound(COBRA_KEY), bound(MODE_KEY), bound(ASSIST_KEY));
    if key == jet_key {
        if client::screen_is_open() && pressed {
            return true;
        }
        let mut j = state();
        if pressed {
            j.held = Some(0);
        } else {
            let opened = j.picker;
            j.held = None;
            j.picker = false;
            drop(j);
            if !opened {
                toggle(host);
            } else if let Some(camera) = host.camera() {
                client::sound(host, "minecraft:ui.button.click", camera.position, 0.6, 1.2, 0);
            }
        }
        return true;
    }
    let in_jet = state().jet.is_some();
    let picker = state().picker;
    if picker && pressed {
        let step = match key {
            "key.keyboard.a" | "key.keyboard.left" | "key.keyboard.w" | "key.keyboard.up" => Some(jet_sim::COUNT - 1),
            "key.keyboard.d" | "key.keyboard.right" | "key.keyboard.s" | "key.keyboard.down" => Some(1),
            _ => None,
        };
        if let Some(step) = step {
            choose(host, step);
            return true;
        }
    }
    if !in_jet {
        return false;
    }
    let mut j = state();
    let Some(jet) = j.jet.as_mut() else {
        return false;
    };
    match key {
        "key.mouse.left" => {
            jet.inp.fire = pressed;
            return true;
        }
        "key.mouse.right" => {
            if pressed {
                drop(j);
                fire_missile(host);
            }
            return true;
        }
        "key.mouse.middle" => {
            if pressed {
                drop(j);
                drop_bomb(host);
            }
            return true;
        }
        _ => {}
    }
    if key == cobra {
        jet.inp.cobra = pressed;
        return true;
    }
    if pressed && key == mode {
        jet.mode = if jet.mode == Mode::MouseAim { Mode::Direct } else { Mode::MouseAim };
        let m = jet.mode;
        drop(j);
        if let Some(c) = host.camera() {
            client::sound(host, "minecraft:block.note_block.pling", c.position, 0.6, if m == Mode::Direct { 0.8 } else { 1.4 }, 0);
        }
        return true;
    }
    if pressed && key == assist {
        jet.assist = !jet.assist;
        let on = jet.assist;
        drop(j);
        if let Some(c) = host.camera() {
            client::sound(host, "minecraft:block.note_block.pling", c.position, 0.6, if on { 1.6 } else { 0.6 }, 0);
        }
        return true;
    }
    // the flight key and the powers' movement keys do nothing in a jet
    key == host.mod_key(ID, client::POWERS[client::FLIGHT].key, client::POWERS[client::FLIGHT].default)
}

/// The mouse wheel chooses in the picker.
pub fn scroll(host: &mut Host<'_>, dy: f64) -> bool {
    if !state().picker || dy == 0.0 {
        return false;
    }
    choose(host, if dy > 0.0 { jet_sim::COUNT - 1 } else { 1 });
    true
}

fn choose(host: &mut Host<'_>, step: usize) {
    let selected = {
        let mut j = state();
        j.selected = (j.selected + step) % jet_sim::COUNT;
        j.selected
    };
    if let Some(c) = host.camera() {
        client::sound(host, "minecraft:block.note_block.hat", c.position, 0.6, 0.8 + 0.1 * selected as f32, 0);
    }
}

/// The movement keys fly the jet, not the feet. True while flying.
pub fn movement(forward: &mut f32, strafe: &mut f32, jump: &mut bool, sneak: &mut bool, sprint: bool) -> bool {
    let mut j = state();
    let picker = j.picker;
    let Some(jet) = j.jet.as_mut() else {
        if picker {
            *forward = 0.0;
            *strafe = 0.0;
        }
        return false;
    };
    jet.inp.forward = f64::from(*forward);
    jet.inp.strafe = f64::from(*strafe);
    jet.inp.jump = *jump;
    jet.inp.sneak = *sneak;
    jet.inp.sprint = sprint;
    *forward = 0.0;
    *strafe = 0.0;
    *jump = false;
    *sneak = false;
    true
}

/// Whether the mouse buttons and the pick key belong to the jet now.
pub fn claims(_button: i32) -> bool {
    state().jet.is_some()
}

// ---- getting in and out ------------------------------------------------------------------------------

fn toggle(host: &mut Host<'_>) {
    let Some(camera) = host.camera() else {
        return;
    };
    let leaving = state().jet.take();
    if let Some(jet) = leaving {
        leave(host, &camera, &jet, false);
        return;
    }
    board(host, &camera);
}

fn board(host: &mut Host<'_>, camera: &NativeCamera) {
    let (kind, third, chase) = {
        let j = state();
        (j.selected, third_person(&j), j.chase)
    };
    let spec = &specs()[kind];
    let look = vec(client::look(camera));
    // the player's eyes: the camera, or in front of it when it is behind them
    let eye = if third { vec(camera.position) + look * f64::from(chase) } else { vec(camera.position) };
    let mut ground = WorldGround::new(host);
    let radar = ground.radar(eye, 6);
    let yaw = f64::from(camera.yaw);
    let mut sim;
    if radar < 3.5 {
        // standing: the jet on its wheels, facing the way the player faces
        let rot = Q::facing(yaw);
        let ground_y = eye.y - radar;
        let mut pos = eye - rot.rotate(spec.eye);
        pos.y = ground_y - spec.gear[1].y + 0.03;
        sim = Sim::new(kind, pos, rot, 0.0);
        sim.ctl.throttle = 0.0;
    } else {
        // in the air: flying, nose where the player looks (not too steep)
        let pitch = f64::from(camera.pitch).clamp(-30.0, 30.0).to_radians();
        let level = Q::facing(yaw);
        let rot = level.mul(Q::axis_angle(v(1.0, 0.0, 0.0), pitch));
        let pos = eye - rot.rotate(spec.eye);
        sim = Sim::new(kind, pos, rot, 75.0);
        sim.ctl.throttle = 0.8;
        sim.engine = 0.8;
    }
    let (left, _, _) = sim.rot.basis();
    let _ = left;
    let stores = jet_mesh::stores(spec);
    let jet = Jet {
        prev: (sim.pos, sim.rot),
        throttle: sim.ctl.throttle,
        sim,
        fcs: Fcs::default(),
        mode: Mode::MouseAim,
        assist: true,
        inp: Inputs::default(),
        frozen: None,
        moved: V::ZERO,
        rounds: if spec.guns { MAX_ROUNDS } else { 0 },
        missiles: u32::from(spec.missiles),
        bombs: u32::from(spec.bombs),
        reload_missile: 0,
        reload_bomb: 0,
        carpet: 0,
        bay_until: 0,
        lock: (0, V::ZERO),
        radar,
        impact: None,
        pull_up: false,
        trails: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
        mach_was: 0.0,
        last_boom: 0,
        gear_was: 1.0,
        stick: (0.0, 0.0),
        ticks: 0,
    };
    let _ = stores;
    {
        let mut c = client::state();
        c.flying = None;
    }
    let mut j = state();
    j.jet = Some(jet);
    j.you = None;
    j.dimension = camera.dimension.clone();
    drop(j);
    client::protect_falls(20.0);
    client::sound(host, "minecraft:block.iron_trapdoor.close", eye.arr(), 1.0, 0.6, 0);
    client::sound(host, "minecraft:block.beacon.activate", eye.arr(), 0.8, 1.6, 4);
}

fn leave(host: &mut Host<'_>, camera: &NativeCamera, jet: &Jet, crashed: bool) {
    send(host, Up::Leave);
    client::protect_falls(20.0);
    let at = jet.sim.pos;
    if crashed {
        let p = at.arr();
        {
            let mut guard = client::state();
            let c = &mut *guard;
            client::effect(c, Kind::Orb, 0.0, 1.6, p, [0.0, 1.0, 0.0], 14.0, [1.0, 0.55, 0.2]);
            client::effect(c, Kind::Shockwave, 0.0, 1.5, p, [0.0, 1.0, 0.0], 30.0, [1.0, 0.6, 0.3]);
            let mut smoke = crate::client::Effect::new(Kind::Smoke, 0.2, 14.0, p, [40.0, 14.0, 3.0], 4.0, [0.2, 0.19, 0.18]);
            smoke.q_w = 0.8;
            smoke.c_w = 0.9;
            c.push_effect(smoke);
        }
        client::sound(host, "minecraft:entity.generic.explode", p, 6.0, 0.6, 0);
        client::sound(host, "minecraft:entity.generic.explode", p, 6.0, 0.4, 3);
        // thrown clear of the wreck
        host.push_player([0.0, 0.9, 0.0]);
    } else {
        // the player keeps the jet's speed, in blocks a tick
        let v = jet.sim.vel * (1.0 / 20.0);
        host.push_player([v.x as f32, v.y as f32, v.z as f32]);
        client::sound(host, "minecraft:block.iron_trapdoor.open", camera.position, 1.0, 0.6, 0);
    }
}

// ---- each tick --------------------------------------------------------------------------------------

pub fn tick(host: &mut Host<'_>, camera: &NativeCamera) {
    {
        let mut j = state();
        j.ticks += 1;
        if let Some(h) = j.held {
            j.held = Some(h + 1);
            if h + 1 == HOLD_TICKS {
                j.picker = true;
                drop(j);
                client::sound(host, "minecraft:ui.button.click", camera.position, 0.5, 1.4, 0);
            }
        }
    }
    let now = Instant::now();
    {
        let mut j = state();
        j.tracers.retain(|t| t.start.elapsed().as_secs_f32() < t.life);
        j.shots.retain(|(_, at)| now.duration_since(*at).as_secs_f32() < 0.5);
        let ids: Vec<u32> = j.shots.iter().map(|s| s.0.id).collect();
        j.smoke.retain_mut(|(id, trail)| {
            trail.retain(|(_, t)| now.duration_since(*t).as_secs_f32() < 4.0);
            ids.contains(id) || !trail.is_empty()
        });
    }
    if state().jet.is_none() {
        return;
    }
    let look = vec(client::look(camera));
    let mut events: Vec<Event> = Vec::new();
    let crashed = {
        let mut j = state();
        let ticks = j.ticks;
        let you = j.you;
        let Some(jet) = j.jet.as_mut() else {
            return;
        };
        fly_tick(host, camera, jet, look, you, ticks, &mut events)
    };
    if let Some(reason) = crashed {
        let jet = state().jet.take();
        if let Some(jet) = jet {
            let _ = reason;
            leave(host, camera, &jet, true);
        }
        return;
    }
    for e in events {
        e.run(host);
    }
}

/// Things a tick decides that touch the client's own state (pushed after the jets' lock goes).
enum Event {
    Effect(crate::client::Effect),
    Sound(&'static str, [f64; 3], f32, f32, i32),
    Send(Up),
}

impl Event {
    fn run(self, host: &mut Host<'_>) {
        match self {
            Event::Effect(e) => client::state().push_effect(e),
            Event::Sound(name, at, volume, pitch, delay) => client::sound(host, name, at, volume, pitch, delay),
            Event::Send(up) => send(host, up),
        }
    }
}

fn fly_tick(host: &mut Host<'_>, camera: &NativeCamera, jet: &mut Jet, look: V, you: Option<(V, u64)>, ticks: u64, events: &mut Vec<Event>) -> Option<Crash> {
    jet.ticks += 1;
    jet.prev = (jet.sim.pos, jet.sim.rot);
    // the throttle stays where it is set
    jet.throttle = (jet.throttle + jet.inp.forward * 0.03).clamp(0.0, 1.0);
    let afterburner = jet.inp.sprint && jet.throttle > 0.9;
    let bay = jet.ticks < jet.bay_until || jet.carpet > 0;
    let mut pilot = Pilot {
        mode: jet.mode,
        assist: jet.assist,
        throttle: jet.throttle,
        afterburner,
        airbrake: jet.inp.jump,
        cobra: jet.inp.cobra,
        bay,
        ..Default::default()
    };
    let (left, _, fwd) = jet.sim.rot.basis();
    match jet.mode {
        Mode::MouseAim => {
            // sneak looks around without steering
            let aim = if jet.inp.sneak { *jet.frozen.get_or_insert(look) } else { look };
            if !jet.inp.sneak {
                jet.frozen = None;
            }
            pilot.aim = aim;
            pilot.stick_roll = -jet.inp.strafe;
            pilot.rudder = -jet.inp.strafe * 0.5;
        }
        Mode::Direct => {
            // the mouse is the stick: how far the look is off the nose
            let t = jet.sim.rot.unrotate(look);
            pilot.stick_pitch = (t.y.atan2(t.z.max(0.05)) / 0.42).clamp(-1.0, 1.0);
            pilot.stick_roll = ((-t.x).atan2(t.z.max(0.05)) / 0.42).clamp(-1.0, 1.0);
            pilot.rudder = -jet.inp.strafe;
            pilot.aim = fwd;
        }
    }
    jet.stick = (pilot.stick_pitch, pilot.stick_roll);
    let _ = left;
    let mut ground = WorldGround::new(host);
    jet.radar = ground.radar(jet.sim.pos, 60);
    for _ in 0..STEPS {
        let ctl = jet_sim::fly(&jet.sim, &pilot, &mut jet.fcs, jet.radar, 0.01);
        jet.sim.step(&ctl, &mut ground, 0.01);
        if jet.sim.crashed.is_some() {
            return jet.sim.crashed;
        }
    }
    let sim = &jet.sim;
    let (left, up, fwd) = sim.rot.basis();
    // the player follows the seat: push them so that next tick they move to where it will be
    let eye_height = f64::from(camera.eye_height.max(0.5));
    let target = sim.eye() + sim.vel * 0.05 - v(0.0, eye_height, 0.0);
    let feet = match you {
        Some((p, at)) if ticks.saturating_sub(at) < 10 => p + jet.moved * (ticks.saturating_sub(at) as f64 + 1.0),
        _ => sim.eye() - v(0.0, eye_height, 0.0),
    };
    let mut want = target - feet;
    if want.len() > 9.5 {
        want = want.norm() * 9.5;
    }
    let after = v(jet.moved.x * 0.91, (jet.moved.y - 0.08) * 0.98, jet.moved.z * 0.91);
    let push = want - after;
    jet.moved = want;
    host.push_player([push.x as f32, push.y as f32, push.z as f32]);
    // The camera rides the jet itself (`camera_pose`), so the player's body lagging behind the
    // seat is never a crash: the server puts it back in the seat if it falls far behind.
    client::protect_falls(5.0);
    events.push(Event::Send(Up::Fly { dimension: camera.dimension.clone(), eye: sim.eye().arr(), vel: sim.vel.arr(), fwd: fwd.arr() }));

    weapons(host, camera, jet, events);
    let sim = &jet.sim;
    // where a bomb would land; and whether the ground is coming up fast
    jet.impact = predict_bomb(sim.pos, sim.vel, sim.pos.y - jet.radar).map(|p| p - sim.eye());
    jet.pull_up = sim.vel.y < -12.0 && jet.radar / -sim.vel.y < 4.0 && !sim.on_ground;
    // reloading
    let spec = sim.spec();
    if jet.missiles < u32::from(spec.missiles) {
        jet.reload_missile += 1;
        if jet.reload_missile >= 60 || sim.on_ground && sim.airspeed() < 2.0 {
            jet.reload_missile = 0;
            jet.missiles += 1;
        }
    }
    if jet.bombs < u32::from(spec.bombs) && jet.carpet == 0 {
        jet.reload_bomb += 1;
        if jet.reload_bomb >= if spec.carpet { 20 } else { 80 } || sim.on_ground && sim.airspeed() < 2.0 {
            jet.reload_bomb = 0;
            jet.bombs += 1;
        }
    }
    if spec.guns && !jet.inp.fire && jet.rounds < MAX_ROUNDS {
        jet.rounds = (jet.rounds + 6).min(MAX_ROUNDS);
    }

    // effects and sounds
    let pos = sim.pos;
    let speed = sim.airspeed();
    if (jet.mach_was - 1.0).signum() != (sim.mach - 1.0).signum() && jet.ticks.saturating_sub(jet.last_boom) > 40 && jet.ticks > 20 {
        jet.last_boom = jet.ticks;
        let mut e = crate::client::Effect::new(Kind::Shockwave, 0.0, 1.2, (pos - fwd * 4.0).arr(), fwd.arr(), 26.0, [0.9, 0.95, 1.0]);
        e.seed = 3.0;
        events.push(Event::Effect(e));
        events.push(Event::Sound("minecraft:entity.warden.sonic_boom", pos.arr(), 3.0, 0.6, 0));
        events.push(Event::Sound("minecraft:entity.generic.explode", pos.arr(), 3.0, 0.5, 2));
    }
    jet.mach_was = sim.mach;
    let now = Instant::now();
    let tip = |side: f64| pos + sim.rot.rotate(v(side * spec.span * 0.5, 0.0, -1.5));
    let vortex = sim.g > 4.0 || sim.alpha > 0.22 && speed > 40.0;
    for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
        let trail = &mut jet.trails[k];
        if vortex {
            trail.push_back((tip(side), now));
        } else if trail.back().is_some_and(|(p, _)| p.x.is_finite()) {
            trail.push_back((v(f64::NAN, 0.0, 0.0), now));
        }
        while trail.len() > 18 {
            trail.pop_front();
        }
        trail.retain(|(_, t)| now.duration_since(*t).as_secs_f32() < 1.4);
    }
    let contrail = sim.pos.y > 200.0 && speed > 40.0;
    {
        let trail = &mut jet.trails[2];
        if contrail {
            let behind = pos + sim.rot.rotate(spec.engines[0] * 0.0 + v(0.0, 0.0, -spec.length * 0.55));
            trail.push_back((behind, now));
        } else if trail.back().is_some_and(|(p, _)| p.x.is_finite()) {
            trail.push_back((v(f64::NAN, 0.0, 0.0), now));
        }
        while trail.len() > 20 {
            trail.pop_front();
        }
        trail.retain(|(_, t)| now.duration_since(*t).as_secs_f32() < 6.0);
    }
    let eye = camera.position;
    if jet.ticks % 8 == 0 {
        let e = sim.engine as f32;
        events.push(Event::Sound("minecraft:block.beacon.ambient", eye, 0.25 + 0.5 * e, 0.6 + 0.9 * e, 0));
        events.push(Event::Sound("minecraft:entity.blaze.burn", pos.arr(), 0.4 + 1.2 * e, 0.5 + 0.2 * e, 0));
    }
    if sim.burner > 0.3 && jet.ticks % 5 == 0 && spec.flame {
        events.push(Event::Sound("minecraft:item.firecharge.use", pos.arr(), 0.8 * sim.burner as f32, 0.45, 0));
    }
    if jet.ticks % 30 == 0 && speed > 20.0 {
        events.push(Event::Sound("minecraft:item.elytra.flying", eye, (speed as f32 / 140.0).min(0.7), 0.7 + speed as f32 / 200.0, 0));
    }
    if sim.stalled && jet.ticks % 10 == 0 || jet.pull_up && jet.ticks % 6 == 0 {
        events.push(Event::Sound("minecraft:block.note_block.bass", eye, 0.8, if jet.pull_up { 1.4 } else { 0.8 }, 0));
    }
    if (sim.gear - jet.gear_was).abs() > 0.01 && (jet.gear_was == 0.0 || jet.gear_was == 1.0) {
        events.push(Event::Sound(if sim.gear > jet.gear_was { "minecraft:block.piston.extend" } else { "minecraft:block.piston.contract" }, eye, 0.6, 0.7, 0));
    }
    jet.gear_was = if sim.gear > 0.99 { 1.0 } else if sim.gear < 0.01 { 0.0 } else { 0.5 };
    if jet.lock.0 > 0 && jet.ticks % if jet.lock.0 == 2 { 2 } else { 6 } == 0 {
        events.push(Event::Sound("minecraft:block.note_block.bit", eye, 0.35, if jet.lock.0 == 2 { 2.0 } else { 1.5 }, 0));
    }
    let _ = (left, up);
    None
}

/// Where a bomb let go now would land: falling, slowed a little by the air.
fn predict_bomb(pos: V, vel: V, ground: f64) -> Option<V> {
    let (mut p, mut v) = (pos, vel);
    for _ in 0..400 {
        v = v + V::UP * (-jet_sim::GRAVITY * 0.05) - v * (0.004 * 0.05 * v.len().min(200.0) / 10.0);
        p = p + v * 0.05;
        if p.y <= ground {
            return Some(p);
        }
    }
    None
}

// ---- weapons ----------------------------------------------------------------------------------------

fn weapons(host: &mut Host<'_>, camera: &NativeCamera, jet: &mut Jet, events: &mut Vec<Event>) {
    let sim = &jet.sim;
    let spec = sim.spec();
    let _ = spec.about;
    let (_, _, fwd) = sim.rot.basis();
    let dimension = camera.dimension.clone();
    // the cannon: two rounds a tick while the button is held
    if jet.inp.fire && spec.guns && jet.rounds >= 2 {
        jet.rounds -= 2;
        let muzzle = sim.pos + sim.rot.rotate(spec.eye + v(0.45, -0.55, 1.6));
        events.push(Event::Send(Up::Gun { dimension: dimension.clone(), from: muzzle.arr(), dir: fwd.arr(), count: 2 }));
        let mut ground = WorldGround::new(host);
        let vel = sim.vel + fwd * MUZZLE_SPEED;
        let mut hit = None;
        let mut s = 2.0;
        while s < 380.0 {
            let p = muzzle + fwd * s;
            if ground.solid(p.x, p.y, p.z) {
                hit = Some(s);
                break;
            }
            s += 2.0;
        }
        let range = hit.unwrap_or(380.0);
        let life = (range / vel.len()) as f32;
        {
            let mut j_tracers = Vec::new();
            j_tracers.push(Tracer { from: muzzle, vel, start: Instant::now(), life });
            PENDING.lock().unwrap_or_else(|p| p.into_inner()).extend(j_tracers);
        }
        if let Some(d) = hit {
            let p = (muzzle + fwd * d).arr();
            let mut spark = crate::client::Effect::new(Kind::Orb, life, 0.3, p, [0.0, 1.0, 0.0], 1.4, [1.0, 0.75, 0.35]);
            spark.seed = (jet.ticks % 97) as f32;
            events.push(Event::Effect(spark));
            if jet.ticks % 3 == 0 {
                let mut dust = crate::client::Effect::new(Kind::Smoke, life, 2.5, p, [5.0, 3.0, 1.5], 0.8, [0.45, 0.4, 0.33]);
                dust.c_w = 0.5;
                events.push(Event::Effect(dust));
            }
        }
        let flash = crate::client::Effect::new(Kind::Orb, 0.0, 0.06, muzzle.arr(), fwd.arr(), 0.9, [1.0, 0.8, 0.4]);
        events.push(Event::Effect(flash));
        if jet.ticks % 2 == 0 {
            events.push(Event::Sound("minecraft:entity.firework_rocket.blast", muzzle.arr(), 0.9, 1.7, 0));
        }
    }
    // the bomber's stick of bombs, one every three ticks once the doors are open
    if jet.carpet > 0 && jet.ticks % 3 == 0 && jet.sim.bay > 0.9 {
        jet.carpet -= 1;
        jet.bombs = jet.bombs.saturating_sub(1);
        let side = if jet.carpet % 2 == 0 { 0.6 } else { -0.6 };
        let from = jet.sim.pos + jet.sim.rot.rotate(v(side, -1.4, (jet.carpet % 3) as f64 - 1.0));
        events.push(Event::Send(Up::Bomb { dimension, from: from.arr(), vel: jet.sim.vel.arr(), big: true }));
        events.push(Event::Sound("minecraft:block.dispenser.launch", from.arr(), 1.0, 0.5, 0));
        if jet.carpet == 0 {
            jet.bay_until = jet.ticks + 20;
        }
    }
}

/// Tracers made this tick, handed to the jets' state once its lock is free.
static PENDING: Mutex<Vec<Tracer>> = Mutex::new(Vec::new());

fn fire_missile(host: &mut Host<'_>) {
    let Some(camera) = host.camera() else {
        return;
    };
    let mut j = state();
    let Some(jet) = j.jet.as_mut() else {
        return;
    };
    let spec = jet.sim.spec();
    if jet.missiles == 0 || spec.missiles == 0 {
        drop(j);
        client::sound(host, "minecraft:block.dispenser.fail", camera.position, 0.6, 1.4, 0);
        return;
    }
    let stores = jet_mesh::stores(spec);
    let shown: Vec<usize> = stores.iter().enumerate().filter(|(_, s)| s.missile).map(|(i, _)| i).collect();
    // the store that goes: the last still shown, or the bay
    let index = jet.missiles as usize - 1;
    let local = match shown.get(index) {
        Some(&i) => stores[i].pos,
        None => {
            jet.bay_until = jet.ticks + 22;
            v(0.0, -1.2, 0.5)
        }
    };
    jet.missiles -= 1;
    let (_, _, fwd) = jet.sim.rot.basis();
    let from = jet.sim.pos + jet.sim.rot.rotate(local);
    let up = Up::Missile { dimension: camera.dimension.clone(), from: from.arr(), vel: jet.sim.vel.arr(), dir: fwd.arr() };
    drop(j);
    send(host, up);
    client::sound(host, "minecraft:entity.firework_rocket.launch", from.arr(), 2.0, 0.6, 0);
    client::sound(host, "minecraft:entity.blaze.shoot", from.arr(), 1.5, 0.5, 2);
}

fn drop_bomb(host: &mut Host<'_>) {
    let Some(camera) = host.camera() else {
        return;
    };
    let mut j = state();
    let Some(jet) = j.jet.as_mut() else {
        return;
    };
    let spec = jet.sim.spec();
    if jet.bombs == 0 || jet.carpet > 0 {
        drop(j);
        client::sound(host, "minecraft:block.dispenser.fail", camera.position, 0.6, 1.2, 0);
        return;
    }
    if spec.carpet {
        // the bomber opens its doors and lets the whole stick go
        jet.carpet = jet.bombs;
        jet.bay_until = jet.ticks + 200;
        drop(j);
        client::sound(host, "minecraft:block.iron_door.open", camera.position, 1.0, 0.5, 0);
        return;
    }
    let stores = jet_mesh::stores(spec);
    let shown: Vec<usize> = stores.iter().enumerate().filter(|(_, s)| !s.missile).map(|(i, _)| i).collect();
    let local = shown.get(jet.bombs as usize - 1).map_or(v(0.0, -1.0, 0.0), |&i| stores[i].pos);
    jet.bombs -= 1;
    let from = jet.sim.pos + jet.sim.rot.rotate(local);
    let up = Up::Bomb { dimension: camera.dimension.clone(), from: from.arr(), vel: jet.sim.vel.arr(), big: false };
    drop(j);
    send(host, up);
    client::sound(host, "minecraft:block.dispenser.launch", from.arr(), 1.0, 0.6, 0);
}

/// What the server says.
pub fn message(host: &mut Host<'_>, data: &[u8]) {
    let Some(down) = Down::decode(data) else {
        return;
    };
    let mut j = state();
    let ticks = j.ticks;
    match down {
        Down::You(p) => j.you = Some((vec(p), ticks)),
        Down::Lock { state, at } => {
            if let Some(jet) = j.jet.as_mut() {
                jet.lock = (state.min(2), vec(at));
            }
        }
        Down::Shots(shots) => {
            let now = Instant::now();
            for s in &shots {
                if s.missile {
                    let at = match j.smoke.iter().position(|(id, _)| *id == s.id) {
                        Some(i) => i,
                        None => {
                            j.smoke.push((s.id, VecDeque::new()));
                            j.smoke.len() - 1
                        }
                    };
                    let trail = &mut j.smoke[at].1;
                    trail.push_back((vec(s.pos), now));
                    while trail.len() > 14 {
                        trail.pop_front();
                    }
                }
            }
            j.shots = shots.into_iter().map(|s| (s, now)).collect();
        }
        Down::Boom { at, radius } => {
            drop(j);
            let r = radius as f32;
            let mut guard = client::state();
            let c = &mut *guard;
            client::effect(c, Kind::Orb, 0.0, 0.9, at, [0.0, 1.0, 0.0], r * 2.2, [1.0, 0.6, 0.25]);
            client::effect(c, Kind::Shockwave, 0.0, 1.0, at, [0.0, 1.0, 0.0], r * 5.0, [1.0, 0.6, 0.3]);
            let mut smoke = crate::client::Effect::new(Kind::Smoke, 0.15, 7.0, at, [f64::from(r) * 6.0, f64::from(r) * 2.5, 3.0], r * 0.8, [0.25, 0.23, 0.21]);
            smoke.q_w = 0.4;
            smoke.c_w = 0.8;
            c.push_effect(smoke);
            let _ = host;
            return;
        }
    }
}

/// Each frame: where the camera is and how far through the tick.
pub fn frame(host: &mut Host<'_>) {
    let Some(camera) = host.camera() else {
        return;
    };
    let pending: Vec<Tracer> = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|p| p.into_inner()));
    let mut j = state();
    j.tracers.extend(pending);
    j.partial = camera.partial_tick;
    j.look = vec(client::look(&camera));
    j.camera = camera.position;
}

// ---- the camera ------------------------------------------------------------------------------------

/// The third-person camera's distance: far enough back to see the jet.
pub fn camera_distance(distance: &mut f32) {
    let mut j = state();
    j.detached = Some(Instant::now());
    let want = match j.jet.as_ref() {
        Some(jet) => jet.sim.spec().chase as f32,
        None => *distance,
    };
    j.chase += (want - j.chase) * 0.15;
    if j.jet.is_some() || (j.chase - *distance).abs() > 0.05 {
        *distance = j.chase;
    }
}

/// How far through the tick this frame is.
pub fn partial() -> f32 {
    state().partial
}

/// The jet's pose this frame, between the last two ticks.
fn pose(j: &Jets) -> Option<(V, Q)> {
    let jet = j.jet.as_ref()?;
    let t = f64::from(j.partial.clamp(0.0, 1.0));
    Some((jet.prev.0.lerp(jet.sim.pos, t), jet.prev.1.slerp(jet.sim.rot, t)))
}

/// Turns the camera with the jet: in the cockpit it leans with the jet's roll (all the way,
/// fixed to the cockpit, in direct mode); a hard pull or a stall shakes it.
pub fn shake(shake: &mut NativeShake) {
    let j = state();
    let Some(jet) = j.jet.as_ref() else {
        return;
    };
    let Some((_, rot)) = pose(&j) else {
        return;
    };
    let third = third_person(&j);
    let sim = &jet.sim;
    let (left, up, fwd) = rot.basis();
    if !third {
        if jet.mode == Mode::Direct {
            // the view is the cockpit's: rotate the player's view onto the jet's
            let (pitch, yaw, roll) = view_onto(j.look, -left, up, fwd);
            shake.pitch += pitch;
            shake.yaw += yaw;
            shake.roll += roll;
        } else {
            let roll = left.y.atan2(up.y).to_degrees();
            shake.roll += (roll * 0.4) as f32;
        }
    }
    let buffet = (sim.g.abs() - 5.0).max(0.0) * 0.12 + if sim.stalled { 0.6 } else { 0.0 } + if sim.on_ground { (sim.airspeed() / 80.0).min(0.4) } else { 0.0 };
    if buffet > 0.0 {
        let t = Instant::now().elapsed().as_secs_f32() + j.ticks as f32 * 0.05;
        shake.pitch += (buffet as f32) * (t * 51.0).sin() * 0.5;
        shake.roll += (buffet as f32) * (t * 43.0).sin() * 0.3;
    }
}

/// The shake angles (pitch, yaw, roll, degrees) that turn a camera looking along `look` (level)
/// into one whose right, up and forward axes are these. The game builds the view as
/// roll(z) * pitch(x) * yaw(y) * the look's own view.
pub fn view_onto(look: V, right: V, up: V, fwd: V) -> (f32, f32, f32) {
    let f = look.norm();
    let r = f.cross(V::UP).norm();
    let r = if r.len() < 0.5 { v(1.0, 0.0, 0.0) } else { r };
    let u = r.cross(f);
    // rows: right, up, back
    let cam = [r, u, f * -1.0];
    let want = [right, up, fwd * -1.0];
    // S = W * C^T
    let mut s = [[0.0f64; 3]; 3];
    for i in 0..3 {
        for k in 0..3 {
            s[i][k] = want[i].dot(cam[k]);
        }
    }
    let alpha = s[2][1].clamp(-1.0, 1.0).asin();
    let beta = (-s[2][0]).atan2(s[2][2]);
    let gamma = (-s[0][1]).atan2(s[1][1]);
    (alpha.to_degrees() as f32, beta.to_degrees() as f32, gamma.to_degrees() as f32)
}

/// A touch wider with speed.
pub fn fov(degrees: f32) -> f32 {
    let j = state();
    match j.jet.as_ref() {
        Some(jet) => degrees + (jet.sim.airspeed() as f32 / 12.0).min(10.0),
        None => degrees,
    }
}

/// While flying, the player's own body isn't drawn (they sit in the jet), nor their hand.
pub fn hides_player() -> bool {
    state().jet.is_some()
}

// ---- drawing ----------------------------------------------------------------------------------------

/// Where the jet's centre is this frame, from the camera at `camera`.
fn anchor(at: V, camera: [f64; 3]) -> V {
    at - vec(camera)
}

/// The camera this frame, riding the jet (between the last two ticks, so it is smooth at any
/// frame rate): in the cockpit, at the pilot's eyes; behind, the chase distance back along the
/// look. `pose` is the camera's position, yaw and pitch, changed in place. False with no jet.
pub fn camera_pose(partial: f32, third: bool, pose: &mut ([f64; 3], f32, f32)) -> bool {
    let mut j = state();
    j.third = third;
    j.partial = partial;
    let Some((at, rot)) = self::pose(&j) else {
        return false;
    };
    let Some(jet) = j.jet.as_ref() else {
        return false;
    };
    let eye = at + rot.rotate(jet.sim.spec().eye);
    let look = vec(flight::look(f64::from(pose.1), f64::from(pose.2)));
    let camera = if third { eye - look * f64::from(j.chase) + V::UP * 0.6 } else { eye };
    pose.0 = camera.arr();
    true
}

/// The models to draw this frame, and the `Frame` uniform's bytes for them.
pub fn gpu(view_proj: [f32; 16], size: [u32; 2], depth_size: [u32; 2], camera: [f64; 3], seconds: f32) -> Option<(Vec<u8>, Vec<Draw>)> {
    let j = state();
    let now = Instant::now();
    let mut draws = Vec::new();
    let mut frame: Vec<f32> = Vec::with_capacity(gfx::JET_FRAME_SIZE / 4);
    frame.extend_from_slice(&view_proj);
    let day = client::DAY_TIME.load(std::sync::atomic::Ordering::Relaxed);
    let a = (day.rem_euclid(24_000) as f64 / 24_000.0) * std::f64::consts::TAU;
    let night = crate::render::night(day);
    frame.extend_from_slice(&[a.cos() as f32, a.sin() as f32, 0.15, night]);
    frame.extend_from_slice(&[size[0] as f32, size[1] as f32, depth_size[0] as f32, depth_size[1] as f32]);
    let third = third_person(&j);
    let storm = {
        let c = client::state();
        crate::items::atmosphere(&c, camera)[1]
    };
    frame.extend_from_slice(&[seconds, if j.jet.is_some() && !third { 1.0 } else { 0.0 }, storm, 0.0]);
    if let (Some(jet), Some((centre, rot))) = (j.jet.as_ref(), pose(&j)) {
        let sim = &jet.sim;
        let spec = sim.spec();
        let (left, up, fwd) = rot.basis();
        let at = anchor(centre, camera);
        let mut parts = sim.parts;
        // stores already fired are hidden
        let stores = jet_mesh::stores(spec);
        let (mut missiles_left, mut bombs_left) = (jet.missiles as usize, jet.bombs as usize);
        for (i, s) in stores.iter().enumerate() {
            let shown = if s.missile {
                let keep = missiles_left > 0;
                missiles_left = missiles_left.saturating_sub(1);
                keep
            } else {
                let keep = bombs_left > 0;
                bombs_left = bombs_left.saturating_sub(1);
                keep
            };
            parts[part::STORES as usize + i] = if shown { 0.0 } else { 1.0 };
        }
        draws.push(Draw {
            mesh: sim.kind,
            model: gfx::model_matrix(left.arr(), up.arr(), fwd.arr(), at.arr()),
            parts,
            glow: (sim.engine * 0.7 + sim.burner * 1.5) as f32,
            hide_pilot: !third,
            world_test: true,
            cut: jet_mesh::cockpit_cut(sim.kind),
        });
        for axis in [left, up, fwd] {
            frame.extend_from_slice(&[axis.x as f32, axis.y as f32, axis.z as f32, 0.0]);
        }
        let speed = sim.airspeed();
        let vdir = if speed > 1.0 { sim.vel.norm() } else { fwd };
        frame.extend_from_slice(&[vdir.x as f32, vdir.y as f32, vdir.z as f32, speed as f32]);
        let aim = jet.frozen.unwrap_or(j.look);
        frame.extend_from_slice(&[aim.x as f32, aim.y as f32, aim.z as f32, if jet.mode == Mode::Direct { 1.0 } else { 0.0 }]);
        let lock_dir = (jet.lock.1 - vec(camera)).norm();
        frame.extend_from_slice(&[lock_dir.x as f32, lock_dir.y as f32, lock_dir.z as f32, f32::from(jet.lock.0) * 0.5]);
        let heading = fwd.x.atan2(-fwd.z).to_degrees().rem_euclid(360.0);
        frame.extend_from_slice(&[sim.pos.y as f32, heading as f32, jet.throttle as f32, sim.burner as f32]);
        frame.extend_from_slice(&[sim.g as f32, sim.mach as f32, sim.alpha.to_degrees() as f32, if sim.stalled { 1.0 } else { 0.0 }]);
        frame.extend_from_slice(&[jet.rounds as f32, jet.missiles as f32, jet.bombs as f32, sim.gear as f32]);
        match jet.impact {
            Some(d) => {
                let d = d.norm();
                frame.extend_from_slice(&[d.x as f32, d.y as f32, d.z as f32, if spec.bombs > 0 && jet.bombs > 0 { 1.0 } else { 0.0 }]);
            }
            None => frame.extend_from_slice(&[0.0, -1.0, 0.0, 0.0]),
        }
        let roll = left.y.atan2(up.y).to_degrees();
        let pitch = fwd.y.clamp(-1.0, 1.0).asin().to_degrees();
        frame.extend_from_slice(&[roll as f32, pitch as f32, jet.radar as f32, if jet.pull_up { 1.0 } else { 0.0 }]);
        frame.extend_from_slice(&[if jet.assist { 1.0 } else { 0.0 }, jet.stick.0 as f32, jet.stick.1 as f32, 0.0]);
    } else {
        for _ in 0..12 {
            frame.extend_from_slice(&[0.0, 0.0, 1.0, 0.0]);
        }
    }
    // missiles and bombs in flight
    for (shot, at) in &j.shots {
        let dt = now.duration_since(*at).as_secs_f64().min(0.2);
        let p = vec(shot.pos) + vec(shot.vel) * dt - vec(camera);
        let f = vec(shot.vel).norm();
        let f = if f.len() < 0.5 { v(0.0, -1.0, 0.0) } else { f };
        let side = if f.y.abs() > 0.95 { v(1.0, 0.0, 0.0) } else { V::UP.cross(f).norm() };
        let u = f.cross(side);
        draws.push(Draw {
            mesh: if shot.missile { gfx::MISSILE } else { gfx::BOMB },
            model: gfx::model_matrix(side.arr(), u.arr(), f.arr(), p.arr()),
            parts: [0.0; 32],
            glow: 0.0,
            hide_pilot: false,
            world_test: true,
            cut: [0.0; 4],
        });
    }
    if draws.is_empty() {
        return None;
    }
    let bytes: Vec<u8> = frame.iter().flat_map(|f| f.to_le_bytes()).collect();
    debug_assert_eq!(bytes.len(), gfx::JET_FRAME_SIZE);
    Some((bytes, draws))
}

/// The jets' light effects (flames, vapour, tracers, the chase camera's markers) for the light
/// pass, and the trails of vapour and smoke, all from the camera at `camera`.
pub fn effects(camera: [f64; 3]) -> (Vec<[[f32; 4]; 4]>, Vec<[f32; 4]>) {
    let j = state();
    let now = Instant::now();
    let cam = vec(camera);
    let mut fx = Vec::new();
    let mut trails = Vec::new();
    let third = third_person(&j);
    if let (Some(jet), Some((centre, rot))) = (j.jet.as_ref(), pose(&j)) {
        let sim = &jet.sim;
        let spec = sim.spec();
        let (_, _, fwd) = rot.basis();
        let at = anchor(centre, camera);
        for e in &spec.engines {
            let nozzle = at + rot.rotate(*e - v(0.0, 0.0, 0.5));
            let exhaust = rot.rotate(v(sim.nozzle.1.sin(), sim.nozzle.0.sin(), -1.0).norm());
            let burn = sim.burner as f32;
            let length = if spec.flame { 1.5 + 2.0 * sim.engine as f32 + 7.0 * burn } else { 1.0 + sim.engine as f32 };
            let color = if spec.flame { [1.0, 0.55 + 0.2 * burn, 0.25 + 0.5 * burn] } else { [0.9, 0.5, 0.3] };
            fx.push([
                [19.0, 0.0, 1.0, 0.0],
                [nozzle.x as f32, nozzle.y as f32, nozzle.z as f32, spec.nozzle as f32 * 0.85],
                [exhaust.x as f32, exhaust.y as f32, exhaust.z as f32, length],
                [color[0], color[1], color[2], if spec.flame { burn } else { 0.0 }],
            ]);
            fx.push([
                [20.0, 0.0, 1.0, 0.0],
                [nozzle.x as f32, nozzle.y as f32, nozzle.z as f32, spec.nozzle as f32],
                [exhaust.x as f32, exhaust.y as f32, exhaust.z as f32, 6.0 + 10.0 * sim.engine as f32],
                [0.0, 0.0, 0.0, sim.engine as f32],
            ]);
        }
        let near_sound = 1.0 - ((sim.mach - 1.0).abs() / 0.07).min(1.0);
        if near_sound > 0.0 {
            let c = at + fwd * (-spec.length * 0.1);
            fx.push([
                [21.0, 0.0, 1.0, 0.0],
                [c.x as f32, c.y as f32, c.z as f32, (spec.span * 0.45) as f32],
                [fwd.x as f32, fwd.y as f32, fwd.z as f32, near_sound as f32],
                [1.0, 1.0, 1.0, 1.0],
            ]);
        }
        if third {
            // where the nose points, far ahead; and the lock
            let ring = at + fwd * 300.0;
            fx.push([[23.0, 0.0, 1.0, 0.0], [ring.x as f32, ring.y as f32, ring.z as f32, 0.012], [0.0; 4], [0.3, 1.0, 0.45, 1.0]]);
            if jet.lock.0 > 0 {
                let t = jet.lock.1 - cam;
                fx.push([[23.0, 0.0, 1.0, 0.0], [t.x as f32, t.y as f32, t.z as f32, 0.02], [0.0, 0.0, 0.0, 1.0], [1.0, 0.25, 0.2, f32::from(jet.lock.0) * 0.5]]);
            }
        }
        for trail in &jet.trails {
            push_trail(&mut trails, trail, cam, now, 0.0, 1.4);
        }
    }
    for t in &j.tracers {
        let age = t.start.elapsed().as_secs_f32();
        let head = t.from + t.vel * f64::from(age) - cam;
        let d = t.vel.norm();
        fx.push([[22.0, age, t.life, 0.0], [head.x as f32, head.y as f32, head.z as f32, 0.06], [d.x as f32, d.y as f32, d.z as f32, 16.0], [1.0, 0.7, 0.3, 1.0]]);
    }
    for (shot, at) in &j.shots {
        if !shot.missile {
            continue;
        }
        let dt = now.duration_since(*at).as_secs_f64().min(0.2);
        let p = vec(shot.pos) + vec(shot.vel) * dt - cam;
        let back = (vec(shot.vel) * -1.0).norm();
        fx.push([[19.0, 0.0, 1.0, 0.0], [p.x as f32, p.y as f32, p.z as f32, 0.12], [back.x as f32, back.y as f32, back.z as f32, 2.5], [1.0, 0.8, 0.5, 1.0]]);
    }
    for (_, trail) in &j.smoke {
        push_trail(&mut trails, trail, cam, now, 2.0, 4.0);
    }
    trails.truncate(96);
    (fx, trails)
}

/// A trail's points from the camera, oldest first, each with its kind and how faded it is; a
/// break between trails is marked by a negative w.
fn push_trail(out: &mut Vec<[f32; 4]>, trail: &VecDeque<(V, Instant)>, cam: V, now: Instant, kind: f32, life: f32) {
    if trail.len() < 2 || out.len() + trail.len() + 1 > 96 {
        return;
    }
    let kind = if kind == 0.0 && trail.len() > 18 { 1.0 } else { kind };
    for (p, t) in trail {
        if !p.x.is_finite() {
            out.push([0.0, 0.0, 0.0, -1.0]);
            continue;
        }
        let age = now.duration_since(*t).as_secs_f32() / life;
        let fade = (1.0 - age).clamp(0.0, 0.999);
        let r = *p - cam;
        out.push([r.x as f32, r.y as f32, r.z as f32, kind * 2.0 + fade]);
    }
    out.push([0.0, 0.0, 0.0, -1.0]);
}

/// The jets' own HUD read-out for the chase camera, and the picker.
pub struct Readout {
    pub name: &'static str,
    pub speed: f64,
    pub altitude: f64,
    pub throttle: f64,
    pub burner: f64,
    pub g: f64,
    pub mach: f64,
    pub rounds: u32,
    pub missiles: u32,
    pub bombs: u32,
    pub mode: Mode,
    pub assist: bool,
    pub lock: u8,
    pub stalled: bool,
    pub gear: f64,
    pub third: bool,
}

pub fn readout() -> (Option<Readout>, bool, usize) {
    let j = state();
    let r = j.jet.as_ref().map(|jet| Readout {
        name: jet.sim.spec().name,
        speed: jet.sim.airspeed(),
        altitude: jet.sim.pos.y,
        throttle: jet.throttle,
        burner: jet.sim.burner,
        g: jet.sim.g,
        mach: jet.sim.mach,
        rounds: jet.rounds,
        missiles: jet.missiles,
        bombs: jet.bombs,
        mode: jet.mode,
        assist: jet.assist,
        lock: jet.lock.0,
        stalled: jet.sim.stalled,
        gear: jet.sim.gear,
        third: third_person(&j),
    });
    (r, j.picker, j.selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rebuilds the view the game would make from a look and the shake angles.
    fn compose(look: V, angles: (f32, f32, f32)) -> [V; 3] {
        let f = look.norm();
        let r = f.cross(V::UP).norm();
        let u = r.cross(f);
        let cam = [r, u, f * -1.0];
        let (p, y, ro) = (f64::from(angles.0).to_radians(), f64::from(angles.1).to_radians(), f64::from(angles.2).to_radians());
        let rx = [[1.0, 0.0, 0.0], [0.0, p.cos(), -p.sin()], [0.0, p.sin(), p.cos()]];
        let ry = [[y.cos(), 0.0, y.sin()], [0.0, 1.0, 0.0], [-y.sin(), 0.0, y.cos()]];
        let rz = [[ro.cos(), -ro.sin(), 0.0], [ro.sin(), ro.cos(), 0.0], [0.0, 0.0, 1.0]];
        let mul = |a: [[f64; 3]; 3], b: [[f64; 3]; 3]| {
            let mut m = [[0.0; 3]; 3];
            for i in 0..3 {
                for k in 0..3 {
                    m[i][k] = (0..3).map(|n| a[i][n] * b[n][k]).sum();
                }
            }
            m
        };
        let s = mul(rz, mul(rx, ry));
        // rows of S * C: the camera's axes in the world
        let mut rows = [V::ZERO; 3];
        for i in 0..3 {
            rows[i] = cam[0] * s[i][0] + cam[1] * s[i][1] + cam[2] * s[i][2];
        }
        rows
    }

    #[test]
    fn the_cockpit_view_is_the_jets() {
        let look = v(0.3, 0.2, 1.0).norm();
        let rot = Q::axis_angle(v(0.2, 1.0, 0.4), 0.9).mul(Q::axis_angle(v(0.0, 0.0, 1.0), 0.7));
        let (left, up, fwd) = rot.basis();
        let angles = view_onto(look, left * -1.0, up, fwd);
        let rows = compose(look, angles);
        assert!((rows[0] - left * -1.0).len() < 1e-4, "{:?}", rows);
        assert!((rows[1] - up).len() < 1e-4);
        assert!((rows[2] - fwd * -1.0).len() < 1e-4);
    }

    #[test]
    fn a_bomb_falls_ahead_of_the_jet() {
        let p = predict_bomb(v(0.0, 200.0, 0.0), v(0.0, 0.0, 80.0), 64.0).unwrap();
        assert!(p.z > 100.0 && p.y <= 64.0);
    }
}
