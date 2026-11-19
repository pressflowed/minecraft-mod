//! Jets, on the server's side: where each pilot's player is (so the window can keep them in the
//! seat), the missile lock, the cannon's rounds, missiles that home on their lock, bombs that
//! fall, and the explosions that leave craters.

use crate::jet_net::{self, Down, Shot, Up};
use crate::server;
use crate::host::Host;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

struct Pilot {
    dimension: String,
    eye: [f64; 3],
    fwd: [f64; 3],
    seen: u64,
    lock: Option<u32>,
    locked_for: u32,
}

struct Ordnance {
    id: u32,
    dimension: String,
    pos: [f64; 3],
    vel: [f64; 3],
    missile: bool,
    big: bool,
    target: Option<u32>,
    age: u32,
}

struct Gun {
    dimension: String,
    from: [f64; 3],
    dir: [f64; 3],
    count: u8,
}

#[derive(Default)]
struct State {
    pilots: HashMap<u128, Pilot>,
    ordnance: Vec<Ordnance>,
    guns: Vec<Gun>,
    next: u32,
    seed: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard: MutexGuard<'_, Option<State>> = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(|| State { seed: 0x9E37_79B9_7F4A_7C15, ..Default::default() }))
}

fn rand(s: &mut State) -> f64 {
    let mut x = s.seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    s.seed = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn add(a: [f64; 3], b: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn norm(a: [f64; 3]) -> [f64; 3] {
    let l = len(a).max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// What a player's window sends about their jet.
pub fn message(host: &mut Host<'_>, player: u128, data: &[u8]) {
    let Some(up) = Up::decode(data) else {
        return;
    };
    let players = host.server_players();
    let me = players.iter().find(|p| p.uuid == player);
    // a fast jet is well ahead of where the server last had its pilot
    let near = |at: [f64; 3], dimension: &str| me.is_some_and(|p| p.dimension == dimension && len(sub(p.position, at)) < 160.0);
    let tick = server::now();
    match up {
        Up::Leave => with(|s| {
            s.pilots.remove(&player);
        }),
        Up::Fly { dimension, eye, vel: _, fwd } => {
            if !near(eye, &dimension) {
                return;
            }
            // the pilot's body caught on something the jet flew past: back in the seat
            if let Some(me) = me.filter(|p| len(sub([p.position[0], p.position[1] + 1.62, p.position[2]], eye)) > 64.0) {
                let line = format!("execute in {dimension} run tp {} {:.2} {:.2} {:.2}", me.name, eye[0], eye[1] - 1.62, eye[2]);
                host.run_command(&line);
            }
            with(|s| {
                let p = s.pilots.entry(player).or_insert(Pilot { dimension: dimension.clone(), eye, fwd, seen: tick, lock: None, locked_for: 0 });
                p.dimension = dimension;
                p.eye = eye;
                p.fwd = fwd;
                p.seen = tick;
            });
        }
        Up::Gun { dimension, from, dir, count } => {
            if near(from, &dimension) {
                with(|s| s.guns.push(Gun { dimension, from, dir, count }));
            }
        }
        Up::Missile { dimension, from, vel, dir } => {
            if !near(from, &dimension) {
                return;
            }
            with(|s| {
                let target = s.pilots.get(&player).filter(|p| p.locked_for >= 2).and_then(|p| p.lock);
                let id = s.next;
                s.next = s.next.wrapping_add(1);
                // dropped clear of the jet, then the motor lights
                let launch = add(vel, dir, 15.0);
                let launch = add(launch, [0.0, -6.0, 0.0], 1.0);
                if s.ordnance.len() < 64 {
                    s.ordnance.push(Ordnance { id, dimension, pos: from, vel: launch, missile: true, big: false, target, age: 0 });
                }
            });
        }
        Up::Bomb { dimension, from, vel, big } => {
            if !near(from, &dimension) {
                return;
            }
            with(|s| {
                let id = s.next;
                s.next = s.next.wrapping_add(1);
                if s.ordnance.len() < 64 {
                    s.ordnance.push(Ordnance { id, dimension, pos: from, vel, missile: false, big, target: None, age: 0 });
                }
            });
        }
    }
}

fn send(host: &mut Host<'_>, player: u128, down: &Down) {
    let _ = host.net_send_to_player(player, jet_net::CHANNEL, &down.encode());
}

/// Whether a block stops a round or a missile: anything but air, fluids and small plants.
fn solid(block: &str) -> bool {
    !(block.is_empty()
        || block.ends_with("air")
        || block.contains("water")
        || block.contains("lava")
        || block.ends_with("grass") && !block.ends_with("grass_block")
        || block.contains("fern")
        || block.contains("flower")
        || block.contains("sapling")
        || block.contains("torch")
        || block.ends_with("vine")
        || block.ends_with("vines")
        || block == "minecraft:snow")
}

/// Whether the cannon chips a block.
fn soft(block: &str) -> bool {
    block.contains("leaves")
        || block.contains("glass")
        || block.ends_with("dirt")
        || block == "minecraft:grass_block"
        || block.ends_with("sand")
        || block == "minecraft:gravel"
        || block.contains("wool")
        || block.contains("planks")
        || block.contains("ice")
        || block.contains("melon")
        || block.contains("pumpkin")
}

/// Things a missile may lock on to: creatures, not items, arrows or falling blocks.
fn lockable(kind: &str) -> bool {
    !(kind.contains("item")
        || kind.contains("falling_block")
        || kind.contains("experience")
        || kind.contains("arrow")
        || kind.contains("trident")
        || kind.contains("display")
        || kind.contains("marker")
        || kind.contains("interaction")
        || kind.contains("painting")
        || kind.contains("frame")
        || kind.contains("armor_stand")
        || kind.contains("lightning")
        || kind.contains("tnt")
        || kind.contains("snowball")
        || kind.contains("potion")
        || kind.contains("firework")
        || kind.contains("fireball")
        || kind.contains("boat")
        || kind.contains("minecart"))
}

/// Each tick of level `dimension`.
pub fn level_tick(host: &mut Host<'_>, dimension: &str) {
    let tick = server::now();
    let (pilots, guns, any) = with(|s| {
        s.pilots.retain(|_, p| tick.saturating_sub(p.seen) < 60);
        let pilots: Vec<(u128, [f64; 3], [f64; 3])> = s.pilots.iter().filter(|(_, p)| p.dimension == dimension).map(|(id, p)| (*id, p.eye, p.fwd)).collect();
        let (mine, rest): (Vec<Gun>, Vec<Gun>) = std::mem::take(&mut s.guns).into_iter().partition(|g| g.dimension == dimension);
        s.guns = rest;
        let any = s.ordnance.iter().any(|o| o.dimension == dimension);
        (pilots, mine, any)
    });
    if pilots.is_empty() && guns.is_empty() && !any {
        return;
    }
    // the mobs about, once
    let mobs: Vec<(u32, [f64; 3])> = server::entities_in(dimension)
        .into_iter()
        .filter(|(_, kind)| lockable(kind))
        .filter_map(|(id, _)| host.entity(id).and_then(|e| host.entity_position(e)).map(|p| (id, p)))
        .collect();
    if !pilots.is_empty() {
        let players = host.server_players();
        for (id, eye, fwd) in &pilots {
            if let Some(p) = players.iter().find(|p| p.uuid == *id) {
                send(host, *id, &Down::You(p.position));
            }
            if tick % 4 == 0 {
                // the lock: the mob nearest the nose, within 14 degrees and 400 blocks
                let mut best: Option<(u32, [f64; 3], f64)> = None;
                for (mob, at) in &mobs {
                    let d = sub(*at, *eye);
                    let dist = len(d);
                    if !(6.0..400.0).contains(&dist) {
                        continue;
                    }
                    let angle = (dot(d, *fwd) / dist).clamp(-1.0, 1.0).acos();
                    if angle < 0.25 && best.is_none_or(|b| angle < b.2) {
                        best = Some((*mob, *at, angle));
                    }
                }
                let state = with(|s| {
                    let p = s.pilots.get_mut(id)?;
                    match best {
                        Some((mob, _, _)) if p.lock == Some(mob) => p.locked_for += 1,
                        Some((mob, _, _)) => {
                            p.lock = Some(mob);
                            p.locked_for = 0;
                        }
                        None => {
                            p.lock = None;
                            p.locked_for = 0;
                        }
                    }
                    Some(if p.lock.is_none() { 0 } else if p.locked_for >= 2 { 2 } else { 1 })
                });
                let at = best.map_or([0.0; 3], |b| b.1);
                send(host, *id, &Down::Lock { state: state.unwrap_or(0), at });
            }
        }
    }
    for gun in guns {
        for _ in 0..gun.count {
            let spread = with(|s| [rand(s) - 0.5, rand(s) - 0.5, rand(s) - 0.5]);
            let dir = norm(add(gun.dir, spread, 0.012));
            shoot(host, dimension, gun.from, dir, &mobs);
        }
    }
    step_ordnance(host, dimension, &mobs, &pilots);
}

/// One cannon round: the first solid block along its line, or the first mob near it.
fn shoot(host: &mut Host<'_>, dimension: &str, from: [f64; 3], dir: [f64; 3], mobs: &[(u32, [f64; 3])]) {
    let mut s = 1.0;
    let mut hit = None;
    while s < 380.0 {
        let p = add(from, dir, s);
        let block = host.block([p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]);
        if solid(&block) {
            hit = Some((s, block));
            break;
        }
        s += 0.7;
    }
    let range = hit.as_ref().map_or(380.0, |h| h.0);
    let struck = mobs.iter().find(|(_, at)| {
        let d = sub(*at, from);
        let along = dot(d, dir);
        along > 0.0 && along < range && len(sub(d, [dir[0] * along, dir[1] * along, dir[2] * along])) < 1.3
    });
    if let Some((_, at)) = struck {
        server::command(host, dimension, *at, "damage @e[type=!minecraft:player,distance=..2,limit=1,sort=nearest] 5 minecraft:arrow");
        return;
    }
    if let Some((s, block)) = hit {
        let p = add(from, dir, s);
        let cell = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
        if soft(&block) && with(|st| rand(st)) < 0.3 {
            let mut columns = HashMap::new();
            columns.insert((cell[0], cell[2]), (cell[1], cell[1]));
            server::carve(host, dimension, &columns);
        }
        if with(|st| rand(st)) < 0.25 {
            server::command(host, dimension, p, "particle minecraft:crit ~ ~ ~ 0.1 0.1 0.1 0.3 3");
        }
    }
}

fn step_ordnance(host: &mut Host<'_>, dimension: &str, mobs: &[(u32, [f64; 3])], pilots: &[(u128, [f64; 3], [f64; 3])]) {
    let dt = 0.05;
    let mut mine: Vec<Ordnance> = with(|s| {
        let (here, rest): (Vec<Ordnance>, Vec<Ordnance>) = std::mem::take(&mut s.ordnance).into_iter().partition(|o| o.dimension == dimension);
        s.ordnance = rest;
        here
    });
    let (min_y, _) = host.world_height();
    let mut booms = Vec::new();
    mine.retain_mut(|o| {
        o.age += 1;
        if o.missile {
            let speed = len(o.vel);
            let mut dir = norm(o.vel);
            if o.age > 4 {
                if let Some(target) = o.target.and_then(|t| mobs.iter().find(|m| m.0 == t)) {
                    // turn towards the target, a little at a time
                    let want = norm(sub(add(target.1, [0.0, 0.8, 0.0], 1.0), o.pos));
                    let k = 0.22;
                    dir = norm(add(dir, sub(want, dir), k));
                    if len(sub(target.1, o.pos)) < 3.5 {
                        booms.push((o.pos, 3.4));
                        return false;
                    }
                }
                let speed = (speed + 14.0).min(190.0);
                o.vel = [dir[0] * speed, dir[1] * speed, dir[2] * speed];
            } else {
                o.vel[1] -= 9.81 * dt;
            }
        } else {
            o.vel[1] -= 9.81 * dt;
            let drag = 1.0 - 0.004 * dt * len(o.vel).min(200.0) / 10.0;
            o.vel = [o.vel[0] * drag, o.vel[1] * drag, o.vel[2] * drag];
        }
        let step = add(o.pos, o.vel, dt);
        let travel = len(sub(step, o.pos));
        let samples = (travel / 0.8).ceil().max(1.0) as usize;
        for k in 1..=samples {
            let p = add(o.pos, sub(step, o.pos), k as f64 / samples as f64);
            let block = host.block([p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]);
            let mob = mobs.iter().any(|m| len(sub(m.1, p)) < 1.6);
            if solid(&block) || mob {
                let radius = if o.missile { 3.4 } else if o.big { 6.5 } else { 4.6 };
                booms.push((p, radius));
                return false;
            }
        }
        o.pos = step;
        let life = if o.missile { 170 } else { 500 };
        if o.age > life {
            if o.missile {
                booms.push((o.pos, 2.0));
            }
            return false;
        }
        o.pos[1] > f64::from(min_y)
    });
    let shots: Vec<Shot> = mine.iter().map(|o| Shot { id: o.id, missile: o.missile, pos: o.pos, vel: o.vel }).collect();
    with(|s| s.ordnance.extend(mine));
    for (at, radius) in &booms {
        explode(host, dimension, *at, *radius);
    }
    for (id, _, _) in pilots {
        send(host, *id, &Down::Shots(shots.clone()));
        for (at, radius) in &booms {
            send(host, *id, &Down::Boom { at: *at, radius: *radius });
        }
    }
}

/// A missile's or bomb's blast: a crater, and everything near hurt and thrown.
fn explode(host: &mut Host<'_>, dimension: &str, at: [f64; 3], radius: f64) {
    let mut columns = HashMap::new();
    server::carve_sphere(&mut columns, at, radius);
    server::carve(host, dimension, &columns);
    let damage = 14.0 * radius;
    server::command(host, dimension, at, &format!("damage @e[type=!minecraft:player,distance=..{:.1}] {damage:.0} minecraft:explosion", radius * 2.2));
    server::fling(host, dimension, at, radius * 3.0, |o, d| {
        let k = (radius * 0.35 / d.max(1.0)).min(1.4);
        [o[0] * k, 0.6 + k * 0.4, o[2] * k]
    });
    server::command(host, dimension, at, &format!("playsound minecraft:entity.generic.explode master @a[distance=..{:.0}] ~ ~ ~ {:.1} 0.7", radius * 40.0, radius * 1.4));
    server::command(host, dimension, at, "particle minecraft:explosion_emitter ~ ~ ~ 0 0 0 0 1 force");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_stops_a_round() {
        assert!(solid("minecraft:stone"));
        assert!(solid("minecraft:grass_block"));
        assert!(solid("minecraft:oak_leaves"));
        assert!(!solid("minecraft:short_grass"));
        assert!(!solid("minecraft:air"));
        assert!(!solid("minecraft:water"));
        assert!(lockable("minecraft:zombie") && !lockable("minecraft:item") && !lockable("minecraft:falling_block"));
    }
}
