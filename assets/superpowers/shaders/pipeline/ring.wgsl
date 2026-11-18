// A shock ring racing across a plane. uv spans -1..1 over the quad; the ring is its rim.
// color: tint; alpha: how strong it still is.
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
    let w = 0.045;
    let x = (r - (1.0 - w * 1.5)) / w;
    let ang = atan2(input.uv.y, input.uv.x);
    let streaks = 0.55 + 0.45 * noise(vec2<f32>(ang * 9.0, frame.time * 1.5));
    let ring = exp(-x * x) * streaks;
    // the ground just behind the front glows as if scorched
    let wake = smoothstep(0.55, 0.97, r) * (1.0 - smoothstep(0.97, 1.0, r)) * 0.18;
    let hot = mix(input.color.rgb, vec3<f32>(1.0, 0.97, 0.9), exp(-x * x * 4.0) * 0.5);
    return vec4<f32>(hot * (ring * 3.0 + wake) * input.color.a, 1.0);
}
