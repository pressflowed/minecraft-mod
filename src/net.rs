//! The one message the client sends the server: a power was used, where and which way.

use mod_api::PayloadCodec;

/// A power used: `power` is a [`crate::powers::Power`] code; `at` where it acts (the eye, a target or
/// a point), `dir` which way it faces (a unit vector), `level` its strength (the laser eyes' power;
/// for Superflight, 1 for taking off and 0 for landing).
#[derive(Clone, Debug, PartialEq, PayloadCodec)]
pub struct Cast {
    pub power: u8,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
    pub level: u8,
}

pub const CAST: &str = "superpowers:cast";
pub const VERSION: &str = "1";

impl Cast {
    pub fn new(power: crate::powers::Power, at: [f64; 3], dir: [f64; 3], level: u8) -> Self {
        Self { power: power as u8, x: at[0], y: at[1], z: at[2], dx: dir[0], dy: dir[1], dz: dir[2], level }
    }

    pub fn at(&self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    /// The direction, made unit length; None for one that isn't finite or is nearly zero.
    pub fn dir(&self) -> Option<[f64; 3]> {
        let d = [self.dx, self.dy, self.dz];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        (l.is_finite() && l > 0.5 && l < 1.5).then(|| [d[0] / l, d[1] / l, d[2] / l])
    }

    /// Checks the sender's bytes: finite and within reach of where they are.
    pub fn sane(&self, player_at: [f64; 3], reach: f64) -> bool {
        let at = self.at();
        at.iter().all(|v| v.is_finite())
            && self.dir().is_some()
            && crate::flight::length(crate::flight::sub(at, player_at)) <= reach
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{decode_payload, encode_payload};

    #[test]
    fn a_cast_survives_the_trip() {
        let cast = Cast::new(crate::powers::Power::Singularity, [1.5, 64.0, -3.25], [0.0, 0.0, 1.0], 3);
        let body = encode_payload(&cast, 1024).unwrap();
        assert_eq!(decode_payload::<Cast>(&body), Ok(cast.clone()));
        assert!(cast.sane([0.0, 64.0, 0.0], 50.0));
        assert!(!cast.sane([500.0, 64.0, 0.0], 50.0));
        let bad = Cast { dx: f64::NAN, ..cast };
        assert!(!bad.sane([0.0, 64.0, 0.0], 50.0));
    }
}
