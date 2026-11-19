//! What the window and the server say to each other about jets, one line of text each, on the
//! channel [`CHANNEL`].
//!
//! Up (window to server): where the jet is each tick, the cannon firing, a missile or a bomb
//! let go. Down (server to window): where the server has the player (the jet pulls them along
//! and checks they followed), the missile lock, every missile and bomb in flight, explosions.

pub const CHANNEL: &str = "superpowers:jet";

#[derive(Clone, Debug, PartialEq)]
pub enum Up {
    /// The jet this tick: the pilot's eyes, its velocity (blocks a second) and nose.
    Fly { dimension: String, eye: [f64; 3], vel: [f64; 3], fwd: [f64; 3] },
    /// `count` rounds from `from` along `dir`.
    Gun { dimension: String, from: [f64; 3], dir: [f64; 3], count: u8 },
    /// A missile let go at `from`, moving at `vel`, pointing `dir`.
    Missile { dimension: String, from: [f64; 3], vel: [f64; 3], dir: [f64; 3] },
    /// A bomb let go; `big` for the bomber's.
    Bomb { dimension: String, from: [f64; 3], vel: [f64; 3], big: bool },
    /// Out of the jet.
    Leave,
}

/// A missile or bomb in flight, as the server has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub id: u32,
    pub missile: bool,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Down {
    /// Where the player's feet are.
    You([f64; 3]),
    /// The lock: 0 none, 1 locking, 2 locked, and on what.
    Lock { state: u8, at: [f64; 3] },
    Shots(Vec<Shot>),
    /// Something went off here, this big.
    Boom { at: [f64; 3], radius: f64 },
}

fn nums(v: &[f64]) -> String {
    v.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(";")
}

fn parse_nums(parts: &mut std::str::Split<'_, char>, n: usize) -> Option<Vec<f64>> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let v: f64 = parts.next()?.parse().ok()?;
        if !v.is_finite() || v.abs() > 3.0e7 {
            return None;
        }
        out.push(v);
    }
    Some(out)
}

fn dim_ok(d: &str) -> bool {
    !d.is_empty() && d.len() <= 64 && d.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_:/.-".contains(c))
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (0.5..=1.5).contains(&l).then(|| [v[0] / l, v[1] / l, v[2] / l])
}

fn three(v: &[f64], at: usize) -> [f64; 3] {
    [v[at], v[at + 1], v[at + 2]]
}

impl Up {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Up::Fly { dimension, eye, vel, fwd } => format!("fly;{dimension};{}", nums(&[eye[0], eye[1], eye[2], vel[0], vel[1], vel[2], fwd[0], fwd[1], fwd[2]])),
            Up::Gun { dimension, from, dir, count } => format!("gun;{dimension};{};{count}", nums(&[from[0], from[1], from[2], dir[0], dir[1], dir[2]])),
            Up::Missile { dimension, from, vel, dir } => format!("missile;{dimension};{}", nums(&[from[0], from[1], from[2], vel[0], vel[1], vel[2], dir[0], dir[1], dir[2]])),
            Up::Bomb { dimension, from, vel, big } => format!("bomb;{dimension};{};{}", nums(&[from[0], from[1], from[2], vel[0], vel[1], vel[2]]), u8::from(*big)),
            Up::Leave => "leave".to_owned(),
        }
        .into_bytes()
    }

    /// A player's own bytes: anything odd is refused.
    pub fn decode(data: &[u8]) -> Option<Up> {
        if data.len() > 512 {
            return None;
        }
        let text = std::str::from_utf8(data).ok()?;
        let mut parts = text.split(';');
        let kind = parts.next()?;
        if kind == "leave" {
            return parts.next().is_none().then_some(Up::Leave);
        }
        let dimension = parts.next()?.to_owned();
        if !dim_ok(&dimension) {
            return None;
        }
        let up = match kind {
            "fly" => {
                let v = parse_nums(&mut parts, 9)?;
                if three(&v, 3).iter().any(|x| x.abs() > 400.0) {
                    return None;
                }
                Up::Fly { dimension, eye: three(&v, 0), vel: three(&v, 3), fwd: unit(three(&v, 6))? }
            }
            "gun" => {
                let v = parse_nums(&mut parts, 6)?;
                let count: u8 = parts.next()?.parse().ok()?;
                Up::Gun { dimension, from: three(&v, 0), dir: unit(three(&v, 3))?, count: count.min(4) }
            }
            "missile" => {
                let v = parse_nums(&mut parts, 9)?;
                if three(&v, 3).iter().any(|x| x.abs() > 400.0) {
                    return None;
                }
                Up::Missile { dimension, from: three(&v, 0), vel: three(&v, 3), dir: unit(three(&v, 6))? }
            }
            "bomb" => {
                let v = parse_nums(&mut parts, 6)?;
                let big: u8 = parts.next()?.parse().ok()?;
                if three(&v, 3).iter().any(|x| x.abs() > 400.0) {
                    return None;
                }
                Up::Bomb { dimension, from: three(&v, 0), vel: three(&v, 3), big: big == 1 }
            }
            _ => return None,
        };
        parts.next().is_none().then_some(up)
    }
}

impl Down {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Down::You(p) => format!("you;{}", nums(p)),
            Down::Lock { state, at } => format!("lock;{state};{}", nums(at)),
            Down::Shots(shots) => {
                let mut s = String::from("shots");
                for shot in shots.iter().take(40) {
                    s.push_str(&format!(
                        ";{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2}",
                        shot.id,
                        u8::from(shot.missile),
                        shot.pos[0],
                        shot.pos[1],
                        shot.pos[2],
                        shot.vel[0],
                        shot.vel[1],
                        shot.vel[2]
                    ));
                }
                s
            }
            Down::Boom { at, radius } => format!("boom;{};{radius:.2}", nums(at)),
        }
        .into_bytes()
    }

    pub fn decode(data: &[u8]) -> Option<Down> {
        let text = std::str::from_utf8(data).ok()?;
        let mut parts = text.split(';');
        let kind = parts.next()?;
        Some(match kind {
            "you" => Down::You(three(&parse_nums(&mut parts, 3)?, 0)),
            "lock" => {
                let state: u8 = parts.next()?.parse().ok()?;
                Down::Lock { state, at: three(&parse_nums(&mut parts, 3)?, 0) }
            }
            "shots" => {
                let mut shots = Vec::new();
                for item in parts {
                    let f: Vec<&str> = item.split(',').collect();
                    if f.len() != 8 {
                        return None;
                    }
                    let n = |i: usize| f[i].parse::<f64>().ok().filter(|v| v.is_finite());
                    shots.push(Shot {
                        id: f[0].parse().ok()?,
                        missile: f[1] == "1",
                        pos: [n(2)?, n(3)?, n(4)?],
                        vel: [n(5)?, n(6)?, n(7)?],
                    });
                }
                return Some(Down::Shots(shots));
            }
            "boom" => {
                let v = parse_nums(&mut parts, 4)?;
                Down::Boom { at: three(&v, 0), radius: v[3] }
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_the_trip() {
        let ups = [
            Up::Fly { dimension: "minecraft:overworld".into(), eye: [1.5, 80.0, -3.25], vel: [10.0, -2.0, 90.0], fwd: [0.0, 0.0, 1.0] },
            Up::Gun { dimension: "minecraft:overworld".into(), from: [1.0, 2.0, 3.0], dir: [0.6, 0.0, 0.8], count: 2 },
            Up::Missile { dimension: "minecraft:overworld".into(), from: [1.0, 2.0, 3.0], vel: [0.0, 0.0, 80.0], dir: [0.0, 0.0, 1.0] },
            Up::Bomb { dimension: "minecraft:overworld".into(), from: [1.0, 2.0, 3.0], vel: [0.0, 0.0, 80.0], big: true },
            Up::Leave,
        ];
        for u in ups {
            assert_eq!(Up::decode(&u.encode()), Some(u));
        }
        let shots = Down::Shots(vec![Shot { id: 4, missile: true, pos: [1.0, 2.0, 3.0], vel: [4.0, 5.0, 6.0] }]);
        assert_eq!(Down::decode(&shots.encode()), Some(shots));
        let lock = Down::Lock { state: 2, at: [1.0, 2.0, 3.0] };
        assert_eq!(Down::decode(&lock.encode()), Some(lock));
        assert_eq!(Up::decode(b"fly;Bad;1;2;3;0;0;0;0;0;1"), None);
        assert_eq!(Up::decode(b"gun;minecraft:overworld;1;2;3;0;0;0;1"), None);
    }
}
