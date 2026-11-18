// Shooting stars and twinkling stars, on one quad high over the camera. color: how dark the night is.
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

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let n = hash21(p);
    return vec2<f32>(n, hash21(p + vec2<f32>(n * 17.0, 3.1)));
}

fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let dir = normalize(input.position);
    if (dir.y <= 0.05) {
        return vec4<f32>(0.0);
    }
    let t = frame.time;
    let s = dir.xz / (dir.y + 0.35);
    var col = vec3<f32>(0.0);
    for (var k = 0; k < 4; k = k + 1) {
        let fk = f32(k);
        let tt = t / (4.0 + fk * 1.7) + fk * 0.37;
        let seed = vec2<f32>(floor(tt), fk * 7.31);
        let phase = fract(tt);
        if (hash21(seed + 0.5) < 0.45 || phase > 0.2) {
            continue;
        }
        let progress = phase / 0.2;
        let start = (hash22(seed) - 0.5) * 2.4;
        let angle = hash21(seed + 2.7) * 6.2831853;
        let heading = vec2<f32>(cos(angle), sin(angle));
        let head = start + heading * (0.6 + hash21(seed + 5.1) * 0.5) * progress;
        let tail = head - heading * (0.18 + 0.12 * progress);
        let seg = head - tail;
        let along = clamp(dot(s - tail, seg) / dot(seg, seg), 0.0, 1.0);
        let off = s - (tail + seg * along);
        let fade = smoothstep(0.0, 0.15, progress) * (1.0 - smoothstep(0.7, 1.0, progress));
        let streak = exp(-dot(off, off) / 0.000012) * along * along;
        let flare = exp(-dot(s - head, s - head) / 0.00004);
        col = col + vec3<f32>(1.0, 0.92, 0.75) * (streak * 2.5 + flare * 4.0) * fade;
    }
    let g = s * 90.0;
    let cell = floor(g);
    let h = hash21(cell);
    if (h > 0.982) {
        let centre = cell + 0.25 + hash22(cell + 1.3) * 0.5;
        let d2 = dot(g - centre, g - centre);
        let twinkle = 0.35 + 0.65 * pow(0.5 + 0.5 * sin(t * (1.5 + h * 9.0) + h * 61.0), 3.0);
        let tint = mix(vec3<f32>(0.6, 0.8, 1.0), vec3<f32>(1.0, 0.7, 0.9), hash21(cell + 9.7));
        col = col + tint * exp(-d2 * 40.0) * twinkle * 1.4;
    }
    return vec4<f32>(col * smoothstep(0.05, 0.2, dir.y) * input.color.rgb, 1.0);
}
