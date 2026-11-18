// A black hole's accretion disk, a flat quad round it: uv -1..1 spans the disk's outer edge;
// it begins at 0.18, white-hot inside, orange, then violet, turning faster near the middle.
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

fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let p = input.uv;
    let r = length(p);
    if (r < 0.17 || r > 1.0) {
        return vec4<f32>(0.0);
    }
    let spin = frame.time * 2.4 / max(r, 0.22);
    let rot = vec2<f32>(p.x * cos(spin) - p.y * sin(spin), p.x * sin(spin) + p.y * cos(spin));
    let n = fbm(rot * 5.0 + vec2<f32>(3.1, 1.7));
    let streak = 0.5 + 0.5 * sin(r * 55.0 - frame.time * 5.0 + n * 7.0);
    let radial = smoothstep(0.17, 0.25, r) * (1.0 - smoothstep(0.55, 1.0, r));
    let heat = clamp(1.0 - (r - 0.17) / 0.7, 0.0, 1.0);
    let tint = mix(mix(vec3<f32>(0.35, 0.05, 0.6), vec3<f32>(1.0, 0.42, 0.08), smoothstep(0.1, 0.6, heat)),
                   vec3<f32>(1.0, 0.92, 0.7), smoothstep(0.6, 1.0, heat));
    // the side turning towards the camera is brighter
    let doppler = 1.0 + 0.6 * (rot.y / r);
    return vec4<f32>(tint * radial * (0.3 + 1.6 * n * streak) * doppler * 1.6 * input.color.a, 1.0);
}
