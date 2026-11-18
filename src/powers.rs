//! Every power: its key, its name on the panel, its default key and its cooldown.

/// A power's slot. The numbers are what a cast message carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Power {
    Punch = 0,
    Lasers = 1,
    Thunder = 2,
    Frost = 3,
    Gravity = 4,
    Dash = 5,
    Flight = 6,
    TitanFist = 7,
    OmegaBeam = 8,
    Wrath = 9,
    AbsoluteZero = 10,
    Singularity = 11,
    Cataclysm = 12,
}

pub struct Info {
    pub power: Power,
    /// The key mapping's id (`superpowers:<key>` in the controls).
    pub key: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    /// Seconds before it can be used again.
    pub cooldown: f32,
    /// The panel's accent colour, `0xRRGGBB`.
    pub color: u32,
    /// One of the five mega powers or the Cataclysm: drawn in their own part of the panel.
    pub mega: bool,
}

pub const ALL: [Info; 13] = [
    Info { power: Power::Punch, key: "punch", label: "Super Punch", default: "key.keyboard.z", cooldown: 1.2, color: 0xFFB347, mega: false },
    Info { power: Power::Lasers, key: "lasers", label: "Laser Eyes", default: "key.keyboard.g", cooldown: 0.25, color: 0xFF3B3B, mega: false },
    Info { power: Power::Thunder, key: "thunder", label: "Thunder Strike", default: "key.keyboard.v", cooldown: 3.0, color: 0x7FB8FF, mega: false },
    Info { power: Power::Frost, key: "frost", label: "Frost Nova", default: "key.keyboard.b", cooldown: 4.0, color: 0x9FF3FF, mega: false },
    Info { power: Power::Gravity, key: "gravity", label: "Gravity Well", default: "key.keyboard.n", cooldown: 8.0, color: 0xC17BFF, mega: false },
    Info { power: Power::Dash, key: "dash", label: "Hyper Dash", default: "key.keyboard.j", cooldown: 1.0, color: 0x7CFFB2, mega: false },
    Info { power: Power::Flight, key: "flight", label: "Superflight", default: "key.keyboard.r", cooldown: 0.3, color: 0xFFFFFF, mega: false },
    Info { power: Power::TitanFist, key: "titan_fist", label: "Titan Fist", default: "key.keyboard.y", cooldown: 8.0, color: 0xFF8A2A, mega: true },
    Info { power: Power::OmegaBeam, key: "omega_beam", label: "Omega Beam", default: "key.keyboard.u", cooldown: 10.0, color: 0xFF2A6A, mega: true },
    Info { power: Power::Wrath, key: "wrath", label: "Wrath of the Gods", default: "key.keyboard.i", cooldown: 12.0, color: 0x9AC8FF, mega: true },
    Info { power: Power::AbsoluteZero, key: "absolute_zero", label: "Absolute Zero", default: "key.keyboard.o", cooldown: 14.0, color: 0xBFF8FF, mega: true },
    Info { power: Power::Singularity, key: "singularity", label: "Singularity", default: "key.keyboard.k", cooldown: 20.0, color: 0xB060FF, mega: true },
    Info { power: Power::Cataclysm, key: "cataclysm", label: "CATACLYSM (hold)", default: "key.keyboard.m", cooldown: 30.0, color: 0xFF4020, mega: true },
];

/// Other keys: the panel, and a loop while flying.
pub const PANEL: (&str, &str, &str) = ("panel", "Powers panel", "key.keyboard.h");
pub const LOOP: (&str, &str, &str) = ("loop", "Superflight: loop", "key.keyboard.x");

pub fn info(power: Power) -> &'static Info {
    &ALL[power as usize]
}

pub fn from_code(code: u8) -> Option<Power> {
    ALL.iter().map(|i| i.power).find(|p| *p as u8 == code)
}

/// How long the Cataclysm's key must be held, in ticks.
pub const CATACLYSM_CHARGE: u32 = 40;
/// The Cataclysm's crater: 20 chunks each way, 40 chunks across.
pub const CATACLYSM_RADIUS: f64 = 320.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_match_and_keys_differ() {
        for (i, info) in ALL.iter().enumerate() {
            assert_eq!(info.power as usize, i);
            assert_eq!(from_code(i as u8), Some(info.power));
        }
        let mut keys: Vec<&str> = ALL.iter().map(|i| i.default).collect();
        keys.push(PANEL.2);
        keys.push(LOOP.2);
        let count = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), count, "two powers share a default key");
        assert!(CATACLYSM_RADIUS * 2.0 / 16.0 >= 38.0, "at least 38 chunks across");
    }
}

/// Where the bolts of a lightning power fall, the same on the client (which draws them) and the
/// server (which strikes them): offsets from the target and the tick each falls, from one seed.
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
