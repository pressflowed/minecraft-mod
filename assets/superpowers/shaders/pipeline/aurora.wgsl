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

fn ribbons(xz: vec2<f32>, t: f32) -> f32 {
    let p = (xz + vec2<f32>(t * 1.5, t * 0.6)) * 0.0021;
    let bend = fbm(p * vec2<f32>(1.0, 0.6) + vec2<f32>(t * 0.010, -t * 0.006)) * 5.0;
    let v = p.y * 0.8 + bend;
    let tri = 1.0 - abs(fract(v) * 2.0 - 1.0);
    let sheet = pow(tri, 9.0);
    let lit = smoothstep(0.45, 0.75, hash21(vec2<f32>(floor(v), 3.7)));
    let rays = 0.45 + 0.55 * noise(vec2<f32>(xz.x * 0.045 + t * 0.35, v * 3.0));
    let strength = smoothstep(0.2, 0.7, noise(p * 0.7 + vec2<f32>(-t * 0.004, t * 0.003)));
    return sheet * lit * rays * strength;
}

fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let f = input.color.a;
    let dist = length(input.position);
    let up = input.position.y / max(dist, 1.0);
    let s = ribbons(input.uv * 1000.0, frame.time);
    let tint = mix(mix(vec3<f32>(0.10, 1.00, 0.45), vec3<f32>(0.05, 0.65, 1.00), smoothstep(0.0, 0.45, f)),
                   vec3<f32>(0.75, 0.15, 1.00), smoothstep(0.45, 1.0, f));
    let fade = pow(1.0 - f, 1.6) * exp(-dist * 0.00045) * smoothstep(0.02, 0.22, up);
    return vec4<f32>(tint * s * fade * input.color.rgb * 0.8, 1.0);
}
