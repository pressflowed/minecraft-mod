//! The sky pass's WGSL, kept in a Rust file so the launcher packs it with the mod's source.
//!
//! One full-screen triangle, drawn additively over the finished world picture wherever the
//! world's depth is 0 (open sky). For each sky pixel it rebuilds the view ray from the inverse
//! camera matrix and adds three things, all scaled by how far into the night it is:
//!
//! - aurora curtains: one 2D ribbon field sampled on stacked horizontal planes high above the
//!   camera, so the ribbons rise as vertical curtains, green at the hem, cyan, then violet;
//! - shooting stars: a few slots, each firing now and then along a random path;
//! - extra stars that twinkle in soft colours.

pub const SKY_WGSL: &str = r#"
struct Frame {
    // Camera-relative clip space to camera-relative world space.
    inv_view_proj: mat4x4<f32>,
    // seconds (wrapping hourly), night 0..1, colour target width, height
    a: vec4<f32>,
    // depth width, height, camera x and z (wrapped to keep precision)
    b: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
// The world's Depth32Float depth, reversed (0 is the sky), read as a plain float texture.
@group(0) @binding(1) var scene_depth: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    // (-1,-1), (3,-1), (-1,3): one triangle over the whole screen.
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    let ndc = vec2<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let n = hash21(p);
    return vec2<f32>(n, hash21(p + vec2<f32>(n * 17.0, 3.1)));
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
    for (var i = 0; i < 3; i = i + 1) {
        sum = sum + amp * noise(p);
        p = p * 2.03 + vec2<f32>(1.7, 9.2);
        amp = amp * 0.5;
    }
    return sum;
}

// How bright the ribbon field is at a point on one of the aurora's planes (world blocks).
fn ribbons(xz: vec2<f32>, t: f32) -> f32 {
    let drift = vec2<f32>(t * 1.5, t * 0.6);
    let p = (xz + drift) * 0.0021;
    // Ribbons run roughly east-west and fold as the noise bends them.
    let bend = fbm(p * vec2<f32>(1.0, 0.6) + vec2<f32>(t * 0.010, -t * 0.006)) * 5.0;
    let v = p.y * 0.8 + bend;
    let tri = 1.0 - abs(fract(v) * 2.0 - 1.0);
    let sheet = pow(tri, 9.0);
    // Only some of the ribbons glow, each at its own brightness.
    let band = hash21(vec2<f32>(floor(v), 3.7));
    let lit = smoothstep(0.45, 0.75, band);
    // Fine vertical rays drifting along each ribbon.
    let rays = 0.45 + 0.55 * noise(vec2<f32>(xz.x * 0.045 + t * 0.35, v * 3.0));
    // Some stretches of ribbon are faint, some bright, and that changes slowly.
    let strength = smoothstep(0.2, 0.7, noise(p * 0.7 + vec2<f32>(-t * 0.004, t * 0.003)));
    return sheet * lit * rays * strength;
}

fn aurora(dir: vec3<f32>, t: f32, cam: vec2<f32>, jitter: f32) -> vec3<f32> {
    if (dir.y <= 0.02) {
        return vec3<f32>(0.0);
    }
    let layers = 28;
    var col = vec3<f32>(0.0);
    for (var i = 0; i < layers; i = i + 1) {
        // Each pixel starts its layers at its own offset, so they blend instead of banding.
        let f = (f32(i) + jitter) / f32(layers);
        // From the hem at 220 blocks up to the top at 520.
        let height = 220.0 + f * 300.0;
        let dist = height / dir.y;
        if (dist > 6000.0) {
            continue;
        }
        let xz = dir.xz * dist + cam;
        let s = ribbons(xz, t);
        let green = vec3<f32>(0.10, 1.00, 0.45);
        let cyan = vec3<f32>(0.05, 0.65, 1.00);
        let violet = vec3<f32>(0.75, 0.15, 1.00);
        let tint = mix(mix(green, cyan, smoothstep(0.0, 0.45, f)), violet, smoothstep(0.45, 1.0, f));
        // Brightest at the hem, fading towards the top, and with distance.
        let weight = pow(1.0 - f, 1.6) * exp(-dist * 0.00045);
        col = col + tint * s * weight;
    }
    let horizon = smoothstep(0.02, 0.22, dir.y);
    return col * horizon * 0.42;
}

// The sky folded onto a plane above the camera: shooting stars and stars live here.
fn sky_plane(dir: vec3<f32>) -> vec2<f32> {
    return dir.xz / (dir.y + 0.35);
}

fn shooting_stars(dir: vec3<f32>, t: f32) -> vec3<f32> {
    if (dir.y <= 0.05) {
        return vec3<f32>(0.0);
    }
    let s = sky_plane(dir);
    var col = vec3<f32>(0.0);
    for (var k = 0; k < 4; k = k + 1) {
        let fk = f32(k);
        let period = 4.0 + fk * 1.7;
        let tt = t / period + fk * 0.37;
        let id = floor(tt);
        let phase = fract(tt);
        let seed = vec2<f32>(id, fk * 7.31);
        // Not every period has one, and each lasts a fifth of its period.
        if (hash21(seed + 0.5) < 0.45 || phase > 0.2) {
            continue;
        }
        let progress = phase / 0.2;
        let start = (hash22(seed) - 0.5) * 2.4;
        let angle = hash21(seed + 2.7) * 6.2831853;
        let heading = vec2<f32>(cos(angle), sin(angle));
        let travel = 0.6 + hash21(seed + 5.1) * 0.5;
        let head = start + heading * travel * progress;
        let tail = head - heading * (0.18 + 0.12 * progress);
        let seg = head - tail;
        let along = clamp(dot(s - tail, seg) / dot(seg, seg), 0.0, 1.0);
        let off = s - (tail + seg * along);
        let d2 = dot(off, off);
        let fade = smoothstep(0.0, 0.15, progress) * (1.0 - smoothstep(0.7, 1.0, progress));
        let streak = exp(-d2 / 0.000012) * along * along;
        let flare = exp(-dot(s - head, s - head) / 0.00004);
        let warm = vec3<f32>(1.0, 0.92, 0.75);
        col = col + warm * (streak * 2.5 + flare * 4.0) * fade;
    }
    return col;
}

fn twinkles(dir: vec3<f32>, t: f32) -> vec3<f32> {
    if (dir.y <= 0.03) {
        return vec3<f32>(0.0);
    }
    let s = sky_plane(dir) * 90.0;
    let cell = floor(s);
    let h = hash21(cell);
    if (h < 0.982) {
        return vec3<f32>(0.0);
    }
    let centre = cell + 0.25 + hash22(cell + 1.3) * 0.5;
    let d2 = dot(s - centre, s - centre);
    let twinkle = 0.35 + 0.65 * pow(0.5 + 0.5 * sin(t * (1.5 + h * 9.0) + h * 61.0), 3.0);
    let hue = hash21(cell + 9.7);
    let tint = mix(vec3<f32>(0.6, 0.8, 1.0), vec3<f32>(1.0, 0.7, 0.9), hue);
    let horizon = smoothstep(0.03, 0.2, dir.y);
    return tint * exp(-d2 * 40.0) * twinkle * horizon * 1.4;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let night = frame.a.y;
    let size = frame.a.zw;
    let depth_size = frame.b.xy;
    let pixel = vec2<i32>(in.pos.xy * depth_size / size);
    let last = vec2<i32>(depth_size) - vec2<i32>(1, 1);
    let depth = textureLoad(scene_depth, clamp(pixel, vec2<i32>(0, 0), last), 0).r;
    // Only open sky: terrain, mobs and clouds keep what is in front of the aurora.
    if (depth > 0.0 || night <= 0.0) {
        return vec4<f32>(0.0);
    }
    let point = frame.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(point.xyz / point.w);
    let t = frame.a.x;
    let jitter = hash21(floor(in.pos.xy));
    var col = aurora(dir, t, frame.b.zw, jitter);
    col = col + shooting_stars(dir, t);
    col = col + twinkles(dir, t);
    return vec4<f32>(col * night, 0.0);
}
"#;
