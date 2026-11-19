fn free_shake(game: &Game<'_>, partial: f32) -> glam::Mat4 {
    if !game.playing() {
        return glam::Mat4::IDENTITY;
    }
    let mut shake = Shake::default();
    game.native_camera_shake(partial, &mut shake);
    if shake == Shake::default() {
        return glam::Mat4::IDENTITY;
    }
    let mut scale = game.screen_effect_scale();
    if scale.is_finite() {
        scale = scale.clamp(0.0, 1.0);
    } else {
        scale = 1.0;
    }
    let mut free = Shake::default();
    if shake.pitch.is_finite() {
        free.pitch = shake.pitch.rem_euclid(360.0);
    }
    if shake.yaw.is_finite() {
        free.yaw = shake.yaw.rem_euclid(360.0);
    }
    if shake.roll.is_finite() {
        free.roll = shake.roll.rem_euclid(360.0);
    }
    for i in 0..3 {
        if shake.offset[i].is_finite() {
            free.offset[i] = (shake.offset[i] * scale).clamp(-SHAKE_MAX_OFFSET, SHAKE_MAX_OFFSET);
        }
    }
    shake_pose(free)
}
