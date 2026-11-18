//! The world's half. A cast arrives in [`receive`] and waits; each level's tick
//! ([`level_tick`]) runs the casts of the players in it, and what they leave going (black holes,
//! storms, the Cataclysm's spreading crater), since only the level being ticked can be changed.

use crate::flight::{self, V3};
use crate::net::Cast;
use crate::powers::{self, Power};
use mod_api::{
    BlockEdit, BlockRun, BlockState, EntityRef, Falling, Heightmap, Host, Identifier, LevelRef,
    HostBlockStates, PlayerRef, ServerHost, StatePredicate,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard};

/// Blocks a power may break: every block a tool mines, plants, glass and wool. Bedrock,
/// barriers, portals and command blocks are in none of these tags.
const BREAKABLE: [&str; 8] = [
    "minecraft:mineable/pickaxe",
    "minecraft:mineable/shovel",
    "minecraft:mineable/axe",
    "minecraft:mineable/hoe",
    "minecraft:replaceable",
    "minecraft:leaves",
    "minecraft:impermeable",
    "minecraft:wool",
];

/// How far from the player a cast may act.
const REACH: f64 = 1100.0;

#[derive(Clone, Debug)]
enum Job {
    Cast { player: u128, cast: Cast },
    Bolt { at: V3, radius: f64 },
    Collapse { at: V3, radius: f64, caster: Option<u32> },
    /// One band of the Cataclysm's crater: from `inner` to `outer` blocks out.
    Crater { at: V3, inner: f64, outer: f64, caster: Option<u32> },
}

struct Queued {
    due: u64,
    dimension: String,
    job: Job,
}

struct Well {
    dimension: String,
    at: V3,
    until: u64,
    singularity: bool,
    caster: Option<u32>,
}

struct Server {
    tick: u64,
    incoming: Vec<(u128, Cast)>,
    queue: Vec<Queued>,
    wells: Vec<Well>,
    /// Flying players' entity ids, and the tick their fall protection ends (u64::MAX while up).
    flyers: BTreeMap<u32, u64>,
    last: BTreeMap<(u128, u8), u64>,
    seed: u64,
}

static SERVER: Mutex<Server> = Mutex::new(Server {
    tick: 0,
    incoming: Vec::new(),
    queue: Vec::new(),
    wells: Vec::new(),
    flyers: BTreeMap::new(),
    last: BTreeMap::new(),
    seed: 0x2545_F491_4F6C_DD1D,
});

fn state() -> MutexGuard<'static, Server> {
    SERVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn rand(s: &mut Server) -> f64 {
    let mut x = s.seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    s.seed = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn random() -> f64 {
    rand(&mut state())
}

/// The fewest ticks between two casts of a power by one player.
fn spacing(power: Power) -> u64 {
    match power {
        Power::Lasers | Power::OmegaBeam | Power::Flight => 1,
        Power::Dash => 10,
        Power::Punch => 18,
        Power::Thunder => 50,
        Power::Frost => 70,
        Power::Gravity => 150,
        Power::TitanFist => 150,
        Power::Wrath => 230,
        Power::AbsoluteZero => 270,
        Power::Singularity => 390,
        Power::Cataclysm => 590,
    }
}

/// A cast from a player's client (the payload handler): kept for that player's level's tick.
pub fn receive(player: u128, cast: Cast) {
    let Some(power) = powers::from_code(cast.power) else {
        return;
    };
    let mut s = state();
    if power != Power::OmegaBeam && power != Power::Lasers && power != Power::Flight {
        let key = (player, cast.power);
        let now = s.tick;
        if s.last.get(&key).is_some_and(|&t| now < t + spacing(power)) {
            return;
        }
        s.last.insert(key, now);
    }
    if s.incoming.len() < 512 {
        s.incoming.push((player, cast));
    }
}

pub fn server_tick() {
    let mut s = state();
    s.tick += 1;
    let now = s.tick;
    s.flyers.retain(|_, until| *until > now);
}

/// Whether a fall should hurt this entity: not while it flies, nor just after.
pub fn fall_protected(entity: u32) -> bool {
    state().flyers.contains_key(&entity)
}

fn dimension_of(host: &dyn Host, level: LevelRef) -> Option<String> {
    level.dimension().id(host)
}

pub fn level_tick(host: &mut dyn ServerHost, level: LevelRef) {
    let Some(dimension) = dimension_of(host, level) else {
        return;
    };
    // The casts of players standing in this level.
    let incoming = std::mem::take(&mut state().incoming);
    let mut later = Vec::new();
    for (uuid, cast) in incoming {
        let Some(player) = host.player(uuid) else {
            continue;
        };
        let here = player.level(host).and_then(|l| dimension_of(host, l));
        if here.as_deref() == Some(dimension.as_str()) {
            let mut s = state();
            let due = s.tick;
            s.queue.push(Queued { due, dimension: dimension.clone(), job: Job::Cast { player: uuid, cast } });
        } else {
            later.push((uuid, cast));
        }
    }
    state().incoming.extend(later.into_iter().take(64));

    let (jobs, wells) = {
        let mut s = state();
        let now = s.tick;
        let mut jobs = Vec::new();
        s.queue.retain(|q| {
            if q.dimension == dimension && q.due <= now {
                jobs.push(q.job.clone());
                false
            } else {
                true
            }
        });
        s.wells.retain(|w| w.until > now);
        let wells: Vec<(V3, bool, Option<u32>, u64)> = s
            .wells
            .iter()
            .filter(|w| w.dimension == dimension)
            .map(|w| (w.at, w.singularity, w.caster, w.until - now))
            .collect();
        (jobs, wells)
    };
    for job in jobs {
        run(host, level, &dimension, job);
    }
    for (at, singularity, caster, left) in wells {
        pull(host, level, &dimension, at, singularity, caster, left);
    }
}

fn queue(dimension: &str, delay: u64, job: Job) {
    let mut s = state();
    let due = s.tick + delay;
    s.queue.push(Queued { due, dimension: dimension.to_owned(), job });
}

fn run(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, job: Job) {
    match job {
        Job::Cast { player, cast } => {
            let Some(p) = host.player(player) else {
                return;
            };
            cast_by(host, level, dimension, p, cast);
        }
        Job::Bolt { at, radius } => {
            let _ = level.strike_lightning(host, at, false);
            let mut columns = HashMap::new();
            carve_sphere(&mut columns, at, radius);
            carve(host, level, dimension, &columns, true);
            hurt_near(host, level, at, radius + 3.0, 12.0, "minecraft:lightning_bolt", None);
        }
        Job::Collapse { at, radius, caster } => {
            let mut columns = HashMap::new();
            carve_sphere(&mut columns, at, radius);
            carve(host, level, dimension, &columns, true);
            hurt_near(host, level, at, radius * 2.0, 30.0 + radius as f32 * 4.0, "minecraft:explosion", caster);
            fling(host, level, at, radius * 3.5, caster, |offset, d| {
                let k = (2.5 / d.max(1.0)).min(1.5);
                [offset[0] * k, 1.0, offset[2] * k]
            });
        }
        Job::Crater { at, inner, outer, caster } => crater_band(host, level, dimension, at, inner, outer, caster),
    }
}

fn cast_by(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, player: PlayerRef, cast: Cast) {
    let Some(power) = powers::from_code(cast.power) else {
        return;
    };
    let me = player.entity();
    let Some(position) = me.position(host) else {
        return;
    };
    if !cast.sane(position, REACH) {
        return;
    }
    let (at, dir) = (cast.at(), cast.dir().unwrap_or([0.0, 0.0, 1.0]));
    let caster = Some(me.id());
    match power {
        Power::Punch => punch(host, level, dimension, at, dir, 1.0, caster),
        Power::TitanFist => punch(host, level, dimension, at, dir, 5.0, caster),
        Power::Lasers => laser(host, level, dimension, at, cast.level, caster),
        Power::OmegaBeam => omega(host, level, dimension, at, dir, caster),
        Power::Thunder | Power::Wrath => {
            let wrath = power == Power::Wrath;
            let (count, radius, ticks) = if wrath { (36, 46.0, 70) } else { (7, 8.0, 18) };
            for (offset, tick) in powers::bolts(cast.level, count, radius, ticks) {
                let bolt = [at[0] + offset[0], at[1], at[2] + offset[1]];
                queue(dimension, u64::from(tick), Job::Bolt { at: bolt, radius: if wrath { 3.0 } else { 1.6 } });
            }
        }
        Power::Frost => frost(host, level, dimension, at, 14, caster),
        Power::AbsoluteZero => frost(host, level, dimension, at, 70, caster),
        Power::Gravity | Power::Singularity => {
            let singularity = power == Power::Singularity;
            let ticks = if singularity { 160 } else { 100 };
            let mut s = state();
            let until = s.tick + ticks;
            s.wells.push(Well { dimension: dimension.to_owned(), at, until, singularity, caster });
            drop(s);
            let radius = if singularity { 26.0 } else { 4.5 };
            queue(dimension, ticks, Job::Collapse { at, radius, caster });
        }
        Power::Dash => dash(host, level, at, dir, caster),
        Power::Flight => {
            let mut s = state();
            // Up: protected until landing. Down: still protected for ten seconds.
            let until = if cast.level == 1 { u64::MAX } else { s.tick + 200 };
            s.flyers.insert(me.id(), until);
        }
        Power::Cataclysm => {
            {
                // the caster is spared the fall
                let mut s = state();
                let until = s.tick + 600;
                s.flyers.entry(me.id()).and_modify(|u| *u = (*u).max(until)).or_insert(until);
            }
            let r = powers::CATACLYSM_RADIUS;
            let band = 10.0;
            let mut inner = 0.0;
            let mut delay = 4;
            while inner < r {
                let outer = (inner + band).min(r);
                queue(dimension, delay, Job::Crater { at, inner, outer, caster });
                inner = outer;
                delay += 1;
            }
            for k in 0..14u64 {
                let a = random() * std::f64::consts::TAU;
                let d = 40.0 + random() * (r - 60.0);
                let bolt = [at[0] + a.cos() * d, at[1], at[2] + a.sin() * d];
                queue(dimension, 6 + k * 5, Job::Bolt { at: bolt, radius: 2.0 });
            }
        }
    }
}

/// Super Punch (`k` 1) and Titan Fist (`k` 5): a cone that widens as it goes shatters, part of
/// it flies off, and what stands in it is hurt and thrown.
#[allow(clippy::too_many_arguments)]
fn punch(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, eye: V3, dir: V3, k: f64, caster: Option<u32>) {
    let mut columns = HashMap::new();
    let mut debris = Vec::new();
    let length = (16.0 * k) as i32;
    for step in 1..=length {
        let s = f64::from(step);
        let center = flight::add(eye, flight::scale(dir, s));
        let radius = 1.6 + s * 0.3;
        carve_sphere(&mut columns, center, radius);
        if step % (k as i32).max(1) == 0 {
            for _ in 0..2 {
                let r = random() * radius;
                let a = random() * std::f64::consts::TAU;
                let h = (random() - 0.5) * radius;
                debris.push([center[0] + a.cos() * r, center[1] + h, center[2] + a.sin() * r]);
            }
        }
    }
    for pos in debris {
        let spread = [(random() - 0.5) * 0.8, random() * 0.6 + 0.5, (random() - 0.5) * 0.8];
        let velocity = [dir[0] * 1.6 + spread[0], dir[1] + spread[1], dir[2] * 1.6 + spread[2]];
        throw(host, level, dimension, pos, velocity);
    }
    carve(host, level, dimension, &columns, true);
    let reach = 16.0 * k;
    hurt_cone(host, level, eye, dir, reach, 26.0 * k as f32, "minecraft:explosion", caster);
    fling(host, level, eye, reach + 2.0, caster, |offset, d| {
        let along = flight::dot(offset, dir);
        if along < 0.0 || d - along > 2.0 + along * 0.4 {
            return [0.0; 3];
        }
        let push = 3.0 * k.sqrt();
        [dir[0] * push, 1.0 + dir[1].max(0.0) * 2.0, dir[2] * push]
    });
}

/// Laser Eyes: the beams melt what they touch and set what stands there alight.
fn laser(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, at: V3, power: u8, caster: Option<u32>) {
    let p = f64::from(power.clamp(1, 5));
    let mut columns = HashMap::new();
    carve_sphere(&mut columns, at, 0.45 + p * 0.32);
    carve(host, level, dimension, &columns, true);
    let reach = 1.2 + p * 0.35;
    for e in entities_near(host, level, at, reach, caster) {
        let _ = e.hurt(host, "minecraft:in_fire", 1.5 + p as f32 * 1.5, None, None);
        let _ = e.set_remaining_fire_ticks(host, 100);
    }
}

/// The Omega Beam, every other tick while it fires: a tunnel 4 blocks across, 200 long.
fn omega(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, eye: V3, dir: V3, caster: Option<u32>) {
    let mut columns = HashMap::new();
    let mut s = 3.0;
    while s < 200.0 {
        carve_sphere(&mut columns, flight::add(eye, flight::scale(dir, s)), 2.2);
        s += 2.0;
    }
    carve(host, level, dimension, &columns, true);
    hurt_cone(host, level, eye, dir, 200.0, 12.0, "minecraft:in_fire", caster);
}

/// Frost Nova (`radius` 14) and Absolute Zero (70): water freezes, lava turns to obsidian, snow
/// settles; at 70 ice spikes rise too. What stands there is frozen, slowed and hurt.
fn frost(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, feet: V3, radius: i32, caster: Option<u32>) {
    let Some(states) = states(host, &["minecraft:snow", "minecraft:ice", "minecraft:obsidian", "minecraft:packed_ice", "minecraft:blue_ice"]) else {
        return;
    };
    let Some(filters) = predicates(host, &["minecraft:air", "minecraft:water", "minecraft:lava"]) else {
        return;
    };
    let mut runs = Vec::new();
    let (cx, cz) = (feet[0].floor() as i32, feet[2].floor() as i32);
    let surface = Some(Heightmap::MotionBlocking);
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz > radius * radius {
                continue;
            }
            let (x, z) = (cx + dx, cz + dz);
            // the top block: water to ice, lava to obsidian; then a layer of snow above
            runs.push(BlockRun { x, z, y: -1, height: 1, surface, block: 1, only: Some(1) });
            runs.push(BlockRun { x, z, y: -1, height: 1, surface, block: 2, only: Some(2) });
            runs.push(BlockRun { x, z, y: 0, height: 1, surface, block: 0, only: Some(0) });
        }
    }
    if radius > 30 {
        // ice spikes
        for _ in 0..40 {
            let a = random() * std::f64::consts::TAU;
            let d = 6.0 + random() * f64::from(radius - 8);
            let (x, z) = (cx + (a.cos() * d) as i32, cz + (a.sin() * d) as i32);
            let tall = 5 + (random() * 16.0) as u32;
            let block = if random() < 0.3 { 4 } else { 3 };
            for (ox, oz, h) in [(0, 0, tall), (1, 0, tall * 2 / 3), (0, 1, tall * 2 / 3), (-1, 0, tall / 2), (0, -1, tall / 2)] {
                runs.push(BlockRun { x: x + ox, z: z + oz, y: 0, height: h, surface, block, only: Some(0) });
            }
        }
    }
    let _ = host.edit_blocks(dimension, &BlockEdit { palette: states, filters, runs });
    let r = f64::from(radius) + 1.0;
    for e in entities_near(host, level, feet, r, caster) {
        let _ = e.hurt(host, "minecraft:freeze", if radius > 30 { 20.0 } else { 6.0 }, None, None);
    }
    let effect = if radius > 30 { 8 } else { 5 };
    let line = format!(
        "execute in {dimension} positioned {:.2} {:.2} {:.2} run effect give @e[type=!minecraft:player,distance=..{r:.0}] minecraft:slowness {} {effect}",
        feet[0], feet[1], feet[2], if radius > 30 { 15 } else { 8 }
    );
    let _ = host.commands().run_command(&line);
}

/// Hyper Dash: what stands along the 16 blocks ahead is struck and thrown aside.
fn dash(host: &mut dyn ServerHost, level: LevelRef, from: V3, dir: V3, caster: Option<u32>) {
    hurt_cone(host, level, from, dir, 17.0, 12.0, "minecraft:generic", caster);
    fling(host, level, from, 18.0, caster, |offset, _| {
        let along = flight::dot(offset, dir);
        let side = flight::sub(offset, flight::scale(dir, along));
        let off = flight::length(side);
        if !(0.0..=17.0).contains(&along) || off > 3.0 {
            return [0.0; 3];
        }
        let k = 1.6 / off.max(0.3);
        [side[0] * k + dir[0] * 0.8, 0.7, side[2] * k + dir[2] * 0.8]
    });
}

/// A black hole, each tick: everything near is pulled in and swirls, and blocks are torn from
/// the ground and thrown in. The Singularity also eats the land round it, more each tick.
#[allow(clippy::too_many_arguments)]
fn pull(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, at: V3, singularity: bool, caster: Option<u32>, left: u64) {
    let reach = if singularity { 120.0 } else { 24.0 };
    fling(host, level, at, reach, caster, |offset, d| {
        if d < 0.8 {
            return [0.0; 3];
        }
        let inward = flight::scale(offset, -1.0 / d);
        let swirl = [-inward[2], 0.0, inward[0]];
        let k = if singularity { 0.25 + 0.9 * (1.0 - d / reach).max(0.0) } else { 0.12 + 0.5 * (1.0 - d / reach).max(0.0) };
        flight::add(flight::add(flight::scale(inward, k), flight::scale(swirl, k * 0.6)), [0.0, 0.04, 0.0])
    });
    let tick = state().tick;
    let throws = if singularity { 3 } else { usize::from(tick % 3 == 0) };
    for _ in 0..throws {
        let a = random() * std::f64::consts::TAU;
        let r = if singularity { 10.0 + random() * 60.0 } else { 4.0 + random() * 10.0 };
        let x = (at[0] + a.cos() * r).floor() as i32;
        let z = (at[2] + a.sin() * r).floor() as i32;
        // the first solid block below the hole there
        let mut y = at[1].floor() as i32;
        let bottom = y - if singularity { 48 } else { 16 };
        while y > bottom {
            if let Some(state) = level.block_state(host, [x, y, z]) {
                if throwable(host, state) {
                    let pos = [f64::from(x) + 0.5, f64::from(y) + 1.05, f64::from(z) + 0.5];
                    let toward = flight::sub(at, pos);
                    let d = flight::length(toward).max(1.0);
                    let speed = if singularity { 1.8 } else { 0.9 };
                    let velocity = [toward[0] / d * speed, 0.9 + toward[1] / d * 0.5, toward[2] / d * speed];
                    let mut columns = HashMap::new();
                    columns.insert((x, z), (y, y));
                    carve(host, level, dimension, &columns, true);
                    throw(host, level, dimension, pos, velocity);
                    break;
                }
            }
            y -= 1;
        }
    }
    if singularity {
        // The ball of land it has eaten grows from 3 to 24 blocks across 8 seconds.
        let eaten = 3.0 + 21.0 * (1.0 - left as f64 / 160.0).clamp(0.0, 1.0);
        if tick % 4 == 0 {
            let mut columns = HashMap::new();
            carve_sphere(&mut columns, at, eaten);
            carve(host, level, dimension, &columns, true);
        }
    }
    if tick % 10 == 0 {
        hurt_near(host, level, at, if singularity { 10.0 } else { 2.0 }, if singularity { 10.0 } else { 4.0 }, "minecraft:magic", caster);
    }
}

/// One band of the Cataclysm's crater, `inner` to `outer` blocks from ground zero: a bowl 90
/// blocks deep at the centre, shallowing to the rim, cleared of everything up to 120 blocks
/// above the ground, with a floor of scorched stone. Unloaded chunks are left as they are.
#[allow(clippy::too_many_arguments)]
fn crater_band(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, at: V3, inner: f64, outer: f64, caster: Option<u32>) {
    let Some(palette) = states(host, &["minecraft:air", "minecraft:blackstone", "minecraft:magma_block", "minecraft:basalt"]) else {
        return;
    };
    let r = powers::CATACLYSM_RADIUS;
    let (cx, cz) = (at[0].floor() as i32, at[2].floor() as i32);
    let o = outer.ceil() as i32;
    let mut runs = Vec::new();
    let mut seed = (inner as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let surface = Some(Heightmap::WorldSurface);
    for dx in -o..=o {
        for dz in -o..=o {
            let d = f64::from(dx * dx + dz * dz).sqrt();
            if d < inner || d >= outer {
                continue;
            }
            let t = d / r;
            let depth = (90.0 * (1.0 - t * t)).max(2.0) as i32;
            let (x, z) = (cx + dx, cz + dz);
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let floor = match seed % 7 {
                0 => 2,
                1 | 2 => 3,
                _ => 1,
            };
            runs.push(BlockRun { x, z, y: -depth - 1, height: 1, surface, block: floor, only: None });
            runs.push(BlockRun { x, z, y: -depth, height: (depth + 120) as u32, surface, block: 0, only: None });
        }
    }
    let _ = host.edit_blocks(dimension, &BlockEdit { palette, filters: Vec::new(), runs });
    // what stands in this band is struck down and hurled outward
    let mid = (inner + outer) / 2.0;
    let box_min = [at[0] - outer, at[1] - 100.0, at[2] - outer];
    let box_max = [at[0] + outer, at[1] + 200.0, at[2] + outer];
    let found = level.entities_in(host, box_min, box_max).unwrap_or_default();
    for e in found {
        if Some(e.id()) == caster {
            continue;
        }
        let Some(p) = e.position(host) else {
            continue;
        };
        let off = flight::sub(p, at);
        let d = (off[0] * off[0] + off[2] * off[2]).sqrt();
        if d < inner - 2.0 || d >= outer + 2.0 {
            continue;
        }
        let _ = e.hurt(host, "minecraft:explosion", 200.0 * (1.0 - mid / r).max(0.25) as f32, None, None);
        let k = 3.0 / d.max(1.0);
        let _ = e.push(host, [off[0] * k, 1.5, off[2] * k]);
    }
}

/// Pushes what is within `radius` of `at` (not the caster) by what `push` says for its offset
/// and distance.
fn fling(host: &mut dyn ServerHost, level: LevelRef, at: V3, radius: f64, caster: Option<u32>, push: impl Fn(V3, f64) -> V3) {
    for e in entities_near(host, level, at, radius, caster) {
        let Some(p) = e.position(host) else {
            continue;
        };
        let offset = flight::sub(p, at);
        let v = push(offset, flight::length(offset));
        if v != [0.0; 3] && v.iter().all(|x| x.is_finite()) {
            let _ = e.push(host, v.map(|x| x.clamp(-4.0, 4.0)));
        }
    }
}

fn entities_near(host: &dyn Host, level: LevelRef, at: V3, radius: f64, caster: Option<u32>) -> Vec<EntityRef> {
    let min = [at[0] - radius, at[1] - radius, at[2] - radius];
    let max = [at[0] + radius, at[1] + radius, at[2] + radius];
    level
        .entities_in(host, min, max)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| Some(e.id()) != caster)
        .filter(|e| e.position(host).is_some_and(|p| flight::length(flight::sub(p, at)) <= radius))
        .collect()
}

fn hurt_near(host: &mut dyn ServerHost, level: LevelRef, at: V3, radius: f64, amount: f32, kind: &str, caster: Option<u32>) {
    for e in entities_near(host, level, at, radius, caster) {
        let _ = e.hurt(host, kind, amount, None, None);
    }
}

/// Hurts what stands in a cone from `from` along `dir`, `reach` long, widening as it goes.
#[allow(clippy::too_many_arguments)]
fn hurt_cone(host: &mut dyn ServerHost, level: LevelRef, from: V3, dir: V3, reach: f64, amount: f32, kind: &str, caster: Option<u32>) {
    let mid = flight::add(from, flight::scale(dir, reach / 2.0));
    for e in entities_near(host, level, mid, reach / 2.0 + 4.0, caster) {
        let Some(p) = e.position(host) else {
            continue;
        };
        let offset = flight::sub(p, from);
        let along = flight::dot(offset, dir);
        let off = flight::length(flight::sub(offset, flight::scale(dir, along)));
        if (0.0..=reach).contains(&along) && off <= 2.5 + along * 0.35 {
            let _ = e.hurt(host, kind, amount, None, None);
        }
    }
}

/// Throws the block at `pos` as a falling block (it stays where it was: the caller clears it).
fn throw(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, pos: V3, velocity: V3) {
    let block_pos = pos.map(|v| v.floor() as i32);
    let Some(state) = level.block_state(host, block_pos) else {
        return;
    };
    if !throwable(host, state) {
        return;
    }
    let mut falling = Falling::default();
    falling.drop_item = false;
    let _ = host.spawn_falling_block(dimension, state, pos, velocity, falling);
}

/// Worth throwing: a solid block, not air or a liquid, and in a tag a power may break.
fn throwable(host: &dyn ServerHost, state: BlockState) -> bool {
    let block = state.block();
    BREAKABLE[..4]
        .iter()
        .any(|tag| host.block_in_tag(block, tag).unwrap_or(false))
}

fn states(host: &dyn ServerHost, names: &[&str]) -> Option<Vec<BlockState>> {
    names.iter().map(|n| host.block_state_id(n)).collect()
}

/// Block predicates for exact blocks (`minecraft:water`) or tags (`#minecraft:leaves`).
fn predicates(host: &dyn ServerHost, names: &[&str]) -> Option<Vec<StatePredicate>> {
    names
        .iter()
        .map(|name| {
            if let Some(tag) = name.strip_prefix('#') {
                let id = Identifier::parse(tag).ok()?;
                let key = host.tags().key(&id)?;
                StatePredicate::tag(&host.block_states(), host.tags(), key, &[]).ok()
            } else {
                let block = host.block_key(name)?;
                StatePredicate::block(&host.block_states(), block, &[]).ok()
            }
        })
        .collect()
}

/// Adds a ball of radius `r` at `c` to the columns to clear: `(x, z)` to its lowest and
/// highest y.
pub fn carve_sphere(columns: &mut HashMap<(i32, i32), (i32, i32)>, c: V3, r: f64) {
    let (x0, x1) = ((c[0] - r).floor() as i32, (c[0] + r).floor() as i32);
    let (z0, z1) = ((c[2] - r).floor() as i32, (c[2] + r).floor() as i32);
    for x in x0..=x1 {
        for z in z0..=z1 {
            let dx = f64::from(x) + 0.5 - c[0];
            let dz = f64::from(z) + 0.5 - c[2];
            let left = r * r - dx * dx - dz * dz;
            if left < 0.0 {
                continue;
            }
            let half = left.sqrt();
            // the blocks whose centres lie inside the ball
            let (y0, y1) = ((c[1] - half - 0.5).ceil() as i32, (c[1] + half - 0.5).floor() as i32);
            if y1 < y0 {
                continue;
            }
            columns
                .entry((x, z))
                .and_modify(|(lo, hi)| {
                    *lo = (*lo).min(y0);
                    *hi = (*hi).max(y1);
                })
                .or_insert((y0, y1));
        }
    }
}

/// Clears the columns in one edit: of the blocks a power may break (`only_breakable`), or of
/// everything; never the bottom five layers of the world.
fn carve(host: &mut dyn ServerHost, level: LevelRef, dimension: &str, columns: &HashMap<(i32, i32), (i32, i32)>, only_breakable: bool) {
    if columns.is_empty() {
        return;
    }
    let Some(air) = host.block_state_id("minecraft:air") else {
        return;
    };
    let (min_y, height) = level.height(host).unwrap_or((-64, 384));
    let (floor, top) = (min_y + 5, min_y + height - 1);
    let filters = if only_breakable {
        let tags: Vec<String> = BREAKABLE.iter().map(|t| format!("#{t}")).collect();
        let names: Vec<&str> = tags.iter().map(String::as_str).collect();
        predicates(host, &names).unwrap_or_default()
    } else {
        Vec::new()
    };
    let passes: Vec<Option<u32>> = if filters.is_empty() { vec![None] } else { (0..filters.len() as u32).map(Some).collect() };
    let mut runs = Vec::with_capacity(columns.len() * passes.len());
    for (&(x, z), &(lo, hi)) in columns {
        let (lo, hi) = (lo.max(floor), hi.min(top));
        if hi < lo {
            continue;
        }
        for &only in &passes {
            runs.push(BlockRun { x, z, y: lo, height: (hi - lo + 1) as u32, surface: None, block: 0, only });
        }
    }
    let _ = host.edit_blocks(dimension, &BlockEdit { palette: vec![air], filters, runs });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ball_is_round() {
        let mut columns = HashMap::new();
        carve_sphere(&mut columns, [0.5, 64.5, 0.5], 3.0);
        assert_eq!(columns.get(&(0, 0)), Some(&(61, 67)));
        assert!(!columns.contains_key(&(3, 3)));
        assert_eq!(columns.keys().filter(|(_, z)| *z == 0).count(), 7);
    }

    #[test]
    fn the_cataclysm_spans_forty_chunks() {
        assert_eq!(powers::CATACLYSM_RADIUS * 2.0 / 16.0, 40.0);
    }
}
