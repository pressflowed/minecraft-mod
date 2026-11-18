fn unbounded_shake(shake: Shake, scale: f32) -> Shake {
    let scale = if scale.is_finite() { scale.clamp(0.0, 1.0) } else { 1.0 };
    let turn = |v: f32| if v.is_finite() { (v * scale).clamp(-SHAKE_MAX_DEGREES, SHAKE_MAX_DEGREES) } else { 0.0 };
    let shift = |v: f32| if v.is_finite() { (v * scale).clamp(-SHAKE_MAX_OFFSET, SHAKE_MAX_OFFSET) } else { 0.0 };
    // Roll turns freely, and isn't damped by Distortion Effects: it is where the flyer's
    // wings are, not a shake.
    Shake {
        pitch: turn(shake.pitch),
        yaw: turn(shake.yaw),
        roll: if shake.roll.is_finite() { shake.roll.rem_euclid(360.0) } else { 0.0 },
        offset: shake.offset.map(shift),
    }
}
