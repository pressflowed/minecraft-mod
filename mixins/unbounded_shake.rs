fn unbounded_shake(shake: NativeShake, scale: f32) -> NativeShake {
    let scale = if scale.is_finite() { scale.clamp(0.0, 1.0) } else { 1.0 };
    let shift = |v: f32| if v.is_finite() { (v * scale).clamp(-SHAKE_MAX_OFFSET, SHAKE_MAX_OFFSET) } else { 0.0 };
    let free = |v: f32| if v.is_finite() { v.rem_euclid(360.0) } else { 0.0 };
    NativeShake {
        pitch: free(shake.pitch),
        yaw: if shake.yaw.is_finite() { (shake.yaw * scale).clamp(-SHAKE_MAX_DEGREES, SHAKE_MAX_DEGREES) } else { 0.0 },
        roll: free(shake.roll),
        offset: shake.offset.map(shift),
    }
}
