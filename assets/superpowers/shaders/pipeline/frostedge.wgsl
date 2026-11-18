// Frost creeping in from the edges of the view: uv 0..1 across the quad in front of the camera.
// alpha: how strong.
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
    let c = input.uv * 2.0 - 1.0;
    let edge = smoothstep(0.35, 0.95, length(c * vec2<f32>(0.75, 1.0)));
    let crystals = fbm(input.uv * vec2<f32>(40.0, 24.0)) * 0.6 + fbm(input.uv * vec2<f32>(140.0, 90.0)) * 0.4;
    let ice = smoothstep(0.5, 0.85, crystals * 0.8 + edge * 0.5) * edge;
    return vec4<f32>(vec3<f32>(0.8, 0.93, 1.0), ice * input.color.a * 0.75);
}
