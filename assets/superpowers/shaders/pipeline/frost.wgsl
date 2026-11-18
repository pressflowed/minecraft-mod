// Frost spreading over the ground: a flat quad, uv -1..1 over the frost's reach. Ice crystals
// sparkle in cells half a block wide; alpha times 100 is the reach in blocks.
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
    let r = length(input.uv);
    if (r > 1.0) {
        return vec4<f32>(0.0);
    }
    let reach = input.color.a * 100.0;
    let local = input.uv * reach;
    let cell = floor(local * 2.0);
    let h = hash21(cell);
    let inner = fract(local * 2.0) - 0.5;
    let star = exp(-dot(inner, inner) * 30.0);
    let sparkle = step(0.85, h) * star * (0.5 + 0.5 * sin(frame.time * (4.0 + h * 9.0) + h * 40.0));
    let rime = smoothstep(0.35, 0.8, fbm(local * 0.6)) * 0.22;
    let edge = smoothstep(0.85, 0.99, r) * 0.6;
    return vec4<f32>(input.color.rgb * (sparkle * 2.5 + rime + edge), 1.0);
}
