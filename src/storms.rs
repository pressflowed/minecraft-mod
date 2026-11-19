//! The Storm Scepter's events in the world: each one a [`Storm`] that the server steps every
//! tick in its level, changing the land as it goes. Heights come from the level's own
//! heightmaps, so a flood fills the low ground, a wave climbs the shore and a volcano rises on
//! whatever the land was.

use crate::server::{self, BREAKABLE};
use crate::weather::{self, Dice, EVENTS};
use crate::host::{Host, NativeBlockEdit, NativeBlockRun, NativeHeightmap};
use std::collections::{HashMap, HashSet};
use std::f64::consts::TAU;

/// Plants, leaves and what grows: what even a weak wind takes.
const PLANTS: [&str; 7] = [
    "#minecraft:leaves",
    "#minecraft:mineable/hoe",
    "#minecraft:wool",
    "minecraft:short_grass",
    "minecraft:tall_grass",
    "minecraft:fern",
    "minecraft:large_fern",
];

pub struct Storm {
    pub dimension: String,
    pub event: usize,
    at: [f64; 3],
    /// For a tsunami: the way the wave rolls (towards the caller), and how far.
    dir: [f64; 3],
    run: f64,
    seed: u32,
    age: u64,
    ticks: u64,
    dice: Dice,
    /// Columns already struck, for the events that strike each column once.
    struck: HashSet<(i32, i32)>,
    cored: HashSet<(i32, i32)>,
    /// Columns to finish next step (water after a wave has cleared them).
    pending: Vec<(i32, i32)>,
    /// How far along: a wave's front, each fault's length.
    progress: Vec<f64>,
    /// The volcano's crater height, once known.
    top: Option<f64>,
}

impl Storm {
    pub fn new(dimension: &str, event: usize, at: [f64; 3], toward: [f64; 3], distance: f64) -> Self {
        let seed = weather::seed_of(at, event);
        let flat = (toward[0] * toward[0] + toward[2] * toward[2]).sqrt();
        let dir = if flat > 1e-3 { [toward[0] / flat, 0.0, toward[2] / flat] } else { [1.0, 0.0, 0.0] };
        Self {
            dimension: dimension.to_owned(),
            event,
            at,
            dir,
            run: weather::wave_run(distance),
            seed,
            age: 0,
            ticks: (EVENTS[event].seconds * 20.0) as u64,
            dice: Dice::new(seed ^ 0x5EED),
            struck: HashSet::new(),
            cored: HashSet::new(),
            pending: Vec::new(),
            progress: Vec::new(),
            top: None,
        }
    }

    pub fn done(&self) -> bool {
        self.age > self.ticks
    }

    fn seconds(&self) -> f64 {
        self.age as f64 / 20.0
    }

    /// 0 to 1 to 0: easing in over `rise` seconds and out over the last `fall`.
    fn envelope(&self, rise: f64, fall: f64) -> f64 {
        let t = self.seconds();
        let total = self.ticks as f64 / 20.0;
        smooth((t / rise).min(1.0)) * smooth(((total - t) / fall).clamp(0.0, 1.0))
    }

    fn rand(&mut self) -> f64 {
        self.dice.next()
    }
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// One tick of `storm` in its level.
pub fn step(host: &mut Host<'_>, storm: &mut Storm) {
    let dimension = storm.dimension.clone();
    let dim = dimension.as_str();
    match storm.event {
        e if weather::ef(e).is_some() => tornado(host, dim, storm, weather::ef(e).unwrap_or(1)),
        weather::HURRICANE => hurricane(host, dim, storm),
        weather::TSUNAMI => tsunami(host, dim, storm),
        weather::FLOOD => flood(host, dim, storm),
        weather::SOLAR_FLARE => flare(host, dim, storm),
        weather::NUKE => nuke(host, dim, storm),
        weather::METEOR => meteor(host, dim, storm),
        weather::EARTHQUAKE => earthquake(host, dim, storm),
        weather::VOLCANO => volcano(host, dim, storm),
        _ => {}
    }
    storm.age += 1;
}

// ---- edits ------------------------------------------------------------------------------------

/// A bulk edit in the making: a palette, filters, and runs naming them.
struct Edit {
    palette: Vec<String>,
    filters: Vec<String>,
    runs: Vec<NativeBlockRun>,
    floor: i32,
    top: i32,
}

impl Edit {
    fn new(host: &Host<'_>, palette: &[&str], filters: &[&str]) -> Self {
        let (min_y, height) = host.world_height();
        Self {
            palette: palette.iter().map(|s| (*s).to_owned()).collect(),
            filters: filters.iter().map(|s| (*s).to_owned()).collect(),
            runs: Vec::new(),
            floor: min_y + 5,
            top: min_y + height - 1,
        }
    }

    /// `height` blocks from `y` up (absolute), set to `block` where filter `only` matches.
    fn run(&mut self, x: i32, z: i32, y: i32, height: i32, block: u32, only: Option<u32>) {
        let lo = y.max(self.floor);
        let hi = (y + height - 1).min(self.top);
        if hi < lo {
            return;
        }
        self.runs.push(NativeBlockRun { x, z, y: lo, height: (hi - lo + 1) as u32, surface: None, block, only });
    }

    /// The same run once for each filter in `filters`.
    fn run_each(&mut self, x: i32, z: i32, y: i32, height: i32, block: u32, filters: std::ops::Range<u32>) {
        for f in filters {
            self.run(x, z, y, height, block, Some(f));
        }
    }

    fn apply(self, host: &mut Host<'_>, dimension: &str) {
        if self.runs.is_empty() {
            return;
        }
        let edit = NativeBlockEdit { palette: self.palette, filters: self.filters, runs: self.runs };
        let _ = host.edit_blocks(dimension, &edit);
    }
}

/// Each column's height on `map` (the first free block over it), None where not loaded.
fn heights(host: &Host<'_>, dimension: &str, columns: &[(i32, i32)], map: NativeHeightmap) -> Vec<Option<i32>> {
    if columns.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(columns.len());
    for chunk in columns.chunks(60_000) {
        match host.surface_heights(dimension, chunk, map) {
            Ok(h) => out.extend(h),
            Err(_) => out.extend(std::iter::repeat_n(None, chunk.len())),
        }
    }
    out
}

/// The columns of a disc of radius `r` round `(cx, cz)`, each with its distance.
fn disc(cx: f64, cz: f64, r: f64) -> Vec<(i32, i32, f64)> {
    let mut out = Vec::new();
    let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
    let (z0, z1) = ((cz - r).floor() as i32, (cz + r).ceil() as i32);
    for x in x0..=x1 {
        for z in z0..=z1 {
            let d = (f64::from(x) + 0.5 - cx).hypot(f64::from(z) + 0.5 - cz);
            if d <= r {
                out.push((x, z, d));
            }
        }
    }
    out
}

/// A cheap hash of a column, 0 to 1, for scattering fire and choosing stone.
fn speckle(x: i32, z: i32, salt: u32) -> f64 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343) ^ (z as u32).wrapping_mul(0xD816_3841) ^ salt.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    f64::from(h >> 8) / f64::from(1u32 << 24)
}

/// Throws the top block of column `(x, z)` (read on the level's top solid surface) at `velocity`.
fn toss(host: &mut Host<'_>, dimension: &str, x: i32, z: i32, below: i32, velocity: [f64; 3]) {
    let h = heights(host, dimension, &[(x, z)], NativeHeightmap::OceanFloor);
    let Some(Some(top)) = h.first().copied() else {
        return;
    };
    let y = top - 1 - below;
    let block = host.block([x, y, z]);
    if !server::throwable(&block) {
        return;
    }
    let mut columns = HashMap::new();
    columns.insert((x, z), (y, y));
    server::carve(host, dimension, &columns);
    server::throw(host, dimension, &block, [f64::from(x) + 0.5, f64::from(y) + 0.6, f64::from(z) + 0.5], velocity);
}

fn lightning(dimension: &str, at: [f64; 3]) {
    server::queue_lightning(dimension, at, 1.0);
}

// ---- tornadoes ----------------------------------------------------------------------------------

fn tornado(host: &mut Host<'_>, dim: &str, s: &mut Storm, ef: u8) {
    if s.age % 2 != 0 {
        return;
    }
    let env = s.envelope(2.0, 3.0);
    let r = weather::tornado_radius(ef) * env;
    if r < 1.0 {
        return;
    }
    let off = weather::tornado_offset(s.seed, ef, s.seconds());
    let (cx, cz) = (s.at[0] + off[0], s.at[2] + off[1]);
    let center = [cx, s.at[1], cz];
    let core = r * 0.55;
    // what the wind takes, by strength: plants and leaves; then trees, roofs and glass; then the soil
    let mut filters: Vec<&str> = PLANTS.to_vec();
    filters.push("#minecraft:replaceable");
    if ef >= 2 {
        filters.push("#minecraft:mineable/axe");
        filters.push("#minecraft:impermeable");
    }
    let plant_count = filters.len() as u32;
    filters.push("#minecraft:mineable/shovel");
    filters.push("#minecraft:mineable/pickaxe");
    let (scour, rock) = match ef {
        1 | 2 => (0, 0),
        3 => (1, 0),
        4 => (3, 0),
        _ => (4, 2),
    };
    let mut fresh = Vec::new();
    for (x, z, d) in disc(cx, cz, r) {
        let inner = d <= core;
        let new_outer = s.struck.insert((x, z));
        let new_inner = inner && s.cored.insert((x, z));
        if new_outer || new_inner {
            fresh.push((x, z, new_outer, new_inner, d));
        }
    }
    let cols: Vec<(i32, i32)> = fresh.iter().map(|f| (f.0, f.1)).collect();
    let ground = heights(host, dim, &cols, NativeHeightmap::MotionBlockingNoLeaves);
    let mut edit = Edit::new(host, &["minecraft:air"], &filters);
    for ((x, z, new_outer, new_inner, d), h) in fresh.into_iter().zip(ground) {
        let Some(h) = h else { continue };
        if new_outer {
            // from under the trunks' tops to over the crowns
            let lift = 8 + 2 * i32::from(ef);
            edit.run_each(x, z, h - lift, lift + 34, 0, 0..plant_count);
        }
        if new_inner && scour > 0 {
            let fall = 1.0 - (d / core).powi(2);
            let deep = (f64::from(scour) * (0.5 + 0.5 * fall)).round() as i32;
            edit.run(x, z, h - deep, deep, 0, Some(plant_count));
            if rock > 0 && d < core * 0.6 {
                edit.run(x, z, h - deep - rock, rock + deep, 0, Some(plant_count + 1));
            }
        }
    }
    edit.apply(host, dim);
    // debris torn up and flung round the funnel
    for _ in 0..(1 + usize::from(ef)) {
        let a = s.rand() * TAU;
        let d = s.rand().sqrt() * r * 1.2;
        let (x, z) = ((cx + a.cos() * d).floor() as i32, (cz + a.sin() * d).floor() as i32);
        let swirl = [-a.sin(), 0.0, a.cos()];
        let spin = 0.5 + 0.15 * f64::from(ef);
        let lift = 0.7 + 0.22 * f64::from(ef) + s.rand() * 0.4;
        let velocity = [swirl[0] * spin - a.cos() * 0.15, lift, swirl[2] * spin - a.sin() * 0.15];
        toss(host, dim, x, z, 0, velocity);
    }
    // everything near is drawn in, spun round and lifted
    let reach = r * 3.2;
    let k_ef = 0.12 + 0.07 * f64::from(ef);
    server::fling(host, dim, center, reach + 40.0, |o, _| {
        let d = o[0].hypot(o[2]);
        if d > reach || d < 0.2 {
            return [0.0; 3];
        }
        let inward = [-o[0] / d, 0.0, -o[2] / d];
        let swirl = [-inward[2], 0.0, inward[0]];
        let k = k_ef * (1.0 - d / reach).max(0.0).sqrt();
        let lift = if d < r * 1.3 { 0.18 + 0.09 * f64::from(ef) } else { 0.04 };
        [inward[0] * k * 0.6 + swirl[0] * k, lift, inward[2] * k * 0.6 + swirl[2] * k]
    });
    if s.age % 10 == 0 {
        server::command(host, dim, center, &format!("damage @e[type=!minecraft:player,distance=..{:.0}] {} minecraft:fly_into_wall", r * 1.2, 2 * ef));
    }
}

// ---- hurricane ----------------------------------------------------------------------------------

const HURRICANE_RADIUS: f64 = 140.0;
const EYE: f64 = 16.0;

fn hurricane(host: &mut Host<'_>, dim: &str, s: &mut Storm) {
    if s.age % 2 != 0 {
        return;
    }
    let env = s.envelope(4.0, 5.0);
    let (cx, cz) = (s.at[0], s.at[2]);
    // patches of plants and leaves stripped, most near the eye wall
    let mut patches = Vec::new();
    for _ in 0..((6.0 * env) as usize) {
        let d = EYE + (HURRICANE_RADIUS - EYE) * s.rand().powf(1.7);
        let a = s.rand() * TAU;
        patches.push((cx + a.cos() * d, cz + a.sin() * d, 3.0, false));
    }
    if s.age % 6 == 0 {
        // the eye wall's gusts snap trees
        let d = EYE * (1.0 + s.rand() * 2.0);
        let a = s.rand() * TAU;
        patches.push((cx + a.cos() * d, cz + a.sin() * d, 2.5, true));
    }
    let mut cols = Vec::new();
    for &(px, pz, pr, snap) in &patches {
        for (x, z, _) in disc(px, pz, pr) {
            cols.push((x, z, snap));
        }
    }
    let only: Vec<(i32, i32)> = cols.iter().map(|c| (c.0, c.1)).collect();
    let ground = heights(host, dim, &only, NativeHeightmap::MotionBlockingNoLeaves);
    let mut filters: Vec<&str> = PLANTS.to_vec();
    filters.push("#minecraft:mineable/axe");
    let mut edit = Edit::new(host, &["minecraft:air"], &filters);
    let plants = PLANTS.len() as u32;
    for ((x, z, snap), h) in cols.into_iter().zip(ground) {
        let Some(h) = h else { continue };
        edit.run_each(x, z, h - 8, 40, 0, 0..plants);
        if snap {
            edit.run(x, z, h - 8, 40, 0, Some(plants));
        }
    }
    edit.apply(host, dim);
    // debris on the wind
    for _ in 0..2 {
        let d = EYE * (1.0 + s.rand() * 3.0);
        let a = s.rand() * TAU;
        let swirl = [-a.sin(), 0.0, a.cos()];
        let (x, z) = ((cx + a.cos() * d).floor() as i32, (cz + a.sin() * d).floor() as i32);
        toss(host, dim, x, z, 0, [swirl[0] * 1.3, 0.8 + s.rand() * 0.6, swirl[2] * 1.3]);
    }
    // the storm surge: water left in the low ground, a little at a time
    if s.age % 10 == 0 {
        let d = EYE + s.rand() * (HURRICANE_RADIUS - EYE);
        let a = s.rand() * TAU;
        let spots = disc(cx + a.cos() * d, cz + a.sin() * d, 2.5);
        let only: Vec<(i32, i32)> = spots.iter().map(|c| (c.0, c.1)).collect();
        let tops = heights(host, dim, &only, NativeHeightmap::MotionBlocking);
        let mut water = Edit::new(host, &["minecraft:water"], &["minecraft:air"]);
        for ((x, z), h) in only.into_iter().zip(tops) {
            if let Some(h) = h {
                water.run(x, z, h, 1, 0, Some(0));
            }
        }
        water.apply(host, dim);
    }
    if s.age % 14 == 0 && env > 0.3 {
        let d = s.rand() * HURRICANE_RADIUS;
        let a = s.rand() * TAU;
        let (x, z) = (cx + a.cos() * d, cz + a.sin() * d);
        let top = heights(host, dim, &[(x.floor() as i32, z.floor() as i32)], NativeHeightmap::MotionBlocking);
        if let Some(Some(h)) = top.first().copied() {
            lightning(dim, [x, f64::from(h), z]);
        }
    }
    let center = s.at;
    server::fling(host, dim, center, HURRICANE_RADIUS + 20.0, |o, _| {
        let d = o[0].hypot(o[2]);
        if d < 1.0 || d > HURRICANE_RADIUS {
            return [0.0; 3];
        }
        let inward = [-o[0] / d, 0.0, -o[2] / d];
        let swirl = [-inward[2], 0.0, inward[0]];
        let wall = ((d - EYE * 1.6) / (HURRICANE_RADIUS * 0.35)).powi(2);
        let k = (0.28 * (-wall).exp() + 0.05) * env;
        if d < EYE {
            return [0.0; 3];
        }
        [swirl[0] * k + inward[0] * k * 0.25, 0.03, swirl[2] * k + inward[2] * k * 0.25]
    });
}

// ---- tsunami ------------------------------------------------------------------------------------

fn tsunami(host: &mut Host<'_>, dim: &str, s: &mut Storm) {
    if s.age % 2 != 0 {
        return;
    }
    if s.progress.is_empty() {
        s.progress.push(0.0);
    }
    let dir = s.dir;
    let side = [-dir[2], 0.0, dir[0]];
    let half = weather::WAVE_WIDTH / 2.0;
    // the water the wave leaves behind, where it cleared last step
    let pending = std::mem::take(&mut s.pending);
    if !pending.is_empty() {
        let tops = heights(host, dim, &pending, NativeHeightmap::MotionBlocking);
        let mut water = Edit::new(host, &["minecraft:water"], &["minecraft:air"]);
        for ((x, z), h) in pending.into_iter().zip(tops) {
            if let Some(h) = h {
                water.run(x, z, h, 1, 0, Some(0));
            }
        }
        water.apply(host, dim);
    }
    let before = s.progress[0];
    let front = (weather::WAVE_SPEED * s.seconds()).min(s.run);
    s.progress[0] = front;
    if front > before {
        let mut cells = HashMap::new();
        let mut a = before;
        while a <= front {
            let mut b = -half;
            while b <= half {
                let x = s.at[0] + dir[0] * a + side[0] * b;
                let z = s.at[2] + dir[2] * a + side[2] * b;
                let edge = smooth((half - b.abs()) / 30.0);
                cells.insert((x.floor() as i32, z.floor() as i32), edge);
                b += 0.7;
            }
            a += 0.7;
        }
        let cols: Vec<(i32, i32)> = cells.keys().copied().collect();
        let ground = heights(host, dim, &cols, NativeHeightmap::MotionBlockingNoLeaves);
        let filters = [
            "#minecraft:leaves",
            "#minecraft:replaceable",
            "#minecraft:mineable/axe",
            "#minecraft:mineable/hoe",
            "#minecraft:wool",
            "#minecraft:impermeable",
            "minecraft:sand",
            "minecraft:gravel",
        ];
        let mut edit = Edit::new(host, &["minecraft:air"], &filters);
        for ((x, z), h) in cols.iter().copied().zip(ground) {
            let Some(h) = h else { continue };
            let edge = cells[&(x, z)];
            let height = (weather::WAVE_HEIGHT * edge) as i32;
            if height < 2 {
                continue;
            }
            edit.run_each(x, z, h - 2, height + 4, 0, 0..6);
            // the shore's sand is dragged away close to the start
            if speckle(x, z, 3) < 0.4 {
                edit.run_each(x, z, h - 1, 1, 0, 6..8);
            }
            if (x + z) % 2 == 0 && edge > 0.3 {
                s.pending.push((x, z));
            }
        }
        edit.apply(host, dim);
    }
    // what stands at the front is swept along
    let origin = s.at;
    server::fling(host, dim, origin, s.run + 40.0, |o, _| {
        let along = o[0] * dir[0] + o[2] * dir[2];
        let across = o[0] * side[0] + o[2] * side[2];
        if !(front - 9.0..=front + 3.0).contains(&along) || across.abs() > half {
            return [0.0; 3];
        }
        [dir[0] * 1.4, 0.55, dir[2] * 1.4]
    });
    if s.age % 6 == 0 {
        for k in -3..=3 {
            let b = f64::from(k) * half / 3.5;
            let at = [s.at[0] + dir[0] * front + side[0] * b, s.at[1] + 2.0, s.at[2] + dir[2] * front + side[2] * b];
            server::command(host, dim, at, "damage @e[type=!minecraft:player,distance=..12] 5 minecraft:drown");
        }
    }
    // the break, just short of the caller: a last surge of water spreads over the land
    if front >= s.run && s.progress.len() == 1 {
        s.progress.push(1.0);
        let center = [s.at[0] + dir[0] * front, s.at[2] + dir[2] * front];
        for (x, z, _) in disc(center[0], center[1], 22.0) {
            if (x + z) % 2 == 0 {
                s.pending.push((x, z));
            }
        }
        let at = [center[0], s.at[1], center[1]];
        server::fling(host, dim, at, 40.0, |o, d| {
            let k = (1.8 / d.max(1.0)).min(1.2);
            [o[0] * k + dir[0], 0.9, o[2] * k + dir[2]]
        });
    }
}

// ---- flood --------------------------------------------------------------------------------------

const FLOOD_RADIUS: f64 = 48.0;
const FLOOD_LEVELS: u64 = 9;

fn flood(host: &mut Host<'_>, dim: &str, s: &mut Storm) {
    if s.age < 30 || (s.age - 30) % 40 != 0 {
        return;
    }
    let level = (s.age - 30) / 40;
    if level >= FLOOD_LEVELS {
        return;
    }
    let y = s.at[1].floor() as i32 + level as i32;
    let mut edit = Edit::new(
        host,
        &["minecraft:water"],
        &["minecraft:air", "minecraft:short_grass", "minecraft:tall_grass", "minecraft:fern", "minecraft:snow"],
    );
    let (cx, cz) = (s.at[0], s.at[2]);
    for (x, z, d) in disc(cx, cz, FLOOD_RADIUS) {
        // a ragged shore
        let a = (f64::from(z) - cz).atan2(f64::from(x) - cx);
        let shore = FLOOD_RADIUS * (0.82 + 0.1 * (a * 3.0 + f64::from(s.seed % 7)).sin() + 0.08 * (a * 7.0).sin());
        if d > shore {
            continue;
        }
        edit.run_each(x, z, y, 1, 0, 0..5);
    }
    edit.apply(host, dim);
    server::command(host, dim, s.at, &format!("playsound minecraft:block.water.ambient master @a[distance=..96] ~ ~{level} ~ 2 0.6"));
}

// ---- solar flare --------------------------------------------------------------------------------

const FLARE_RADIUS: f64 = 38.0;
const FLARE_CORE: f64 = 9.0;

fn flare(host: &mut Host<'_>, dim: &str, s: &mut Storm) {
    let t = s.seconds();
    if !(0.8..6.5).contains(&t) || s.age % 4 != 0 {
        return;
    }
    let r = FLARE_RADIUS * ((t - 0.8) / 4.0).min(1.0);
    let (cx, cz) = (s.at[0], s.at[2]);
    let fresh: Vec<(i32, i32, f64)> = disc(cx, cz, r).into_iter().filter(|c| s.struck.insert((c.0, c.1))).collect();
    let cols: Vec<(i32, i32)> = fresh.iter().map(|c| (c.0, c.1)).collect();
    let ground = heights(host, dim, &cols, NativeHeightmap::MotionBlockingNoLeaves);
    let palette = [
        "minecraft:air",
        "minecraft:glass",
        "minecraft:coarse_dirt",
        "minecraft:magma_block",
        "minecraft:fire",
        "minecraft:orange_stained_glass",
        "minecraft:obsidian",
    ];
    let filters = [
        "#minecraft:leaves",
        "minecraft:snow",
        "minecraft:snow_block",
        "minecraft:ice",
        "minecraft:packed_ice",
        "minecraft:powder_snow",
        "minecraft:water",
        "minecraft:sand",
        "minecraft:red_sand",
        "minecraft:grass_block",
        "minecraft:dirt",
        "minecraft:air",
        "#minecraft:mineable/pickaxe",
        "#minecraft:mineable/shovel",
        "minecraft:lava",
        "minecraft:short_grass",
        "minecraft:tall_grass",
    ];
    let mut edit = Edit::new(host, &palette, &filters);
    for ((x, z, d), h) in fresh.into_iter().zip(ground) {
        let Some(h) = h else { continue };
        // leaves burn, snow and ice melt, shallow water boils away
        edit.run_each(x, z, h - 10, 44, 0, 0..6);
        edit.run(x, z, h - 3, 4, 0, Some(6));
        edit.run(x, z, h, 2, 0, Some(15));
        edit.run(x, z, h, 2, 0, Some(16));
        // the ground scorched: sand to glass, grass to bare earth
        edit.run(x, z, h - 1, 1, 1, Some(7));
        edit.run(x, z, h - 1, 1, 5, Some(8));
        edit.run(x, z, h - 1, 1, 2, Some(9));
        edit.run(x, z, h - 1, 1, 2, Some(10));
        if d < FLARE_CORE {
            // where the beam touches, the rock melts
            edit.run(x, z, h - 2, 2, 3, Some(12));
            edit.run(x, z, h - 2, 2, 3, Some(13));
            edit.run(x, z, h - 3, 3, 6, Some(14));
        }
        if speckle(x, z, s.seed) < 0.16 {
            edit.run(x, z, h, 1, 4, Some(11));
        }
    }
    edit.apply(host, dim);
    if s.age % 10 == 0 {
        let at = s.at;
        server::command(host, dim, at, &format!("damage @e[type=!minecraft:player,distance=..{r:.0}] 6 minecraft:in_fire"));
        server::command(host, dim, at, &format!("execute as @e[type=!minecraft:player,distance=..{r:.0},limit=60] run data merge entity @s {{Fire:200s}}"));
    }
}

// ---- nuke ---------------------------------------------------------------------------------------

const NUKE_CRATER: f64 = 44.0;
const NUKE_REACH: f64 = 150.0;

fn nuke(host: &mut Host<'_>, dim: &str, s: &mut Storm) {
    let at = s.at;
    if s.age == 0 {
        server::queue_crater(dim, at, NUKE_CRATER, 28.0, 70.0, 6.0);
        server::command(host, dim, at, &format!("damage @e[type=!minecraft:player,distance=..{:.0}] 500 minecraft:explosion", NUKE_CRATER + 10.0));
        server::fling(host, dim, at, NUKE_REACH + 10.0, |o, d| {
            let k = (5.0 / d.max(1.0)).min(3.0);
            [o[0] * k, 1.6, o[2] * k]
        });
    }
    if s.age == 25 {
        server::command(host, dim, at, &format!("damage @e[type=!minecraft:player,distance=..{:.0}] 60 minecraft:explosion", NUKE_REACH - 20.0));
        server::command(host, dim, at, &format!("execute as @e[type=!minecraft:player,distance=..{NUKE_REACH:.0},limit=120] run data merge entity @s {{Fire:300s}}"));
    }
    // the scorched land beyond the crater, a band of 8 blocks a tick
    if (8..8 + 14).contains(&s.age) {
        let inner = NUKE_CRATER - 2.0 + (s.age - 8) as f64 * 8.0;
        let outer = (inner + 8.0).min(NUKE_REACH);
        if inner >= outer {
            return;
        }
        let band: Vec<(i32, i32, f64)> = disc(at[0], at[2], outer).into_iter().filter(|c| c.2 >= inner).collect();
        let cols: Vec<(i32, i32)> = band.iter().map(|c| (c.0, c.1)).collect();
        let ground = heights(host, dim, &cols, NativeHeightmap::MotionBlockingNoLeaves);
        let palette = ["minecraft:air", "minecraft:coarse_dirt", "minecraft:fire", "minecraft:glass"];
        let filters = [
            "#minecraft:leaves",
            "minecraft:short_grass",
            "minecraft:tall_grass",
            "#minecraft:mineable/hoe",
            "#minecraft:mineable/axe",
            "#minecraft:impermeable",
            "#minecraft:wool",
            "minecraft:grass_block",
            "minecraft:dirt",
            "minecraft:air",
            "minecraft:sand",
        ];
        let mut edit = Edit::new(host, &palette, &filters);
        for ((x, z, d), h) in band.into_iter().zip(ground) {
            let Some(h) = h else { continue };
            if d < 120.0 || speckle(x, z, 9) < 0.5 {
                edit.run_each(x, z, h - 12, 44, 0, 0..4);
            }
            if d < 80.0 {
                // the blast flattens what is built and grown
                edit.run_each(x, z, h - 12, 44, 0, 4..7);
            }
            if d < 110.0 {
                edit.run(x, z, h - 1, 1, 1, Some(7));
                edit.run(x, z, h - 1, 1, 1, Some(8));
            }
            if d < 70.0 {
                edit.run(x, z, h - 1, 1, 3, Some(10));
            }
            if d < 120.0 && speckle(x, z, 77) < 0.12 {
                edit.run(x, z, h, 1, 2, Some(9));
            }
        }
        edit.apply(host, dim);
    }
}

// ---- meteor -------------------------------------------------------------------------------------

const METEOR_CRATER: f64 = 17.0;

fn meteor(host: &mut Host<'_>, dim: &str, s: &mut Storm) {
    let impact = (weather::METEOR_SECONDS * 20.0) as u64;
    let at = s.at;
    if s.age == impact {
        // ejecta: blocks of the crater thrown out over the land round it
        for _ in 0..40 {
            let a = s.rand() * TAU;
            let d = s.rand().sqrt() * METEOR_CRATER * 0.8;
            let (x, z) = ((at[0] + a.cos() * d).floor() as i32, (at[2] + a.sin() * d).floor() as i32);
            let out = 0.6 + s.rand() * 0.9;
            let below = (s.rand() * 4.0) as i32;
            toss(host, dim, x, z, below, [a.cos() * out, 0.9 + s.rand() * 0.9, a.sin() * out]);
        }
        server::queue_crater(dim, at, METEOR_CRATER, 13.0, 40.0, 4.0);
        server::command(host, dim, at, "damage @e[type=!minecraft:player,distance=..22] 120 minecraft:explosion");
        server::fling(host, dim, at, 46.0, |o, d| {
            let k = (3.0 / d.max(1.0)).min(2.0);
            [o[0] * k, 1.2, o[2] * k]
        });
    }
    if s.age == impact + 8 {
        // the meteorite itself, still glowing, in the bottom of its crater
        let core = disc(at[0], at[2], 3.2);
        let cols: Vec<(i32, i32)> = core.iter().map(|c| (c.0, c.1)).collect();
        let ground = heights(host, dim, &cols, NativeHeightmap::MotionBlockingNoLeaves);
        let palette = ["minecraft:ancient_debris", "minecraft:magma_block", "minecraft:crying_obsidian", "minecraft:obsidian"];
        let mut edit = Edit::new(host, &palette, &[]);
        for ((x, z, d), h) in core.into_iter().zip(ground) {
            let Some(h) = h else { continue };
            if d < 1.3 {
                edit.run(x, z, h, 3, 0, None);
            } else if d < 2.3 {
                let block = if speckle(x, z, 4) < 0.5 { 1 } else { 2 };
                edit.run(x, z, h, 2, block, None);
            } else {
                edit.run(x, z, h, 1, 3, None);
            }
        }
        edit.apply(host, dim);
        // fires round the rim
        let rim: Vec<(i32, i32, f64)> = disc(at[0], at[2], METEOR_CRATER + 6.0).into_iter().filter(|c| c.2 > METEOR_CRATER - 1.0).collect();
        let cols: Vec<(i32, i32)> = rim.iter().map(|c| (c.0, c.1)).collect();
        let tops = heights(host, dim, &cols, NativeHeightmap::MotionBlocking);
        let mut fire = Edit::new(host, &["minecraft:fire"], &["minecraft:air"]);
        for ((x, z, _), h) in rim.into_iter().zip(tops) {
            if let Some(h) = h
                && speckle(x, z, 5) < 0.18
            {
                fire.run(x, z, h, 1, 0, Some(0));
            }
        }
        fire.apply(host, dim);
    }
}

// ---- earthquake ---------------------------------------------------------------------------------

const QUAKE_REACH: f64 = 90.0;

fn earthquake(host: &mut Host<'_>, dim: &str, s: &mut Storm) {
    if s.age % 2 != 0 {
        return;
    }
    let faults = weather::faults(s.seed);
    if s.progress.is_empty() {
        s.progress = vec![0.0; faults.len()];
    }
    let t = s.seconds();
    let mut cells: HashMap<(i32, i32), (i32, bool)> = HashMap::new();
    for (i, &(a, length)) in faults.iter().enumerate() {
        let before = s.progress[i];
        let now = length * (t / 7.0).min(1.0);
        s.progress[i] = now;
        let mut along = before;
        while along < now {
            let p = weather::fault_point(s.seed, i, a, along);
            let open = 1.0 - along / length;
            let width = 0.7 + 1.8 * open;
            let depth = (7.0 + 19.0 * open) as i32;
            let lava = along < length * 0.45;
            for (x, z, _) in disc(s.at[0] + p[0], s.at[2] + p[1], width) {
                let e = cells.entry((x, z)).or_insert((0, false));
                e.0 = e.0.max(depth);
                e.1 |= lava;
            }
            along += 0.8;
        }
    }
    if !cells.is_empty() {
        let cols: Vec<(i32, i32)> = cells.keys().copied().collect();
        let ground = heights(host, dim, &cols, NativeHeightmap::MotionBlockingNoLeaves);
        let mut edit = Edit::new(host, &["minecraft:air", "minecraft:lava"], &BREAKABLE);
        for ((x, z), h) in cols.into_iter().zip(ground) {
            let Some(h) = h else { continue };
            let (depth, lava) = cells[&(x, z)];
            let bottom = h - depth;
            if lava {
                edit.run_each(x, z, bottom, 2, 1, 0..BREAKABLE.len() as u32);
            }
            let from = if lava { bottom + 2 } else { bottom };
            edit.run_each(x, z, from, h + 14 - from, 0, 0..BREAKABLE.len() as u32);
        }
        edit.apply(host, dim);
    }
    // the land shakes: loose blocks hop and land a little way off
    if s.age % 4 == 0 && t < 15.0 {
        for _ in 0..5 {
            let a = s.rand() * TAU;
            let d = s.rand().sqrt() * QUAKE_REACH * 0.8;
            let (x, z) = ((s.at[0] + a.cos() * d).floor() as i32, (s.at[2] + a.sin() * d).floor() as i32);
            let hop = [(s.rand() - 0.5) * 0.5, 0.45 + s.rand() * 0.3, (s.rand() - 0.5) * 0.5];
            toss(host, dim, x, z, 0, hop);
        }
    }
    if s.age % 10 == 0 && t < 15.0 {
        let seed = s.age;
        server::fling(host, dim, s.at, QUAKE_REACH, move |o, _| {
            let wobble = ((seed as f64) * 0.7 + o[0] * 0.1).sin() * 0.15;
            [wobble, 0.42, -wobble]
        });
        server::command(host, dim, s.at, "damage @e[type=!minecraft:player,distance=..30] 2 minecraft:fall");
    }
}

// ---- volcano ------------------------------------------------------------------------------------

const CRATER: f64 = 7.0;
const RISE_STEPS: u64 = 35;

/// The cone's height `d` blocks from its middle, a crater dipping in at the top.
fn cone(d: f64) -> f64 {
    let r = weather::VOLCANO_RADIUS;
    let h = weather::VOLCANO_HEIGHT;
    if d >= r {
        return 0.0;
    }
    let rim = h * (1.0 - CRATER / r).powf(1.3);
    if d < CRATER {
        rim - (CRATER - d) * 1.4
    } else {
        h * (1.0 - d / r).powf(1.3)
    }
}

fn volcano(host: &mut Host<'_>, dim: &str, s: &mut Storm) {
    let at = s.at;
    let r = weather::VOLCANO_RADIUS;
    let (cx, cz) = (at[0], at[2]);
    if s.age == 0 {
        // the land that rises loses its trees first
        let cols: Vec<(i32, i32, f64)> = disc(cx, cz, r);
        let only: Vec<(i32, i32)> = cols.iter().map(|c| (c.0, c.1)).collect();
        let ground = heights(host, dim, &only, NativeHeightmap::MotionBlockingNoLeaves);
        let filters = ["#minecraft:leaves", "#minecraft:mineable/axe", "#minecraft:mineable/hoe", "#minecraft:wool", "#minecraft:replaceable"];
        let mut edit = Edit::new(host, &["minecraft:air"], &filters);
        for ((x, z), h) in only.into_iter().zip(ground) {
            if let Some(h) = h {
                edit.run_each(x, z, h - 8, 40, 0, 0..5);
            }
        }
        edit.apply(host, dim);
        return;
    }
    let step_ticks = 4;
    if s.age % step_ticks == 0 && s.age / step_ticks <= RISE_STEPS {
        let j = s.age / step_ticks;
        let f = |j: u64| smooth(j as f64 / RISE_STEPS as f64);
        let (now, before) = (f(j), f(j - 1));
        let cols = disc(cx, cz, r);
        let only: Vec<(i32, i32)> = cols.iter().map(|c| (c.0, c.1)).collect();
        let tops = heights(host, dim, &only, NativeHeightmap::MotionBlocking);
        let palette = [
            "minecraft:basalt",
            "minecraft:blackstone",
            "minecraft:tuff",
            "minecraft:smooth_basalt",
            "minecraft:magma_block",
            "minecraft:stone",
        ];
        let mut edit = Edit::new(host, &palette, &[]);
        for ((x, z, d), h) in cols.into_iter().zip(tops) {
            let Some(h) = h else { continue };
            let target = cone(d);
            let n = (target * now).floor() as i32 - (target * before).floor() as i32;
            if n <= 0 {
                continue;
            }
            let pick = speckle(x, z, j as u32);
            let near_top = d < CRATER + 4.0;
            let block = if near_top && pick < 0.45 || pick < 0.06 {
                4
            } else if pick < 0.4 {
                0
            } else if pick < 0.62 {
                1
            } else if pick < 0.8 {
                2
            } else if pick < 0.92 {
                3
            } else {
                5
            };
            edit.run(x, z, h, n, block, None);
        }
        edit.apply(host, dim);
        // what stood there rides up the slope instead of being buried
        server::fling(host, dim, at, r, |o, d| {
            if d > r {
                return [0.0; 3];
            }
            [o[0] * 0.02, 0.55, o[2] * 0.02]
        });
    }
    let erupting = s.age / step_ticks > RISE_STEPS + 1;
    if s.age / step_ticks == RISE_STEPS + 1 && s.age % step_ticks == 0 {
        // lava fills the crater and spills over the rim
        let pool = disc(cx, cz, CRATER - 0.5);
        let only: Vec<(i32, i32)> = pool.iter().map(|c| (c.0, c.1)).collect();
        let tops = heights(host, dim, &only, NativeHeightmap::MotionBlocking);
        let mut edit = Edit::new(host, &["minecraft:lava"], &["minecraft:air"]);
        let mut highest = i32::MIN;
        for ((x, z), h) in only.into_iter().zip(tops) {
            if let Some(h) = h {
                highest = highest.max(h);
                edit.run(x, z, h - 1, 3, 0, None);
            }
        }
        for k in 0..4 {
            let a = f64::from(k) / 4.0 * TAU + f64::from(s.seed % 13) * 0.3;
            let (x, z) = ((cx + a.cos() * (CRATER + 1.5)).floor() as i32, (cz + a.sin() * (CRATER + 1.5)).floor() as i32);
            let rim = heights(host, dim, &[(x, z)], NativeHeightmap::MotionBlocking);
            if let Some(Some(h)) = rim.first().copied() {
                edit.run(x, z, h, 1, 0, Some(0));
            }
        }
        edit.apply(host, dim);
        if highest > i32::MIN {
            s.top = Some(f64::from(highest) + 1.0);
        }
        server::command(host, dim, at, "playsound minecraft:entity.generic.explode master @a[distance=..200] ~ ~40 ~ 6 0.5");
    }
    if erupting && s.age % 3 == 0 && s.age + 40 < s.ticks {
        let top = s.top.unwrap_or(at[1] + cone(0.0) + 2.0);
        for _ in 0..3 {
            let a = s.rand() * TAU;
            let speed = 0.35 + s.rand() * 0.95;
            let velocity = [a.cos() * speed, 1.1 + s.rand() * 1.3, a.sin() * speed];
            let pick = s.rand();
            let block = if pick < 0.6 {
                "minecraft:magma_block"
            } else if pick < 0.85 {
                "minecraft:blackstone"
            } else {
                "minecraft:obsidian"
            };
            let pos = [cx + (s.rand() - 0.5) * 4.0, top + 1.5, cz + (s.rand() - 0.5) * 4.0];
            server::throw(host, dim, block, pos, velocity);
        }
        if s.age % 27 == 0 {
            let a = s.rand() * TAU;
            let d = CRATER + s.rand() * (r - CRATER);
            let (x, z) = (cx + a.cos() * d, cz + a.sin() * d);
            let h = heights(host, dim, &[(x.floor() as i32, z.floor() as i32)], NativeHeightmap::MotionBlocking);
            if let Some(Some(h)) = h.first().copied() {
                lightning(dim, [x, f64::from(h), z]);
            }
        }
        if s.age % 10 == 0 {
            let vent = [cx, top, cz];
            server::command(host, dim, vent, "damage @e[type=!minecraft:player,distance=..12] 6 minecraft:lava");
            server::command(host, dim, vent, "particle minecraft:campfire_signal_smoke ~ ~2 ~ 3 2 3 0.08 40 force");
            server::command(host, dim, vent, "particle minecraft:lava ~ ~1 ~ 2 1 2 0 20 force");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volcano_has_a_crater() {
        assert!(cone(0.0) < cone(CRATER));
        assert!(cone(CRATER) > cone(20.0));
        assert_eq!(cone(weather::VOLCANO_RADIUS), 0.0);
        assert!(cone(CRATER) > 30.0);
    }

    #[test]
    fn a_disc_is_round() {
        let d = disc(0.5, 0.5, 3.0);
        assert!(d.iter().any(|c| (c.0, c.1) == (0, 0)));
        assert!(d.iter().all(|c| c.2 <= 3.0));
        assert!(!d.iter().any(|c| (c.0, c.1) == (3, 3)));
    }
}
