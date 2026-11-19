fn unbounded_shake_matrix(game: &crate::game::Game<'_>, partial: f32) -> glam::Mat4 {
    if !game.native_playing() {
        return glam::Mat4::IDENTITY;
    }
    let mut shake = NativeShake::default();
    game.native_camera_shake(partial, &mut shake);
    if shake == NativeShake::default() {
        return glam::Mat4::IDENTITY;
    }
    let mut scale = game.screen_effect_scale();
    if scale.is_finite() {
        scale = scale.clamp(0.0, 1.0);
    } else {
        scale = 1.0;
    }
    // Pitch and roll turn freely: Superflight's loops and rolls. Yaw and offset keep the bound.
    let mut free = NativeShake::default();
    if shake.pitch.is_finite() {
        free.pitch = shake.pitch.rem_euclid(360.0);
    }
    if shake.roll.is_finite() {
        free.roll = shake.roll.rem_euclid(360.0);
    }
    if shake.yaw.is_finite() {
        free.yaw = (shake.yaw * scale).clamp(-SHAKE_MAX_DEGREES, SHAKE_MAX_DEGREES);
    }
    for i in 0..3 {
        if shake.offset[i].is_finite() {
            free.offset[i] = (shake.offset[i] * scale).clamp(-SHAKE_MAX_OFFSET, SHAKE_MAX_OFFSET);
        }
    }
    shake_pose(free)
}
