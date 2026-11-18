// A glowing shock bubble, drawn on a quad facing the camera: its edge brightest, as a shell is.
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
    let x = (r - 0.92) / 0.06;
    let rim = exp(-x * x);
    let inside = pow(r, 6.0) * 0.25 * step(r, 0.92);
    let flicker = 0.75 + 0.25 * noise(vec2<f32>(atan2(input.uv.y, input.uv.x) * 4.0, frame.time * 7.0));
    let col = mix(input.color.rgb, vec3<f32>(1.0), 0.3);
    return vec4<f32>(col * (rim * 1.6 + inside) * flicker * input.color.a, 1.0);
}
