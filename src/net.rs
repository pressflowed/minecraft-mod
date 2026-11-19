//! What a client tells the server when it uses a power: one short line of text on the mod's
//! channel, `power;dimension;x;y;z;dx;dy;dz;level;value`.

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
    TitanFist,
    Omega,
    Wrath,
    AbsoluteZero,
    Singularity,
    Cataclysm,
    /// The Storm Scepter: `level` is the event, `value` how far away the caller stood.
    Weather,
    /// The Gravity Gun: `at` is the cube's lowest corner, `level` its size.
    Grab,
    /// The Gravity Gun flings what it holds along `dir`.
    Fling,
    /// The Gravity Gun lets go of what it holds.
    Drop,
    /// `/superpowers give`: the two items.
    Give,
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
            Act::TitanFist => "titan_fist",
            Act::Omega => "omega",
            Act::Wrath => "wrath",
            Act::AbsoluteZero => "absolute_zero",
            Act::Singularity => "singularity",
            Act::Cataclysm => "cataclysm",
            Act::Weather => "weather",
            Act::Grab => "grab",
            Act::Fling => "fling",
            Act::Drop => "drop",
            Act::Give => "give",
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
            "titan_fist" => Act::TitanFist,
            "omega" => Act::Omega,
            "wrath" => Act::Wrath,
            "absolute_zero" => Act::AbsoluteZero,
            "singularity" => Act::Singularity,
            "cataclysm" => Act::Cataclysm,
            "weather" => Act::Weather,
            "grab" => Act::Grab,
            "fling" => Act::Fling,
            "drop" => Act::Drop,
            "give" => Act::Give,
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
    pub value: f64,
}

impl Use {
    pub fn encode(&self) -> Vec<u8> {
        format!(
            "{};{};{:.3};{:.3};{:.3};{:.4};{:.4};{:.4};{};{:.3}",
            self.act.name(),
            self.dimension,
            self.at[0],
            self.at[1],
            self.at[2],
            self.dir[0],
            self.dir[1],
            self.dir[2],
            self.level,
            self.value
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
        let value: f64 = parts.next()?.parse().ok()?;
        if !value.is_finite() || value.abs() >= 3.0e7 {
            return None;
        }
        if parts.next().is_some() {
            return None;
        }
        Some(Use {
            act,
            dimension: dimension.to_owned(),
            at,
            dir,
            level: match act {
                Act::Wrath => level,
                Act::Weather => level.min(crate::weather::EVENTS.len() as u8 - 1),
                Act::Grab => level.clamp(1, 8),
                _ => level.clamp(1, 5),
            },
            value,
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
            value: 0.0,
        };
        assert_eq!(Use::decode(&sent.encode()), Some(sent));
        let storm = Use {
            act: Act::Weather,
            dimension: "minecraft:overworld".into(),
            at: [-400.5, 70.0, 812.25],
            dir: [0.6, 0.0, 0.8],
            level: 12,
            value: 431.5,
        };
        assert_eq!(Use::decode(&storm.encode()), Some(storm));
    }

    #[test]
    fn odd_bytes_are_refused() {
        assert_eq!(Use::decode(b"punch;minecraft:overworld;1;2;3;0;0;0;1;0"), None);
        assert_eq!(Use::decode(b"nuke;minecraft:overworld;1;2;3;0;0;1;1;0"), None);
        assert_eq!(Use::decode(b"punch;Bad Dim;1;2;3;0;0;1;1;0"), None);
        assert_eq!(Use::decode(b"punch;minecraft:overworld;NaN;2;3;0;0;1;1;0"), None);
        assert_eq!(Use::decode(b"punch;minecraft:overworld;1;2;3;0;0;1;1;0;extra"), None);
        assert_eq!(Use::decode(&[0xff, 0xfe]), None);
    }
}
