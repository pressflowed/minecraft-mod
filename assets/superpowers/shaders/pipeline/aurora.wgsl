// One layer of the aurora, high over the camera: uv is the world's x and z in kilometres, so
// the ribbons stay put as the player moves. alpha: the layer's height, 0 (the hem) to 1;
// color: how dark the night is.
fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(start: vec2<f32>) -> f32 {
    var p = start;
    var sum = 0.0;
    var amp = 0.5;
    for (var i = 0; i < 4; i = i + 1) {
        sum = sum + amp * noise(p);
        p = p * 2.03 + vec2<f32>(1.7, 9.2);
        amp = amp * 0.5;
    }
    return sum;
}

// One curtain of the aurora, hanging in the sky: a strip of quads along its fold, uv.x along it
// in hundreds of blocks, uv.y from its hem (0) to its top (1). alpha: how bright this curtain
// is; color: how dark the night is.
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let x = input.uv.x;
    let y = clamp(input.uv.y, 0.0, 1.0);
    let t = frame.time;
    // fine rays running up the curtain, drifting along it
    let rays = 0.35 + 0.65 * noise(vec2<f32>(x * 9.0 + t * 0.6, t * 0.15)) * noise(vec2<f32>(x * 23.0 - t * 0.9, 4.0));
    // brighter and dimmer stretches that slide along
    let stretch = smoothstep(0.15, 0.75, fbm(vec2<f32>(x * 0.6 - t * 0.05, t * 0.02)));
    // a sharp bright hem, fading upward
    let hem = smoothstep(0.0, 0.04, y) * pow(1.0 - y, 1.4);
    let tint = mix(mix(vec3<f32>(0.10, 1.00, 0.45), vec3<f32>(0.05, 0.65, 1.00), smoothstep(0.05, 0.5, y)),
                   vec3<f32>(0.75, 0.15, 1.00), smoothstep(0.5, 1.0, y));
    let dist = length(input.position);
    let up = input.position.y / max(dist, 1.0);
    let fade = exp(-dist * 0.00035) * smoothstep(0.0, 0.12, up);
    return vec4<f32>(tint * rays * stretch * hem * fade * input.color.rgb * input.color.a * 1.4, 1.0);
}
