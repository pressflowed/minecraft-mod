//! The world's half: what each power does to blocks and mobs. Messages arrive in
//! [`message`], wait in a queue, and run in [`level_tick`] for their dimension, the moment
//! when that level may be changed.

use crate::net::{Act, Use};
use horizon_client::mod_api::{Host, NativeBlockEdit, NativeBlockRun};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// Block tags a power may break: every block a tool mines, plants, glass and wool. Bedrock,
/// barriers, portals and command blocks are in none of them.
const BREAKABLE: [&str; 8] = [
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

#[derive(Clone, Debug)]
enum Job {
    Use(Use),
    Lightning { at: [f64; 3] },
    Collapse { at: [f64; 3] },
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
}

struct Server {
    tick: u64,
    queue: Vec<Queued>,
    wells: Vec<Well>,
    /// Entity id to its dimension and kind, as the level told them.
    entities: HashMap<u32, (String, String)>,
    /// When each player last used each power, against a flood of messages.
    last: HashMap<(u128, u8), u64>,
    seed: u64,
}

static SERVER: Mutex<Option<Server>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<Server>> {
    SERVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with<R>(f: impl FnOnce(&mut Server) -> R) -> R {
    let mut guard = state();
    let server = guard.get_or_insert_with(|| Server {
        tick: 0,
        queue: Vec::new(),
        wells: Vec::new(),
        entities: HashMap::new(),
        last: HashMap::new(),
        seed: 0x2545_F491_4F6C_DD1D,
    });
    f(server)
}

fn rand(server: &mut Server) -> f64 {
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
    }
}

pub fn message(host: &mut dyn Host, player: u128, data: &[u8]) {
    let Some(power) = Use::decode(data) else {
        return;
    };
    // Where the player is, when the server lists them: a power works near its user only.
    let players = host.server_players();
    if let Some(p) = players.iter().find(|p| p.uuid == player) {
        if p.dimension != power.dimension || distance(p.position, power.at) > REACH {
            return;
        }
    }
    with(|s| {
        let key = (player, power.act as u8);
        if s.last.get(&key).is_some_and(|&t| s.tick < t + spacing(power.act)) {
            return;
        }
        s.last.insert(key, s.tick);
        let (due, dimension) = (s.tick, power.dimension.clone());
        s.queue.push(Queued { due, dimension, job: Job::Use(power) });
    });
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
        let wells: Vec<[f64; 3]> = s
            .wells
            .iter()
            .filter(|w| w.dimension == dimension)
            .map(|w| w.at)
            .collect();
        (jobs, wells)
    });
    for job in jobs {
        run(host, dimension, job);
    }
    for at in wells {
        pull(host, dimension, at);
    }
}

fn run(host: &mut dyn Host, dimension: &str, job: Job) {
    match job {
        Job::Use(power) => match power.act {
            Act::Punch => punch(host, dimension, power.at, power.dir),
            Act::Laser => laser(host, dimension, power.at, power.level),
            Act::Thunder => thunder(dimension, power.at),
            Act::Frost => frost(host, dimension, power.at),
            Act::Gravity => {
                with(|s| {
                    let until = s.tick + 100;
                    s.wells.push(Well { dimension: dimension.to_owned(), at: power.at, until });
                    s.queue.push(Queued {
                        due: until,
                        dimension: dimension.to_owned(),
                        job: Job::Collapse { at: power.at },
                    });
                });
                command(host, dimension, power.at, "playsound minecraft:block.portal.trigger master @a[distance=..64] ~ ~ ~ 2 0.5");
            }
            Act::Dash => dash(host, dimension, power.at, power.dir),
        },
        Job::Lightning { at } => {
            command(host, dimension, at, "summon minecraft:lightning_bolt ~ ~ ~");
            command(host, dimension, at, "damage @e[type=!minecraft:player,distance=..4] 10 minecraft:lightning_bolt");
            let mut columns = HashMap::new();
            carve_sphere(&mut columns, at, 1.6);
            carve(host, dimension, &columns);
        }
        Job::Collapse { at } => {
            let mut columns = HashMap::new();
            carve_sphere(&mut columns, at, 4.5);
            carve(host, dimension, &columns);
            command(host, dimension, at, "damage @e[type=!minecraft:player,distance=..9] 24 minecraft:explosion");
            fling(host, dimension, at, 14.0, |offset, d| {
                let k = 2.2 / d.max(1.0);
                [offset[0] * k, 0.9, offset[2] * k]
            });
        }
    }
}

/// Super Punch: a cone 16 blocks long that widens from 1.6 to 6.5 blocks shatters, a share of
/// it flies away as falling blocks, and the mobs in it are hurt and thrown.
fn punch(host: &mut dyn Host, dimension: &str, eye: [f64; 3], dir: [f64; 3]) {
    let mut columns = HashMap::new();
    let mut debris = Vec::new();
    for step in 1..=16 {
        let s = step as f64;
        let center = add(eye, dir, s);
        let radius = 1.6 + s * 0.3;
        carve_sphere(&mut columns, center, radius);
        // A few blocks of each slice fly off.
        for _ in 0..2 {
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
    for s in [3.0, 8.0, 13.0] {
        let at = add(eye, dir, s);
        command(host, dimension, at, &format!("damage @e[type=!minecraft:player,distance=..{:.1}] 26 minecraft:explosion", 2.0 + s * 0.35));
    }
    fling(host, dimension, eye, 18.0, |offset, d| {
        let along = offset[0] * dir[0] + offset[1] * dir[1] + offset[2] * dir[2];
        if along < 0.0 || d - along > 2.0 + along * 0.4 {
            return [0.0; 3];
        }
        [dir[0] * 3.0, 1.0 + dir[1].max(0.0) * 2.0, dir[2] * 3.0]
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
fn thunder(dimension: &str, at: [f64; 3]) {
    with(|s| {
        let now = s.tick;
        s.queue.push(Queued { due: now, dimension: dimension.to_owned(), job: Job::Lightning { at } });
        for k in 0..6u64 {
            let angle = k as f64 / 6.0 * std::f64::consts::TAU + rand(s) * 0.6;
            let radius = 5.0 + rand(s) * 4.0;
            let bolt = [at[0] + angle.cos() * radius, at[1], at[2] + angle.sin() * radius];
            s.queue.push(Queued { due: now + 3 * (k + 1), dimension: dimension.to_owned(), job: Job::Lightning { at: bolt } });
        }
    });
}

/// Frost Nova: water freezes, lava turns to obsidian and snow settles in a circle of 14
/// blocks; the mobs in it are frozen, slowed and hurt.
fn frost(host: &mut dyn Host, dimension: &str, feet: [f64; 3]) {
    let radius = 14i32;
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
    let edit = NativeBlockEdit {
        palette: vec!["minecraft:snow".into(), "minecraft:ice".into(), "minecraft:obsidian".into()],
        filters: vec!["minecraft:air".into(), "minecraft:water".into(), "minecraft:lava".into()],
        runs,
    };
    let _ = host.edit_blocks(dimension, &edit);
    command(host, dimension, feet, "effect give @e[type=!minecraft:player,distance=..15] minecraft:slowness 8 5");
    command(host, dimension, feet, "damage @e[type=!minecraft:player,distance=..15] 6 minecraft:freeze");
    command(host, dimension, feet, "execute as @e[type=!minecraft:player,distance=..15,limit=40] run data merge entity @s {TicksFrozen:400}");
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
fn pull(host: &mut dyn Host, dimension: &str, at: [f64; 3]) {
    fling(host, dimension, at, 24.0, |offset, d| {
        if d < 0.8 {
            return [0.0; 3];
        }
        let inward = [-offset[0] / d, -offset[1] / d, -offset[2] / d];
        let swirl = [-inward[2], 0.0, inward[0]];
        let k = 0.12 + 0.5 * (1.0 - d / 24.0).max(0.0);
        [
            inward[0] * k + swirl[0] * k * 0.6,
            inward[1] * k + 0.04,
            inward[2] * k + swirl[2] * k * 0.6,
        ]
    });
    let tick = with(|s| s.tick);
    if tick % 3 == 0 {
        let a = with(rand) * std::f64::consts::TAU;
        let r = 4.0 + with(rand) * 10.0;
        let x = (at[0] + a.cos() * r).floor() as i32;
        let z = (at[2] + a.sin() * r).floor() as i32;
        // The first solid block under the well's height there.
        let mut y = at[1].floor() as i32;
        let bottom = y - 16;
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
        command(host, dimension, at, "damage @e[type=!minecraft:player,distance=..2] 4 minecraft:magic");
        command(host, dimension, at, "particle minecraft:reverse_portal ~ ~ ~ 3 3 3 0.4 80");
    }
}

/// Pushes every tracked entity within `radius` of `at` in `dimension` by what `push` says for
/// its offset from `at` and its distance.
fn fling(host: &mut dyn Host, dimension: &str, at: [f64; 3], radius: f64, push: impl Fn([f64; 3], f64) -> [f64; 3]) {
    let ids: Vec<(u32, String)> = with(|s| {
        s.entities
            .iter()
            .filter(|(_, (dim, _))| dim == dimension)
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
fn throwable(block: &str) -> bool {
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
fn throw(host: &mut dyn Host, dimension: &str, block: &str, pos: [f64; 3], velocity: [f64; 3]) {
    let v = velocity.map(|v| v.clamp(-3.5, 3.5));
    let line = format!(
        "summon minecraft:falling_block ~ ~ ~ {{BlockState:{{Name:\"{block}\"}},Time:1,DropItem:0b,Motion:[{:.3}d,{:.3}d,{:.3}d]}}",
        v[0], v[1], v[2]
    );
    command(host, dimension, pos, &line);
}

/// Runs `line` as the console does, at `at` in `dimension`.
fn command(host: &mut dyn Host, dimension: &str, at: [f64; 3], line: &str) {
    let full = format!(
        "execute in {dimension} positioned {:.3} {:.3} {:.3} run {line}",
        at[0], at[1], at[2]
    );
    let _ = host.run_command(&full);
}

/// Adds a ball of radius `r` at `c` to the columns to clear: `(x, z)` to its lowest and highest y.
fn carve_sphere(columns: &mut HashMap<(i32, i32), (i32, i32)>, c: [f64; 3], r: f64) {
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
fn carve(host: &mut dyn Host, dimension: &str, columns: &HashMap<(i32, i32), (i32, i32)>) {
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
