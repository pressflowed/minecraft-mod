//! The Storm Scepter's events, and the paths and shapes both halves of the game work out the
//! same way from one seed: where a tornado has got to, which way an earthquake's faults run,
//! where a meteor comes in from.

/// One event the Storm Scepter can call down.
pub struct Event {
    pub label: &'static str,
    /// One line about it, for the picker.
    pub about: &'static str,
    pub color: u32,
    /// Seconds before the scepter can call it again.
    pub cooldown: f32,
    /// Seconds it lasts.
    pub seconds: f32,
}

pub const EF1: usize = 0;
pub const EF5: usize = 4;
pub const HURRICANE: usize = 5;
pub const TSUNAMI: usize = 6;
pub const FLOOD: usize = 7;
pub const SOLAR_FLARE: usize = 8;
pub const NUKE: usize = 9;
pub const METEOR: usize = 10;
pub const EARTHQUAKE: usize = 11;
pub const VOLCANO: usize = 12;

pub const EVENTS: [Event; 13] = [
    Event { label: "EF1 Tornado", about: "strips leaves and plants", color: 0xB8C4CC, cooldown: 4.0, seconds: 20.0 },
    Event { label: "EF2 Tornado", about: "snaps trees, tears roofs", color: 0xA8B4BE, cooldown: 5.0, seconds: 24.0 },
    Event { label: "EF3 Tornado", about: "scours the soil", color: 0x98A2AC, cooldown: 6.0, seconds: 28.0 },
    Event { label: "EF4 Tornado", about: "levels everything wooden", color: 0x8A929A, cooldown: 7.0, seconds: 34.0 },
    Event { label: "EF5 Tornado", about: "grinds the land to stone", color: 0xFF5A4A, cooldown: 8.0, seconds: 42.0 },
    Event { label: "Hurricane", about: "a 280-block storm with an eye", color: 0x6FA8FF, cooldown: 15.0, seconds: 45.0 },
    Event { label: "Tsunami", about: "a 28-block wave rolls at you", color: 0x3FD0FF, cooldown: 12.0, seconds: 16.0 },
    Event { label: "Flood", about: "the water rises 9 blocks", color: 0x4F8CFF, cooldown: 10.0, seconds: 22.0 },
    Event { label: "Solar Flare", about: "the sun reaches down and burns", color: 0xFFC23D, cooldown: 10.0, seconds: 9.0 },
    Event { label: "Nuke", about: "fireball, crater, mushroom cloud", color: 0xFF7A1A, cooldown: 20.0, seconds: 45.0 },
    Event { label: "Meteor", about: "a crater and a meteorite", color: 0xFF4A2A, cooldown: 8.0, seconds: 22.0 },
    Event { label: "Earthquake", about: "the ground splits open", color: 0xC89A6A, cooldown: 14.0, seconds: 16.0 },
    Event { label: "Volcano", about: "a mountain rises and erupts", color: 0xFF3A10, cooldown: 25.0, seconds: 40.0 },
];

/// A tornado's strength, 1 to 5, or None for another event.
pub fn ef(event: usize) -> Option<u8> {
    (EF1..=EF5).contains(&event).then(|| event as u8 + 1)
}

/// The funnel's radius at the ground, in blocks.
pub fn tornado_radius(ef: u8) -> f64 {
    [4.0, 6.0, 9.0, 13.0, 19.0][usize::from(ef.clamp(1, 5) - 1)]
}

/// How fast it crosses the land, in blocks a second.
fn tornado_speed(ef: u8) -> f64 {
    3.0 + 0.5 * f64::from(ef)
}

/// The same seed on both sides, from where the event was called (the client sends the spot).
pub fn seed_of(at: [f64; 3], event: usize) -> u32 {
    let x = at[0].floor() as i64 as u64;
    let z = at[2].floor() as i64 as u64;
    let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ z.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (event as u64) << 32;
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h as u32) | 1
}

/// A little random number source both sides run alike.
pub struct Dice(u32);

impl Dice {
    pub fn new(seed: u32) -> Self {
        Self(seed | 1)
    }

    /// 0 to 1.
    pub fn next(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        f64::from(x >> 8) / f64::from(1u32 << 24)
    }
}

/// Where a tornado is `seconds` after it touched down, from where it touched down: it heads off
/// one way and wanders as it goes.
pub fn tornado_offset(seed: u32, ef: u8, seconds: f64) -> [f64; 2] {
    let mut dice = Dice::new(seed);
    let heading = dice.next() * std::f64::consts::TAU;
    let (p1, p2) = (dice.next() * 6.0, dice.next() * 6.0);
    let speed = tornado_speed(ef);
    let step: f64 = 0.25;
    let (mut x, mut z, mut t) = (0.0, 0.0, 0.0);
    let end = seconds.max(0.0);
    while t < end {
        let dt = step.min(end - t);
        let a = heading + 0.9 * (t * 0.11 + p1).sin() + 0.5 * (t * 0.27 + p2).sin();
        x += a.cos() * speed * dt;
        z += a.sin() * speed * dt;
        t += dt;
    }
    [x, z]
}

/// An earthquake's faults: from the epicentre, each one's heading and length in blocks.
pub fn faults(seed: u32) -> Vec<(f64, f64)> {
    let mut dice = Dice::new(seed ^ 0x51ED);
    let count = 5 + (dice.next() * 3.0) as usize;
    let start = dice.next() * std::f64::consts::TAU;
    (0..count)
        .map(|i| {
            let a = start + i as f64 / count as f64 * std::f64::consts::TAU + (dice.next() - 0.5) * 0.6;
            (a, 45.0 + dice.next() * 45.0)
        })
        .collect()
}

/// A point `s` blocks along the fault heading `a`: the crack zigzags.
pub fn fault_point(seed: u32, index: usize, a: f64, s: f64) -> [f64; 2] {
    let wiggle = (s * 0.21 + index as f64 * 1.7 + f64::from(seed % 97)).sin() * 3.0
        + (s * 0.53 + index as f64 * 0.3).sin() * 1.2;
    let (c, n) = (a.cos(), a.sin());
    [c * s - n * wiggle, n * s + c * wiggle]
}

/// Where a meteor comes in from: a unit vector from the impact back up its path.
pub fn meteor_from(seed: u32) -> [f64; 3] {
    let mut dice = Dice::new(seed ^ 0xA11CE);
    let a = dice.next() * std::f64::consts::TAU;
    let up = 0.55 + dice.next() * 0.3;
    let flat = (1.0 - up * up).sqrt();
    [a.cos() * flat, up, a.sin() * flat]
}

/// How far a meteor falls, and how long it takes.
pub const METEOR_PATH: f64 = 700.0;
pub const METEOR_SECONDS: f64 = 2.6;

/// How long a volcano's cone takes to rise, and how tall and wide it is.
pub const VOLCANO_RISE: f64 = 7.0;
pub const VOLCANO_HEIGHT: f64 = 42.0;
pub const VOLCANO_RADIUS: f64 = 40.0;

/// The tsunami's wave: height, width, and speed (blocks a second).
pub const WAVE_HEIGHT: f64 = 28.0;
pub const WAVE_WIDTH: f64 = 150.0;
pub const WAVE_SPEED: f64 = 14.0;

/// How far a tsunami called `distance` blocks away rolls: it breaks just short of the caller.
pub fn wave_run(distance: f64) -> f64 {
    (distance - 14.0).clamp(30.0, 320.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tornado_goes_the_same_way_on_both_sides() {
        let seed = seed_of([120.4, 70.0, -33.2], 3);
        assert_eq!(seed, seed_of([120.9, 64.0, -33.9], 3));
        let a = tornado_offset(seed, 4, 12.5);
        let b = tornado_offset(seed, 4, 12.5);
        assert_eq!(a, b);
        // about its speed: it goes somewhere, and not too far
        let d = a[0].hypot(a[1]);
        assert!(d > 10.0 && d <= 12.5 * 5.0 + 0.01, "{d}");
    }

    #[test]
    fn every_event_has_a_name_and_a_colour() {
        assert!(EVENTS.iter().all(|e| !e.label.is_empty() && e.color != 0 && e.seconds > 0.0));
        assert_eq!(ef(EF1), Some(1));
        assert_eq!(ef(EF5), Some(5));
        assert_eq!(ef(HURRICANE), None);
        let m = meteor_from(77);
        assert!((m[0] * m[0] + m[1] * m[1] + m[2] * m[2] - 1.0).abs() < 1e-9 && m[1] > 0.5);
        assert!(faults(5).len() >= 5);
    }
}
