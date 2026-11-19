//! Vector maths for Superflight and the powers.
//!
//! Conventions are vanilla's: x east, y up, z south; a yaw of 0 looks south (+z), 90 west; a
//! positive pitch looks down. Roll is positive with the right wing down.

pub type V3 = [f64; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn normalize(a: V3) -> V3 {
    let l = length(a);
    if l < 1e-12 { [0.0, 0.0, 1.0] } else { scale(a, 1.0 / l) }
}

/// Rotates `v` by `angle` radians about the unit `axis` (right-handed).
pub fn rotate(v: V3, axis: V3, angle: f64) -> V3 {
    let (s, c) = angle.sin_cos();
    add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)))
}

/// Where vanilla's yaw and pitch (degrees) look.
pub fn look(yaw: f64, pitch: f64) -> V3 {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_a_quarter_turn() {
        let v = rotate([0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], std::f64::consts::FRAC_PI_2);
        assert!((v[1] - 1.0).abs() < 1e-9 && v[2].abs() < 1e-9, "{v:?}");
        let l = look(0.0, 0.0);
        assert!((l[2] - 1.0).abs() < 1e-9);
    }
}
