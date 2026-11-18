// A beam of light: uv.x runs along it in blocks, uv.y across it from -1 to 1.
// A white-hot core in a coloured glow, with energy racing down it. Lightning uses it too.
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
    let y = input.uv.y;
    let core = exp(-(y * y) / 0.045);
    let halo = exp(-abs(y) * 3.0) * 0.55;
    let pulse = 0.72 + 0.28 * sin(input.uv.x * 1.7 - frame.time * 42.0);
    let flicker = 0.85 + 0.15 * noise(vec2<f32>(frame.time * 55.0, input.uv.x * 0.3));
    let col = input.color.rgb * (core * 2.2 + halo * pulse) + vec3<f32>(1.0) * core * 2.4;
    return vec4<f32>(col * flicker * input.color.a, 1.0);
}
