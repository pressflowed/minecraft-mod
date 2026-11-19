//! The world's half: what each power does to blocks and mobs. Messages arrive in
//! [`message`], wait in a queue, and run in [`level_tick`] for their dimension, the moment
//! when that level may be changed.

use crate::flight;
use crate::net::{Act, Use};
use crate::storms::{self, Storm};
use crate::weather::EVENTS;
use horizon_client::mod_api::{Compound, Host, NativeBlockEdit, NativeBlockRun, Tag};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// Block tags a power may break: every block a tool mines, plants, glass and wool. Bedrock,
/// barriers, portals and command blocks are in none of them.
pub(crate) const BREAKABLE: [&str; 8] = [
    "#minecraft:mineable/pickaxe",
    "#minecraft:mineable/shovel",
    "#minecraft:mineable/axe",
    "#minecraft:mineable/hoe",
    "#minecraft:replaceable",
    "#minecraft:leaves",
    "#minecraft:impermeable",
    "#minecraft:wool",
];

/// How far from the player a power may reach, in blocks.
const REACH: f64 = 220.0;
/// The Storm Scepter reaches as far as the player can see.
const WEATHER_REACH: f64 = 1100.0;
/// At most this many storms at once, so the server keeps up.
const MAX_STORMS: usize = 6;

#[derive(Clone, Debug)]
enum Job {
    Use(Use),
    Grab { player: u128, yaw: f32, power: Use },
    Lightning { at: [f64; 3], radius: f64 },
    Collapse { at: [f64; 3], radius: f64 },
    /// Falling blocks the Gravity Gun let go of, each sent off along `dir` with its own jitter.
    Launch { blocks: Vec<(u32, [f64; 3])>, dir: [f64; 3] },
    /// One band of a crater (the Cataclysm's, a nuke's, a meteor's), `inner` to `outer` blocks
    /// out of one `radius` across, `depth` deep in the middle, cleared `clear` blocks up.
    Crater { at: [f64; 3], inner: f64, outer: f64, radius: f64, depth: f64, clear: f64 },
}

/// What a player's Gravity Gun holds: falling blocks by id, and where each sits in the cube.
struct Grip {
    dimension: String,
    blocks: Vec<(u32, [f64; 3])>,
    size: u8,
    /// The player's yaw when they grabbed it: the cube turns as they turn.
    yaw: f32,
}

/// Blocks a Gravity Gun flung, hurting what they hit for a while.
struct Flung {
    dimension: String,
    blocks: Vec<u32>,
    dir: [f64; 3],
    until: u64,
}

struct Queued {
    due: u64,
    dimension: String,
    job: Job,
}

struct Well {
    dimension: String,
    at: [f64; 3],
    until: u64,
    singularity: bool,
}

pub(crate) struct Server {
    tick: u64,
    queue: Vec<Queued>,
    wells: Vec<Well>,
    /// Entity id to its dimension and kind, as the level told them.
    entities: HashMap<u32, (String, String)>,
    /// When each player last used each power, against a flood of messages.
    last: HashMap<(u128, u16), u64>,
    seed: u64,
    storms: Vec<Storm>,
    grips: HashMap<u128, Grip>,
    flung: Vec<Flung>,
}

static SERVER: Mutex<Option<Server>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<Server>> {
    SERVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn with<R>(f: impl FnOnce(&mut Server) -> R) -> R {
    let mut guard = state();
    let server = guard.get_or_insert_with(|| Server {
        tick: 0,
        queue: Vec::new(),
        wells: Vec::new(),
        entities: HashMap::new(),
        last: HashMap::new(),
        seed: 0x2545_F491_4F6C_DD1D,
        storms: Vec::new(),
        grips: HashMap::new(),
        flung: Vec::new(),
    });
    f(server)
}

pub(crate) fn rand(server: &mut Server) -> f64 {
    let mut x = server.seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    server.seed = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

pub fn tick() {
    with(|s| s.tick += 1);
}

pub fn joined(dimension: &str, entity: u32, kind: &str) {
    if kind == "minecraft:player" || kind == "minecraft:lightning_bolt" {
        return;
    }
    with(|s| {
        s.entities
            .insert(entity, (dimension.to_owned(), kind.to_owned()));
    });
}

pub fn left(entity: u32) {
    with(|s| {
        s.entities.remove(&entity);
    });
}

/// The fewest ticks between two uses of a power by one player.
fn spacing(act: Act) -> u64 {
    match act {
        Act::Laser => 1,
        Act::Dash => 10,
        Act::Punch => 18,
        Act::Thunder => 50,
        Act::Frost => 70,
        Act::Gravity => 150,
        Act::TitanFist => 150,
        Act::Omega => 1,
        Act::Wrath => 230,
        Act::AbsoluteZero => 270,
        Act::Singularity => 390,
        Act::Cataclysm => 590,
        Act::Weather => 60,
        Act::Grab | Act::Fling | Act::Drop => 3,
        Act::Give => 20,
    }
}

pub fn message(host: &mut dyn Host, player: u128, data: &[u8]) {
    let Some(power) = Use::decode(data) else {
        return;
    };
    // Where the player is, when the server lists them: a power works near its user only.
    let players = host.server_players();
    let me = players.iter().find(|p| p.uuid == player);
    if let Some(p) = me {
        let reach = if power.act == Act::Weather { WEATHER_REACH } else { REACH };
        if p.dimension != power.dimension || distance(p.position, power.at) > reach {
            return;
        }
    }
    let name = me.map(|p| p.name.clone());
    let yaw = me.map_or(0.0, |p| p.yaw);
    let go = with(|s| {
        // each Storm Scepter event keeps its own time
        let which = if power.act == Act::Weather { 100 + u16::from(power.level) } else { power.act as u16 };
        let key = (player, which);
        let spacing = match power.act {
            Act::Weather => (EVENTS[usize::from(power.level)].cooldown * 20.0) as u64 - 10,
            act => spacing(act),
        };
        if s.last.get(&key).is_some_and(|&t| s.tick < t + spacing) {
            return false;
        }
        s.last.insert(key, s.tick);
        true
    });
    if !go {
        return;
    }
    match power.act {
        // the Gravity Gun and the give command act at once; the rest wait for their level's tick
        Act::Give => give(host, name.as_deref()),
        Act::Fling => fling_grip(player, power.dir),
        Act::Drop => drop_grip(player),
        Act::Grab => with(|s| {
            let (due, dimension) = (s.tick, power.dimension.clone());
            s.queue.push(Queued { due, dimension, job: Job::Grab { player, yaw, power } });
        }),
        _ => with(|s| {
            let (due, dimension) = (s.tick, power.dimension.clone());
            s.queue.push(Queued { due, dimension, job: Job::Use(power) });
        }),
    }
}

/// `/superpowers give`: the Storm Scepter and the Gravity Gun, named and shining.
fn give(host: &mut dyn Host, name: Option<&str>) {
    let Some(name) = name else {
        return;
    };
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return;
    }
    for (item, label, color) in [("minecraft:breeze_rod", "Storm Scepter", "aqua"), ("minecraft:echo_shard", "Gravity Gun", "gold")] {
        let lines = [
            format!("give {name} {item}[minecraft:custom_name={{text:\"{label}\",color:\"{color}\",italic:false}},minecraft:enchantment_glint_override=true]"),
            format!("give {name} {item}[minecraft:custom_name=\"{label}\"]"),
            format!("give {name} {item}"),
        ];
        for line in lines {
            if host.run_command(&line) {
                break;
            }
        }
    }
}

pub fn level_tick(host: &mut dyn Host, dimension: &str) {
    let (jobs, wells) = with(|s| {
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
        let wells: Vec<([f64; 3], bool, u64)> = s
            .wells
            .iter()
            .filter(|w| w.dimension == dimension)
            .map(|w| (w.at, w.singularity, w.until - now))
            .collect();
        (jobs, wells)
    });
    for job in jobs {
        run(host, dimension, job);
    }
    for (at, singularity, left) in wells {
        pull(host, dimension, at, singularity, left);
    }
    // the storms of this level, each a tick on
    let mut mine: Vec<Storm> = with(|s| {
        let (here, elsewhere) = std::mem::take(&mut s.storms).into_iter().partition(|st| st.dimension == dimension);
        s.storms = elsewhere;
        here
    });
    for storm in &mut mine {
        storms::step(host, storm);
    }
    mine.retain(|st| !st.done());
    with(|s| s.storms.extend(mine));
    hold(host, dimension);
}

pub(crate) fn queue_lightning(dimension: &str, at: [f64; 3], radius: f64) {
    with(|s| {
        let due = s.tick + 1;
        s.queue.push(Queued { due, dimension: dimension.to_owned(), job: Job::Lightning { at, radius } });
    });
}

/// A crater `radius` across and `depth` deep, cut in bands `band` blocks wide, one a tick.
pub(crate) fn queue_crater(dimension: &str, at: [f64; 3], radius: f64, depth: f64, clear: f64, band: f64) {
    with(|s| {
        let mut inner = 0.0;
        let mut due = s.tick + 1;
        while inner < radius {
            let outer = (inner + band).min(radius);
            s.queue.push(Queued {
                due,
                dimension: dimension.to_owned(),
                job: Job::Crater { at, inner, outer, radius, depth, clear },
            });
            inner = outer;
            due += 1;
        }
    });
}

// ---- the Gravity Gun ----------------------------------------------------------------------------

/// How far in front of the eyes the gun holds a cube `size` blocks across.
pub fn hold_distance(size: u8) -> f64 {
    3.0 + f64::from(size) * 1.25
}

/// Lifts the cube `size` blocks across from its lowest corner `at`: each block that can move
/// becomes a falling block the gun holds.
fn grab(host: &mut dyn Host, dimension: &str, player: u128, yaw: f32, at: [f64; 3], size: u8) {
    if with(|s| s.grips.contains_key(&player)) || !matches!(size, 1 | 2 | 4 | 8) {
        return;
    }
    let base = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
    let n = i32::from(size);
    let mut found = Vec::new();
    for dx in 0..n {
        for dy in 0..n {
            for dz in 0..n {
                let pos = [base[0] + dx, base[1] + dy, base[2] + dz];
                let block = host.block(pos);
                if throwable(&block) && !block.contains("water") && !block.contains("lava") {
                    found.push((pos, block));
                }
            }
        }
    }
    if found.is_empty() {
        return;
    }
    let runs = found
        .iter()
        .map(|(p, _)| NativeBlockRun { x: p[0], z: p[2], y: p[1], height: 1, surface: None, block: 0, only: None })
        .collect();
    let _ = host.edit_blocks(dimension, &NativeBlockEdit { palette: vec!["minecraft:air".into()], filters: Vec::new(), runs });
    let half = f64::from(n) / 2.0;
    let center = [f64::from(base[0]) + half, f64::from(base[1]) + half - 0.5, f64::from(base[2]) + half];
    let mut blocks = Vec::new();
    for (pos, block) in found {
        let spot = [f64::from(pos[0]) + 0.5, f64::from(pos[1]), f64::from(pos[2]) + 0.5];
        let mut state = Compound::new();
        state.insert("Name".into(), Tag::String(block.as_str().into()));
        let mut nbt = Compound::new();
        nbt.insert("BlockState".into(), Tag::Compound(state));
        // a falling block left in the air gives up after 30 seconds; this one is held for years
        nbt.insert("Time".into(), Tag::Int(-2_000_000_000));
        nbt.insert("DropItem".into(), Tag::Byte(0));
        nbt.insert("HurtEntities".into(), Tag::Byte(1));
        if let Some(id) = host.summon(dimension, "minecraft:falling_block", spot, nbt) {
            blocks.push((id, sub(spot, center)));
        }
    }
    if blocks.is_empty() {
        return;
    }
    command(host, dimension, center, "playsound minecraft:block.beacon.power_select master @a[distance=..32] ~ ~ ~ 1 1.6");
    command(host, dimension, center, "particle minecraft:reverse_portal ~ ~ ~ 1 1 1 0.3 60");
    with(|s| {
        s.grips.insert(player, Grip { dimension: dimension.to_owned(), blocks, size, yaw });
    });
}

/// Each tick: every held cube is drawn to the point in front of its player's eyes.
fn hold(host: &mut dyn Host, dimension: &str) {
    let grips: Vec<(u128, Vec<(u32, [f64; 3])>, u8, f32)> = with(|s| {
        s.grips
            .iter()
            .filter(|(_, g)| g.dimension == dimension)
            .map(|(p, g)| (*p, g.blocks.clone(), g.size, g.yaw))
            .collect()
    });
    let flung: Vec<(Vec<u32>, [f64; 3])> = with(|s| {
        let now = s.tick;
        s.flung.retain(|f| f.until > now);
        s.flung.iter().filter(|f| f.dimension == dimension).map(|f| (f.blocks.clone(), f.dir)).collect()
    });
    let tick = with(|s| s.tick);
    if !flung.is_empty() && tick % 2 == 0 {
        // what the flung blocks hit is hurt and knocked along
        for (blocks, dir) in flung {
            for &id in blocks.iter().take(40) {
                let Some(pos) = host.entity(id).and_then(|e| host.entity_position(e)) else {
                    continue;
                };
                command(host, dimension, pos, "damage @e[type=!minecraft:player,type=!minecraft:falling_block,distance=..1.8] 7 minecraft:falling_block");
                fling_some(host, dimension, pos, 1.8, false, |_, _| [dir[0] * 1.2, 0.45, dir[2] * 1.2]);
            }
        }
    }
    if grips.is_empty() {
        return;
    }
    let players = host.server_players();
    for (player, blocks, size, yaw0) in grips {
        let Some(p) = players.iter().find(|p| p.uuid == player && p.dimension == dimension) else {
            drop_grip(player);
            continue;
        };
        let eye = [p.position[0], p.position[1] + 1.62, p.position[2]];
        let dir = flight::look(f64::from(p.yaw), f64::from(p.pitch));
        let hold = add(eye, dir, hold_distance(size));
        let turn = f64::from(p.yaw - yaw0).to_radians();
        let (c, sn) = (turn.cos(), turn.sin());
        let mut kept = Vec::with_capacity(blocks.len());
        for (i, (id, offset)) in blocks.into_iter().enumerate() {
            let Some(entity) = host.entity(id) else {
                continue;
            };
            let Some(pos) = host.entity_position(entity) else {
                continue;
            };
            let turned = [offset[0] * c - offset[2] * sn, offset[1], offset[0] * sn + offset[2] * c];
            let bob = (tick as f64 * 0.15 + i as f64 * 0.7).sin() * 0.04;
            let target = [hold[0] + turned[0], hold[1] + turned[1] + bob, hold[2] + turned[2]];
            let mut v = sub(target, pos).map(|d| d * 0.45);
            let speed = length(v);
            if speed > 2.5 {
                v = v.map(|d| d * 2.5 / speed);
            }
            // a falling block's gravity comes off first in its next tick
            v[1] += 0.04;
            let _ = host.entity_set_velocity(entity, v);
            kept.push((id, offset));
        }
        with(|s| {
            if kept.is_empty() {
                s.grips.remove(&player);
            } else if let Some(g) = s.grips.get_mut(&player) {
                g.blocks = kept;
            }
        });
    }
}

/// The Gravity Gun lets fly: every held block goes off along `dir`.
fn fling_grip(player: u128, dir: [f64; 3]) {
    let Some(grip) = with(|s| s.grips.remove(&player)) else {
        return;
    };
    with(|s| {
        let until = s.tick + 60;
        s.flung.push(Flung { dimension: grip.dimension.clone(), blocks: grip.blocks.iter().map(|b| b.0).collect(), dir, until });
        let spin = with_jitter(s, grip.blocks.len());
        s.queue.push(Queued {
            due: s.tick,
            dimension: grip.dimension.clone(),
            job: Job::Launch { blocks: grip.blocks.iter().map(|b| b.0).zip(spin).collect(), dir },
        });
    });
}

fn with_jitter(s: &mut Server, count: usize) -> Vec<[f64; 3]> {
    (0..count).map(|_| [rand(s) - 0.5, rand(s) - 0.5, rand(s) - 0.5]).collect()
}

/// The Gravity Gun lets go: the blocks fall where they are and land.
fn drop_grip(player: u128) {
    if let Some(grip) = with(|s| s.grips.remove(&player)) {
        with(|s| {
            s.queue.push(Queued {
                due: s.tick,
                dimension: grip.dimension.clone(),
                job: Job::Launch { blocks: grip.blocks.iter().map(|b| (b.0, [0.0; 3])).collect(), dir: [0.0, -0.1, 0.0] },
            });
        });
    }
}

fn run(host: &mut dyn Host, dimension: &str, job: Job) {
    match job {
        Job::Grab { player, yaw, power } => grab(host, dimension, player, yaw, power.at, power.level),
        Job::Launch { blocks, dir } => {
            let speed = if dir[1] < -0.05 && dir[0] == 0.0 { 0.0 } else { 3.2 };
            for (id, jitter) in blocks {
                if let Some(entity) = host.entity(id) {
                    let v = if speed == 0.0 {
                        dir
                    } else {
                        [dir[0] * speed + jitter[0] * 0.25, dir[1] * speed + jitter[1] * 0.25 + 0.1, dir[2] * speed + jitter[2] * 0.25]
                    };
                    let _ = host.entity_set_velocity(entity, v);
                }
            }
        }
        Job::Use(power) => match power.act {
            Act::Punch => punch(host, dimension, power.at, power.dir, 1.0),
            Act::TitanFist => punch(host, dimension, power.at, power.dir, 5.0),
            Act::Laser => laser(host, dimension, power.at, power.level),
            Act::Omega => omega(host, dimension, power.at, power.dir),
            Act::Thunder => thunder(dimension, power.at, power.level, false),
            Act::Wrath => thunder(dimension, power.at, power.level, true),
            Act::Frost => frost(host, dimension, power.at, 14),
            Act::AbsoluteZero => frost(host, dimension, power.at, 70),
            Act::Gravity | Act::Singularity => {
                let singularity = power.act == Act::Singularity;
                let ticks = if singularity { 160 } else { 100 };
                with(|s| {
                    let until = s.tick + ticks;
                    s.wells.push(Well { dimension: dimension.to_owned(), at: power.at, until, singularity });
                    s.queue.push(Queued {
                        due: until,
                        dimension: dimension.to_owned(),
                        job: Job::Collapse { at: power.at, radius: if singularity { 26.0 } else { 4.5 } },
                    });
                });
                command(host, dimension, power.at, "playsound minecraft:block.portal.trigger master @a[distance=..64] ~ ~ ~ 2 0.5");
            }
            Act::Dash => dash(host, dimension, power.at, power.dir),
            Act::Weather => {
                let event = usize::from(power.level);
                let full = with(|s| s.storms.len() >= MAX_STORMS);
                if !full {
                    let storm = Storm::new(dimension, event, power.at, power.dir, power.value);
                    with(|s| s.storms.push(storm));
                }
            }
            Act::Grab | Act::Fling | Act::Drop | Act::Give => {}
            Act::Cataclysm => {
                let r = crate::client::CATACLYSM_RADIUS;
                queue_crater(dimension, power.at, r, 90.0, 120.0, 10.0);
                with(|s| {
                    for k in 0..14u64 {
                        let a = rand(s) * std::f64::consts::TAU;
                        let d = 40.0 + rand(s) * (r - 60.0);
                        let at = [power.at[0] + a.cos() * d, power.at[1], power.at[2] + a.sin() * d];
                        s.queue.push(Queued { due: s.tick + 6 + k * 5, dimension: dimension.to_owned(), job: Job::Lightning { at, radius: 2.0 } });
                    }
                });
            }
        },
        Job::Lightning { at, radius } => {
            command(host, dimension, at, "summon minecraft:lightning_bolt ~ ~ ~");
            command(host, dimension, at, &format!("damage @e[type=!minecraft:player,distance=..{:.1}] 12 minecraft:lightning_bolt", radius + 3.0));
            let mut columns = HashMap::new();
            carve_sphere(&mut columns, at, radius);
            carve(host, dimension, &columns);
        }
        Job::Collapse { at, radius } => {
            let mut columns = HashMap::new();
            carve_sphere(&mut columns, at, radius);
            carve(host, dimension, &columns);
            command(host, dimension, at, &format!("damage @e[type=!minecraft:player,distance=..{:.0}] {:.0} minecraft:explosion", radius * 2.0, 30.0 + radius * 4.0));
            fling(host, dimension, at, radius * 3.5, |offset, d| {
                let k = (2.5 / d.max(1.0)).min(1.5);
                [offset[0] * k, 1.0, offset[2] * k]
            });
        }
        Job::Crater { at, inner, outer, radius, depth, clear } => crater_band(host, dimension, at, inner, outer, radius, depth, clear),
    }
}

/// The Omega Beam, every other tick while it fires: a tunnel 4 blocks across, 200 long.
fn omega(host: &mut dyn Host, dimension: &str, eye: [f64; 3], dir: [f64; 3]) {
    let mut columns = HashMap::new();
    let mut step = 3.0;
    while step < 200.0 {
        carve_sphere(&mut columns, add(eye, dir, step), 2.2);
        step += 2.0;
    }
    carve(host, dimension, &columns);
    for s in [20.0, 60.0, 100.0, 140.0, 180.0] {
        command(host, dimension, add(eye, dir, s), "damage @e[type=!minecraft:player,distance=..20] 12 minecraft:in_fire");
    }
}

/// One band of a crater `r` across, `inner` to `outer` blocks from ground zero: a bowl `deep`
/// blocks deep at the centre, shallowing to the rim, cleared of everything up to `clear` blocks
/// over the ground, floored with scorched stone. Unloaded chunks are left as they are.
#[allow(clippy::too_many_arguments)]
fn crater_band(host: &mut dyn Host, dimension: &str, at: [f64; 3], inner: f64, outer: f64, r: f64, deep: f64, clear: f64) {
    let (cx, cz) = (at[0].floor() as i32, at[2].floor() as i32);
    let o = outer.ceil() as i32;
    let mut runs = Vec::new();
    let mut seed = (inner as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let surface = Some(horizon_client::mod_api::NativeHeightmap::WorldSurface);
    for dx in -o..=o {
        for dz in -o..=o {
            let d = f64::from(dx * dx + dz * dz).sqrt();
            if d < inner || d >= outer {
                continue;
            }
            let t = d / r;
            let depth = (deep * (1.0 - t * t)).max(2.0) as i32;
            let (x, z) = (cx + dx, cz + dz);
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let floor = match seed % 7 {
                0 => 2,
                1 | 2 => 3,
                _ => 1,
            };
            runs.push(NativeBlockRun { x, z, y: -depth - 1, height: 1, surface, block: floor, only: None });
            runs.push(NativeBlockRun { x, z, y: -depth, height: (depth + clear as i32) as u32, surface, block: 0, only: None });
        }
    }
    let edit = NativeBlockEdit {
        palette: vec!["minecraft:air".into(), "minecraft:blackstone".into(), "minecraft:magma_block".into(), "minecraft:basalt".into()],
        filters: Vec::new(),
        runs,
    };
    let _ = host.edit_blocks(dimension, &edit);
    let mid = (inner + outer) / 2.0;
    let line = format!(
        "damage @e[type=!minecraft:player,distance={inner:.0}..{outer:.0}] {:.0} minecraft:explosion",
        (200.0 * deep / 90.0).max(60.0) * (1.0 - mid / r).max(0.25)
    );
    command(host, dimension, at, &line);
    fling(host, dimension, at, outer + 2.0, |offset, _| {
        let d = (offset[0] * offset[0] + offset[2] * offset[2]).sqrt();
        if d < inner - 2.0 {
            return [0.0; 3];
        }
        let k = 3.0 / d.max(1.0);
        [offset[0] * k, 1.5, offset[2] * k]
    });
}

/// Super Punch (`k` 1) and Titan Fist (`k` 5): a cone that widens as it goes shatters, a share
/// of it flies away as falling blocks, and the mobs in it are hurt and thrown.
fn punch(host: &mut dyn Host, dimension: &str, eye: [f64; 3], dir: [f64; 3], k: f64) {
    let mut columns = HashMap::new();
    let mut debris = Vec::new();
    let length = (16.0 * k) as i32;
    for step in 1..=length {
        let s = step as f64;
        let center = add(eye, dir, s);
        let radius = 1.6 + s * 0.3;
        carve_sphere(&mut columns, center, radius);
        // A few blocks of each slice fly off.
        for _ in 0..usize::from(step % (k as i32).max(1) == 0) * 2 {
            let r = with(rand) * radius;
            let a = with(rand) * std::f64::consts::TAU;
            let h = (with(rand) - 0.5) * radius;
            debris.push([center[0] + a.cos() * r, center[1] + h, center[2] + a.sin() * r]);
        }
    }
    // The blocks that fly are read before the cone is cleared.
    for pos in debris {
        let block_pos = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
        let block = host.block(block_pos);
        if !throwable(&block) {
            continue;
        }
        let spread = [(with(rand) - 0.5) * 0.8, with(rand) * 0.6 + 0.5, (with(rand) - 0.5) * 0.8];
        let velocity = [dir[0] * 1.6 + spread[0], dir[1] * 1.0 + spread[1], dir[2] * 1.6 + spread[2]];
        throw(host, dimension, &block, pos, velocity);
    }
    carve(host, dimension, &columns);
    for f in [0.2, 0.5, 0.8] {
        let s = 16.0 * k * f;
        let at = add(eye, dir, s);
        command(host, dimension, at, &format!("damage @e[type=!minecraft:player,distance=..{:.1}] {:.0} minecraft:explosion", 2.0 + s * 0.35, 26.0 * k));
    }
    let push = 3.0 * k.sqrt();
    fling(host, dimension, eye, 16.0 * k + 2.0, |offset, d| {
        let along = offset[0] * dir[0] + offset[1] * dir[1] + offset[2] * dir[2];
        if along < 0.0 || d - along > 2.0 + along * 0.4 {
            return [0.0; 3];
        }
        [dir[0] * push, 1.0 + dir[1].max(0.0) * 2.0, dir[2] * push]
    });
}

/// Laser Eyes: the beams melt what they touch and set mobs alight; stronger beams cut wider.
fn laser(host: &mut dyn Host, dimension: &str, at: [f64; 3], level: u8) {
    let power = f64::from(level);
    let mut columns = HashMap::new();
    carve_sphere(&mut columns, at, 0.45 + power * 0.32);
    carve(host, dimension, &columns);
    let reach = 1.2 + power * 0.35;
    command(host, dimension, at, &format!("damage @e[type=!minecraft:player,distance=..{reach:.1}] {:.1} minecraft:in_fire", 1.5 + power * 1.5));
    command(host, dimension, at, &format!("execute as @e[type=!minecraft:player,distance=..{reach:.1},limit=12] run data merge entity @s {{Fire:100s}}"));
    if level >= 4 {
        command(host, dimension, at, "particle minecraft:lava ~ ~ ~ 0.3 0.3 0.3 0 3");
    }
}

/// Thunder Strike: one bolt on the target, then six around it, three ticks apart.
fn thunder(dimension: &str, at: [f64; 3], seed: u8, wrath: bool) {
    let (count, radius, ticks) = if wrath { (36, 46.0, 70) } else { (7, 8.0, 18) };
    with(|s| {
        let now = s.tick;
        for (offset, tick) in crate::client::bolts(seed, count, radius, ticks) {
            let bolt = [at[0] + offset[0], at[1], at[2] + offset[1]];
            let job = Job::Lightning { at: bolt, radius: if wrath { 3.0 } else { 1.6 } };
            s.queue.push(Queued { due: now + u64::from(tick), dimension: dimension.to_owned(), job });
        }
    });
}


/// Frost Nova: water freezes, lava turns to obsidian and snow settles in a circle of 14
/// blocks; the mobs in it are frozen, slowed and hurt.
fn frost(host: &mut dyn Host, dimension: &str, feet: [f64; 3], radius: i32) {
    let mut runs = Vec::new();
    let (cx, cz) = (feet[0].floor() as i32, feet[2].floor() as i32);
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz > radius * radius {
                continue;
            }
            let (x, z) = (cx + dx, cz + dz);
            let surface = Some(horizon_client::mod_api::NativeHeightmap::MotionBlocking);
            // The top block (y -1 from the surface height): water to ice, lava to obsidian.
            runs.push(NativeBlockRun { x, z, y: -1, height: 1, surface, block: 1, only: Some(1) });
            runs.push(NativeBlockRun { x, z, y: -1, height: 1, surface, block: 2, only: Some(2) });
            // The air above it: a layer of snow.
            runs.push(NativeBlockRun { x, z, y: 0, height: 1, surface, block: 0, only: Some(0) });
        }
    }
    if radius > 30 {
        // ice spikes rise across the frozen land
        for _ in 0..40 {
            let a = with(rand) * std::f64::consts::TAU;
            let d = 6.0 + with(rand) * f64::from(radius - 8);
            let (x, z) = (cx + (a.cos() * d) as i32, cz + (a.sin() * d) as i32);
            let tall = 5 + (with(rand) * 16.0) as u32;
            let block = if with(rand) < 0.3 { 4 } else { 3 };
            let surface = Some(horizon_client::mod_api::NativeHeightmap::MotionBlocking);
            for (ox, oz, h) in [(0, 0, tall), (1, 0, tall * 2 / 3), (0, 1, tall * 2 / 3), (-1, 0, tall / 2), (0, -1, tall / 2)] {
                runs.push(NativeBlockRun { x: x + ox, z: z + oz, y: 0, height: h, surface, block, only: Some(0) });
            }
        }
    }
    let edit = NativeBlockEdit {
        palette: vec!["minecraft:snow".into(), "minecraft:ice".into(), "minecraft:obsidian".into(), "minecraft:packed_ice".into(), "minecraft:blue_ice".into()],
        filters: vec!["minecraft:air".into(), "minecraft:water".into(), "minecraft:lava".into()],
        runs,
    };
    let _ = host.edit_blocks(dimension, &edit);
    let r = radius + 1;
    let (secs, damage) = if radius > 30 { (15, 20) } else { (8, 6) };
    command(host, dimension, feet, &format!("effect give @e[type=!minecraft:player,distance=..{r}] minecraft:slowness {secs} 5"));
    command(host, dimension, feet, &format!("damage @e[type=!minecraft:player,distance=..{r}] {damage} minecraft:freeze"));
    command(host, dimension, feet, &format!("execute as @e[type=!minecraft:player,distance=..{r},limit=60] run data merge entity @s {{TicksFrozen:400}}"));
    command(host, dimension, feet, "particle minecraft:snowflake ~ ~1 ~ 6 1 6 0.05 400");
}

/// Hyper Dash: mobs along the 16 blocks ahead are struck and knocked aside.
fn dash(host: &mut dyn Host, dimension: &str, from: [f64; 3], dir: [f64; 3]) {
    for s in [3.0, 7.0, 11.0, 15.0] {
        let at = add(from, dir, s);
        command(host, dimension, at, "damage @e[type=!minecraft:player,distance=..2.5] 12 minecraft:generic");
    }
    fling(host, dimension, from, 18.0, |offset, _| {
        let along = offset[0] * dir[0] + offset[1] * dir[1] + offset[2] * dir[2];
        let side = sub(offset, [dir[0] * along, dir[1] * along, dir[2] * along]);
        let off = length(side);
        if !(0.0..=17.0).contains(&along) || off > 3.0 {
            return [0.0; 3];
        }
        let k = 1.6 / off.max(0.3);
        [side[0] * k + dir[0] * 0.8, 0.7, side[2] * k + dir[2] * 0.8]
    });
}

/// Gravity Well, each tick: everything within 24 blocks is pulled in and swirls, and now and
/// then a loose block is torn out of the ground and thrown in.
fn pull(host: &mut dyn Host, dimension: &str, at: [f64; 3], singularity: bool, left: u64) {
    let reach = if singularity { 120.0 } else { 24.0 };
    fling(host, dimension, at, reach, |offset, d| {
        if d < 0.8 {
            return [0.0; 3];
        }
        let inward = [-offset[0] / d, -offset[1] / d, -offset[2] / d];
        let swirl = [-inward[2], 0.0, inward[0]];
        let k = if singularity { 0.25 + 0.9 * (1.0 - d / reach).max(0.0) } else { 0.12 + 0.5 * (1.0 - d / reach).max(0.0) };
        [
            inward[0] * k + swirl[0] * k * 0.6,
            inward[1] * k + 0.04,
            inward[2] * k + swirl[2] * k * 0.6,
        ]
    });
    let tick = with(|s| s.tick);
    if singularity && tick % 4 == 0 {
        // the ball of land it has eaten grows from 3 to 24 blocks across eight seconds
        let eaten = 3.0 + 21.0 * (1.0 - left as f64 / 160.0).clamp(0.0, 1.0);
        let mut columns = HashMap::new();
        carve_sphere(&mut columns, at, eaten);
        carve(host, dimension, &columns);
    }
    if singularity || tick % 3 == 0 {
        let a = with(rand) * std::f64::consts::TAU;
        let r = if singularity { 10.0 + with(rand) * 60.0 } else { 4.0 + with(rand) * 10.0 };
        let x = (at[0] + a.cos() * r).floor() as i32;
        let z = (at[2] + a.sin() * r).floor() as i32;
        // The first solid block under the well's height there.
        let mut y = at[1].floor() as i32;
        let bottom = y - if singularity { 48 } else { 16 };
        while y > bottom {
            let block = host.block([x, y, z]);
            if throwable(&block) {
                let pos = [f64::from(x) + 0.5, f64::from(y) + 1.05, f64::from(z) + 0.5];
                let toward = sub(at, pos);
                let d = length(toward).max(1.0);
                let velocity = [toward[0] / d * 0.9, 0.9 + toward[1] / d * 0.4, toward[2] / d * 0.9];
                let mut columns = HashMap::new();
                columns.insert((x, z), (y, y));
                carve(host, dimension, &columns);
                throw(host, dimension, &block, pos, velocity);
                break;
            }
            y -= 1;
        }
    }
    if tick % 10 == 0 {
        let (r, damage) = if singularity { (10, 10) } else { (2, 4) };
        command(host, dimension, at, &format!("damage @e[type=!minecraft:player,distance=..{r}] {damage} minecraft:magic"));
        command(host, dimension, at, "particle minecraft:reverse_portal ~ ~ ~ 3 3 3 0.4 80");
    }
}

/// Pushes every tracked entity within `radius` of `at` in `dimension` by what `push` says for
/// its offset from `at` and its distance.
pub(crate) fn fling(host: &mut dyn Host, dimension: &str, at: [f64; 3], radius: f64, push: impl Fn([f64; 3], f64) -> [f64; 3]) {
    fling_some(host, dimension, at, radius, true, push);
}

/// [`fling`], leaving falling blocks out unless `blocks`.
fn fling_some(host: &mut dyn Host, dimension: &str, at: [f64; 3], radius: f64, blocks: bool, push: impl Fn([f64; 3], f64) -> [f64; 3]) {
    let ids: Vec<(u32, String)> = with(|s| {
        s.entities
            .iter()
            .filter(|(_, (dim, kind))| dim == dimension && (blocks || kind != "minecraft:falling_block"))
            .map(|(id, (_, kind))| (*id, kind.clone()))
            .collect()
    });
    for (id, kind) in ids {
        let Some(entity) = host.entity(id) else {
            continue;
        };
        let Some(pos) = host.entity_position(entity) else {
            continue;
        };
        let offset = sub(pos, at);
        let d = length(offset);
        if d > radius {
            continue;
        }
        let velocity = push(offset, d);
        if velocity == [0.0; 3] || velocity.iter().any(|v| !v.is_finite()) {
            continue;
        }
        if !host.push_mob(dimension, id, velocity) && kind != "minecraft:item_frame" && kind != "minecraft:painting" {
            let _ = host.entity_set_velocity(entity, velocity);
        }
    }
}

/// Blocks worth throwing: solid ones, not air, liquids or block entities' blocks.
pub(crate) fn throwable(block: &str) -> bool {
    !(block.is_empty()
        || block.ends_with("air")
        || block == "minecraft:water"
        || block == "minecraft:lava"
        || block == "minecraft:bedrock"
        || block.contains("chest")
        || block.contains("shulker")
        || block.contains("spawner")
        || block.contains("command")
        || block.contains("barrier")
        || block.contains("portal"))
}

/// A falling block of `block` thrown from `pos` at `velocity` blocks a tick; it lands and stays.
pub(crate) fn throw(host: &mut dyn Host, dimension: &str, block: &str, pos: [f64; 3], velocity: [f64; 3]) {
    let v = velocity.map(|v| v.clamp(-3.5, 3.5));
    let line = format!(
        "summon minecraft:falling_block ~ ~ ~ {{BlockState:{{Name:\"{block}\"}},Time:1,DropItem:0b,Motion:[{:.3}d,{:.3}d,{:.3}d]}}",
        v[0], v[1], v[2]
    );
    command(host, dimension, pos, &line);
}

/// Runs `line` as the console does, at `at` in `dimension`.
pub(crate) fn command(host: &mut dyn Host, dimension: &str, at: [f64; 3], line: &str) {
    let full = format!(
        "execute in {dimension} positioned {:.3} {:.3} {:.3} run {line}",
        at[0], at[1], at[2]
    );
    let _ = host.run_command(&full);
}

/// Adds a ball of radius `r` at `c` to the columns to clear: `(x, z)` to its lowest and highest y.
pub(crate) fn carve_sphere(columns: &mut HashMap<(i32, i32), (i32, i32)>, c: [f64; 3], r: f64) {
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

/// Clears the columns of every block a power may break, in one edit, keeping 5 blocks above
/// the bottom of the world.
pub(crate) fn carve(host: &mut dyn Host, dimension: &str, columns: &HashMap<(i32, i32), (i32, i32)>) {
    if columns.is_empty() {
        return;
    }
    let (min_y, height) = host.world_height();
    let floor = min_y + 5;
    let top = min_y + height - 1;
    let mut runs = Vec::with_capacity(columns.len() * BREAKABLE.len());
    for (&(x, z), &(lo, hi)) in columns {
        let (lo, hi) = (lo.max(floor), hi.min(top));
        if hi < lo {
            continue;
        }
        for filter in 0..BREAKABLE.len() as u32 {
            runs.push(NativeBlockRun {
                x,
                z,
                y: lo,
                height: (hi - lo + 1) as u32,
                surface: None,
                block: 0,
                only: Some(filter),
            });
        }
    }
    let edit = NativeBlockEdit {
        palette: vec!["minecraft:air".into()],
        filters: BREAKABLE.iter().map(|f| (*f).to_owned()).collect(),
        runs,
    };
    let _ = host.edit_blocks(dimension, &edit);
}

fn add(a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    length(sub(a, b))
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
        // as wide as it is tall: 7 blocks each way
        assert_eq!(columns.keys().filter(|(_, z)| *z == 0).count(), 7);
    }

    #[test]
    fn precious_blocks_stay_put() {
        assert!(throwable("minecraft:stone"));
        assert!(!throwable("minecraft:bedrock"));
        assert!(!throwable("minecraft:chest"));
        assert!(!throwable("minecraft:cave_air"));
    }
}
