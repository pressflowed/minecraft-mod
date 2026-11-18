// Speed lines streaming past the edges of the view: uv -1..1 across the quad in front of the
// camera. alpha: how fast.
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
    let c = input.uv;
    let ang = atan2(c.y, c.x);
    let id = floor(ang * 140.0);
    let line = step(0.93, hash21(vec2<f32>(id, floor(frame.time * 24.0))));
    let edge = smoothstep(0.55, 1.05, length(c * vec2<f32>(0.6, 1.0)));
    return vec4<f32>(vec3<f32>(0.85, 0.95, 1.0) * line * edge * 0.3 * input.color.a, 1.0);
}
