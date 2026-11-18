//! What a client tells the server when it uses a power: one short line of text on the mod's
//! channel, `power;dimension;x;y;z;dx;dy;dz;level`.

/// The mod's message channel.
pub const CHANNEL: &str = "superpowers:power";

/// Which power a message is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Punch,
    Laser,
    Thunder,
    Frost,
    Gravity,
    Dash,
}

impl Act {
    fn name(self) -> &'static str {
        match self {
            Act::Punch => "punch",
            Act::Laser => "laser",
            Act::Thunder => "thunder",
            Act::Frost => "frost",
            Act::Gravity => "gravity",
            Act::Dash => "dash",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "punch" => Act::Punch,
            "laser" => Act::Laser,
            "thunder" => Act::Thunder,
            "frost" => Act::Frost,
            "gravity" => Act::Gravity,
            "dash" => Act::Dash,
            _ => return None,
        })
    }
}

/// One use of a power: where (`at`), which way (`dir`, a unit vector) and how strong.
#[derive(Clone, Debug, PartialEq)]
pub struct Use {
    pub act: Act,
    pub dimension: String,
    pub at: [f64; 3],
    pub dir: [f64; 3],
    pub level: u8,
}

impl Use {
    pub fn encode(&self) -> Vec<u8> {
        format!(
            "{};{};{:.3};{:.3};{:.3};{:.4};{:.4};{:.4};{}",
            self.act.name(),
            self.dimension,
            self.at[0],
            self.at[1],
            self.at[2],
            self.dir[0],
            self.dir[1],
            self.dir[2],
            self.level
        )
        .into_bytes()
    }

    /// The player's own bytes, checked by nothing: anything odd is refused.
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() > 256 {
            return None;
        }
        let text = std::str::from_utf8(data).ok()?;
        let mut parts = text.split(';');
        let act = Act::parse(parts.next()?)?;
        let dimension = parts.next()?;
        if dimension.is_empty()
            || dimension.len() > 64
            || !dimension
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_:/.-".contains(c))
        {
            return None;
        }
        let mut number = || -> Option<f64> {
            let v: f64 = parts.next()?.parse().ok()?;
            (v.is_finite() && v.abs() < 3.0e7).then_some(v)
        };
        let at = [number()?, number()?, number()?];
        let dir = [number()?, number()?, number()?];
        let length = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if !(0.5..=1.5).contains(&length) {
            return None;
        }
        let dir = [dir[0] / length, dir[1] / length, dir[2] / length];
        let level: u8 = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Use {
            act,
            dimension: dimension.to_owned(),
            at,
            dir,
            level: level.clamp(1, 5),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_use_survives_the_trip() {
        let sent = Use {
            act: Act::Gravity,
            dimension: "minecraft:overworld".into(),
            at: [12.5, 64.0, -3.25],
            dir: [0.0, 0.0, 1.0],
            level: 3,
        };
        assert_eq!(Use::decode(&sent.encode()), Some(sent));
    }

    #[test]
    fn odd_bytes_are_refused() {
        assert_eq!(Use::decode(b"punch;minecraft:overworld;1;2;3;0;0;0;1"), None);
        assert_eq!(Use::decode(b"nuke;minecraft:overworld;1;2;3;0;0;1;1"), None);
        assert_eq!(Use::decode(b"punch;Bad Dim;1;2;3;0;0;1;1"), None);
        assert_eq!(Use::decode(b"punch;minecraft:overworld;NaN;2;3;0;0;1;1"), None);
        assert_eq!(Use::decode(b"punch;minecraft:overworld;1;2;3;0;0;1;1;extra"), None);
        assert_eq!(Use::decode(&[0xff, 0xfe]), None);
    }
}
