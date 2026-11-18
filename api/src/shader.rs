//! The WGSL for both passes, kept in a Rust file so the launcher packs it with the mod's source.
//!
//! Each pixel rebuilds its view ray from the inverse camera matrix and, where the world's depth
//! holds a surface, that surface's position, so effects can light the ground, hide behind
//! hills and wrap round the player. Positions are camera-relative, in blocks.
//!
//! - `fs_lens` (replaces the picture): bends the copied picture round black holes (gravitational
//!   lensing, the event horizon black), ripples it along shock bubbles and frosts its edges.
//! - `fs_glow` (adds light): aurora on open night sky, shock rings on the ground and glowing
//!   shock bubbles, the punch's cone, laser beams and molten impacts, jagged lightning, frost
//!   sparkle, the accretion disk and photon ring, the dash's rainbow trail and speed lines.

pub const WGSL: &str = r#"
struct Effect {
    // kind, age (seconds), duration, seed
    k: vec4<f32>,
    // a point, and a size (radius, length, height or power)
    p: vec4<f32>,
    // a second point or a direction, and a flag
    q: vec4<f32>,
    // colour, intensity
    c: vec4<f32>,
};

struct Frame {
    inv_view_proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    // seconds (wrapping hourly), night 0..1, picture width, height
    a: vec4<f32>,
    // depth width, height, camera x and z (wrapped)
    b: vec4<f32>,
    // effect count, white flash, frost on the screen's edges, unused
    c: vec4<f32>,
    effects: array<Effect, 24>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var scene_depth: texture_2d<f32>;
@group(0) @binding(2) var scene: texture_2d<f32>;
@group(0) @binding(3) var scene_sampler: sampler;

const SHOCKWAVE: i32 = 1;
const PUNCH: i32 = 2;
const LASER: i32 = 3;
const LIGHTNING: i32 = 4;
const FROST: i32 = 5;
const GRAVITY: i32 = 6;
const DASH: i32 = 7;
const FAR: f32 = 1.0e9;

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

// ---- noise ----------------------------------------------------------------------------------

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

fn hue(h: f32) -> vec3<f32> {
    return clamp(abs(fract(h + vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
}

// ---- the view ---------------------------------------------------------------------------------

struct View {
    dir: vec3<f32>,
    // the surface this pixel shows, camera-relative, and how far it is (FAR for open sky)
    surface: vec3<f32>,
    dist: f32,
    has_surface: bool,
    uv: vec2<f32>,
};

fn view_of(pos: vec4<f32>, ndc: vec2<f32>) -> View {
    var v: View;
    let size = frame.a.zw;
    let depth_size = frame.b.xy;
    v.uv = pos.xy / size;
    let pixel = vec2<i32>(pos.xy * depth_size / size);
    let last = vec2<i32>(depth_size) - vec2<i32>(1, 1);
    let depth = textureLoad(scene_depth, clamp(pixel, vec2<i32>(0, 0), last), 0).r;
    let near = frame.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    v.dir = normalize(near.xyz / near.w);
    v.dist = FAR;
    v.has_surface = false;
    v.surface = vec3<f32>(0.0);
    if (depth > 0.0) {
        let s = frame.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
        v.surface = s.xyz / s.w;
        v.dist = length(v.surface);
        v.has_surface = true;
    }
    return v;
}

// The closest approach of the view ray to segment a..b: (distance, how far along the ray,
// how far along the segment 0..1).
fn ray_segment(dir: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let ab = b - a;
    let ab2 = max(dot(ab, ab), 1.0e-6);
    let d_ab = dot(dir, ab);
    let d_a = dot(dir, a);
    let denom = ab2 - d_ab * d_ab;
    var u = 0.0;
    if (denom > 1.0e-6) {
        u = (d_a * d_ab - dot(a, ab)) / denom;
    } else {
        u = 0.0;
    }
    u = clamp(u, 0.0, 1.0);
    let on_seg = a + ab * u;
    let s = max(dot(dir, on_seg), 0.0);
    return vec3<f32>(length(dir * s - on_seg), s, u);
}

// The closest approach of the view ray to point c: (distance, how far along the ray).
fn ray_point(dir: vec3<f32>, c: vec3<f32>) -> vec2<f32> {
    let s = dot(dir, c);
    return vec2<f32>(length(dir * s - c), s);
}

// ---- aurora, shooting stars and stars, on open night sky ---------------------------------------

fn ribbons(xz: vec2<f32>, t: f32) -> f32 {
    let drift = vec2<f32>(t * 1.5, t * 0.6);
    let p = (xz + drift) * 0.0021;
    let bend = fbm(p * vec2<f32>(1.0, 0.6) + vec2<f32>(t * 0.010, -t * 0.006)) * 5.0;
    let v = p.y * 0.8 + bend;
    let tri = 1.0 - abs(fract(v) * 2.0 - 1.0);
    let sheet = pow(tri, 9.0);
    let band = hash21(vec2<f32>(floor(v), 3.7));
    let lit = smoothstep(0.45, 0.75, band);
    let rays = 0.45 + 0.55 * noise(vec2<f32>(xz.x * 0.045 + t * 0.35, v * 3.0));
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
        let f = (f32(i) + jitter) / f32(layers);
        let height = 220.0 + f * 300.0;
        let dist = height / dir.y;
        if (dist > 6000.0) {
            continue;
        }
        let s = ribbons(dir.xz * dist + cam, t);
        let tint = mix(mix(vec3<f32>(0.10, 1.00, 0.45), vec3<f32>(0.05, 0.65, 1.00), smoothstep(0.0, 0.45, f)),
                       vec3<f32>(0.75, 0.15, 1.00), smoothstep(0.45, 1.0, f));
        col = col + tint * s * pow(1.0 - f, 1.6) * exp(-dist * 0.00045);
    }
    return col * smoothstep(0.02, 0.22, dir.y) * 0.42;
}

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
    let tint = mix(vec3<f32>(0.6, 0.8, 1.0), vec3<f32>(1.0, 0.7, 0.9), hash21(cell + 9.7));
    return tint * exp(-d2 * 40.0) * twinkle * smoothstep(0.03, 0.2, dir.y) * 1.4;
}

// ---- the powers' light --------------------------------------------------------------------------

// A shock wave: a ring racing over the ground and a glowing bubble round its centre.
fn shockwave(v: View, e: Effect) -> vec3<f32> {
    let t = clamp(e.k.y / e.k.z, 0.0, 1.0);
    let r = e.p.w * (1.0 - pow(1.0 - t, 3.0));
    let fade = pow(1.0 - t, 1.5);
    let center = e.p.xyz;
    var col = vec3<f32>(0.0);
    if (v.has_surface) {
        let off = v.surface - center;
        let d = length(off);
        let w = 0.45 + 1.6 * t;
        let x = (d - r) / w;
        let streaks = 0.55 + 0.45 * noise(normalize(off.xz + vec2<f32>(1.0e-4, 0.0)) * 9.0 + vec2<f32>(e.k.w, d * 0.15));
        let ring = exp(-x * x) * streaks;
        // just behind the front, the ground glows as if scorched
        let wake = smoothstep(r, r - 3.0, d) * smoothstep(r - 9.0, r - 3.0, d) * 0.12;
        col = col + e.c.rgb * (ring * 1.3 + wake) * fade;
    }
    let rp = ray_point(v.dir, center);
    if (rp.y > 0.0) {
        let shell = 0.45 + r * 0.04;
        let x = (rp.x - r) / shell;
        let front = rp.y - sqrt(max(r * r - rp.x * rp.x, 0.0));
        // seen from outside only: from inside, the bubble would wrap the whole view
        let outside = smoothstep(r * 0.9, r * 1.3, length(center));
        if (front < v.dist && outside > 0.0) {
            let flicker = 0.75 + 0.25 * noise(vec2<f32>(rp.x * 0.7 + e.k.w, frame.a.x * 6.0));
            col = col + mix(e.c.rgb, vec3<f32>(1.0), 0.35) * exp(-x * x) * fade * 0.7 * flicker * outside;
        }
    }
    return col;
}

// The punch: rings of shock flying out from the fist, one after another, growing as they go.
fn punch(v: View, e: Effect) -> vec3<f32> {
    let t = clamp(e.k.y / e.k.z, 0.0, 1.0);
    let axis = normalize(e.q.xyz);
    let denom = dot(v.dir, axis);
    var col = vec3<f32>(0.0);
    if (abs(denom) < 1.0e-3) {
        return col;
    }
    for (var k = 0; k < 4; k = k + 1) {
        let tk = clamp(t * 1.5 - f32(k) * 0.14, 0.0, 1.0);
        if (tk <= 0.0 || tk >= 1.0) {
            continue;
        }
        let c = e.p.xyz + axis * (1.5 + tk * e.p.w);
        let tp = dot(c, axis) / denom;
        if (tp <= 0.0 || tp > v.dist) {
            continue;
        }
        let radius = 0.5 + tk * 5.5;
        let d = abs(length(v.dir * tp - c) - radius);
        let w = 0.10 + tk * 0.45;
        let ring = exp(-(d / w) * (d / w)) * pow(1.0 - tk, 1.5);
        col = col + mix(vec3<f32>(1.0, 0.97, 0.9), e.c.rgb, tk) * ring * 1.4;
    }
    return col;
}

// A laser eye's beam: a white-hot core in a coloured glow, and where it hits, molten light.
fn laser(v: View, e: Effect) -> vec3<f32> {
    let power = e.p.w;
    let origin = e.p.xyz;
    let to = e.q.xyz;
    let t = frame.a.x;
    let flicker = 0.82 + 0.18 * noise(vec2<f32>(t * 55.0, e.k.w * 13.0));
    var col = vec3<f32>(0.0);
    let rs = ray_segment(v.dir, origin, to);
    if (rs.y < v.dist + 0.3) {
        // Measured as an angle, and never wider than a thin line on the screen: right by the
        // eyes a beam would otherwise fill the view.
        let near = max(rs.y, 0.05);
        let width = min((0.018 + 0.012 * power) / near, 0.004 + 0.0015 * power);
        let x = (rs.x / near) / width;
        let out_from_eyes = smoothstep(0.1, 0.5, rs.y);
        let core = exp(-x * x) * out_from_eyes;
        // (no halo where the ray passes beyond the beam's ends: that would draw a disc round them)
        let inside = step(0.001, rs.z) * step(rs.z, 0.999);
        let halo = exp(-abs(x) / 2.2) * 0.3 * out_from_eyes * inside;
        // a heat shimmer running down the beam
        let pulse = 0.7 + 0.3 * sin(rs.z * 40.0 - t * 30.0);
        col = col + (vec3<f32>(1.0) * core * 1.4 + e.c.rgb * (core * 1.2 + halo * pulse)) * flicker;
    }
    if (e.q.w > 0.5) {
        // the impact: a flaring ball of light, and the surface round it glowing hot
        let rp = ray_point(v.dir, to);
        if (rp.y > 0.0 && rp.y < v.dist + 0.8) {
            let size = 0.12 + 0.06 * power;
            let ball = exp(-rp.x * rp.x / (size * size)) * 1.6 + exp(-rp.x / (size * 3.0)) * 0.35;
            col = col + mix(e.c.rgb, vec3<f32>(1.0, 0.95, 0.8), 0.5) * ball * flicker;
        }
        if (v.has_surface) {
            let d = length(v.surface - to);
            let hot = exp(-d * d / (0.25 + 0.18 * power));
            let melt = 0.6 + 0.4 * noise(v.surface.xz * 6.0 + vec2<f32>(t * 3.0, 0.0));
            col = col + vec3<f32>(1.0, 0.35, 0.05) * hot * melt * (0.6 + 0.2 * power);
        }
    }
    return col;
}

// The bolt's sideways wander at height h: big zigzags and small jitter.
fn jag(h: f32, seed: f32) -> vec2<f32> {
    let big = vec2<f32>(noise(vec2<f32>(h * 0.07, seed)), noise(vec2<f32>(h * 0.07, seed + 7.3))) - 0.5;
    let small = vec2<f32>(noise(vec2<f32>(h * 0.6, seed + 3.1)), noise(vec2<f32>(h * 0.6, seed + 11.7))) - 0.5;
    return big * 10.0 + small * 1.6;
}

// Lightning: a jagged bolt from far above down to the ground, striking again as it flickers.
fn lightning(v: View, e: Effect) -> vec3<f32> {
    let age = e.k.y;
    let t = clamp(age / e.k.z, 0.0, 1.0);
    let flick = select(step(0.4, hash21(vec2<f32>(floor(age * 22.0), e.k.w))), 1.0, age < 0.08);
    let strength = flick * pow(1.0 - t, 1.5);
    if (strength <= 0.0) {
        return vec3<f32>(0.0);
    }
    let base = e.p.xyz;
    let height = e.p.w;
    var off = vec2<f32>(0.0);
    var rs = vec3<f32>(0.0);
    for (var i = 0; i < 3; i = i + 1) {
        let foot = base + vec3<f32>(off.x, 0.0, off.y);
        rs = ray_segment(v.dir, foot, foot + vec3<f32>(0.0, height, 0.0));
        off = jag(rs.z * height, e.k.w) * smoothstep(0.0, 0.05, rs.z);
    }
    var col = vec3<f32>(0.0);
    if (rs.y < v.dist + 1.0) {
        let x = rs.x / 0.18;
        let core = exp(-x * x);
        let glow = exp(-rs.x / 2.5) * 0.35;
        col = col + (vec3<f32>(1.0) * core * 3.0 + e.c.rgb * (core * 2.0 + glow)) * strength;
    }
    if (v.has_surface) {
        let d = length(v.surface - base);
        col = col + e.c.rgb * exp(-d * d / 30.0) * 1.1 * strength;
    }
    return col;
}

// Frost: as the nova spreads, every surface it reaches sparkles with ice crystals.
fn frost(v: View, e: Effect) -> vec3<f32> {
    if (!v.has_surface) {
        return vec3<f32>(0.0);
    }
    let age = e.k.y;
    let t = clamp(age / e.k.z, 0.0, 1.0);
    let r = e.p.w * (1.0 - pow(1.0 - min(age / 0.9, 1.0), 3.0));
    let d = length(v.surface - e.p.xyz);
    if (d > r) {
        return vec3<f32>(0.0);
    }
    // world-locked crystal cells, so the sparkle stays put as the camera moves
    let world = v.surface + vec3<f32>(frame.b.z, 0.0, frame.b.w);
    let cell = floor(world * 3.0);
    let h = hash21(cell.xz + vec2<f32>(cell.y * 7.13, cell.y * 1.7));
    let local = fract(world * 3.0) - 0.5;
    let star = exp(-dot(local, local) * 28.0);
    let sparkle = step(0.86, h) * star * (0.5 + 0.5 * sin(frame.a.x * (4.0 + h * 9.0) + h * 40.0));
    let edge = smoothstep(r - 2.5, r, d);
    let fade = 1.0 - smoothstep(0.6, 1.0, t);
    return e.c.rgb * (sparkle * 2.2 + edge * 0.45) * fade;
}

// The black hole's light: a swirling accretion disk, brighter on the side turning towards
// the camera, a thin photon ring at the edge of the shadow, and a violet haze round it.
fn gravity(v: View, e: Effect) -> vec3<f32> {
    let age = e.k.y;
    let dur = e.k.z;
    let grow = smoothstep(0.0, 0.7, age) * (1.0 - smoothstep(dur - 0.35, dur, age));
    let rs = e.p.w * grow;
    if (rs <= 0.01) {
        return vec3<f32>(0.0);
    }
    let center = e.p.xyz;
    let t = frame.a.x;
    let normal = normalize(vec3<f32>(sin(e.k.w) * 0.35, 1.0, cos(e.k.w) * 0.35));
    var col = vec3<f32>(0.0);
    let denom = dot(v.dir, normal);
    if (abs(denom) > 1.0e-4) {
        let tp = dot(center, normal) / denom;
        if (tp > 0.0 && tp < v.dist) {
            let p = v.dir * tp - center;
            let rr = length(p);
            if (rr > rs * 1.25 && rr < rs * 7.0) {
                let u = normalize(cross(normal, vec3<f32>(0.0, 0.0, 1.0)));
                let w = cross(normal, u);
                let local = vec2<f32>(dot(p, u), dot(p, w)) / rs;
                // inner rings turn faster (as orbits do)
                let spin = t * 3.2 / max(length(local), 0.5);
                let rot = vec2<f32>(local.x * cos(spin) - local.y * sin(spin), local.x * sin(spin) + local.y * cos(spin));
                let n = fbm(rot * 1.8 + vec2<f32>(e.k.w, 0.0));
                let streak = 0.5 + 0.5 * sin(length(local) * 9.0 - t * 4.0 + n * 6.0);
                let radial = smoothstep(rs * 1.25, rs * 1.7, rr) * (1.0 - smoothstep(rs * 3.5, rs * 7.0, rr));
                let heat = clamp(1.0 - (rr - rs * 1.25) / (rs * 5.0), 0.0, 1.0);
                let tint = mix(mix(vec3<f32>(0.35, 0.05, 0.55), vec3<f32>(1.0, 0.42, 0.08), smoothstep(0.1, 0.6, heat)), vec3<f32>(1.0, 0.92, 0.7), smoothstep(0.6, 1.0, heat));
                let moving = normalize(cross(normal, p));
                let doppler = 1.0 + 0.75 * dot(moving, -v.dir);
                col = col + tint * radial * (0.25 + 1.0 * n * streak) * doppler * 1.1;
            }
        }
    }
    let rp = ray_point(v.dir, center);
    if (rp.y > 0.0 && rp.y - rs < v.dist) {
        let x = (rp.x - rs * 1.12) / (0.05 * rs);
        col = col + vec3<f32>(1.0, 0.85, 0.6) * exp(-x * x) * 1.5;
        if (rp.x > rs) {
            col = col + vec3<f32>(0.45, 0.15, 0.9) * exp(-(rp.x - rs) / (rs * 2.2)) * 0.35;
        }
    }
    return col * grow;
}

// The dash: a rainbow trail of afterimages along the path, and speed lines on the screen.
fn dash(v: View, e: Effect, ndc: vec2<f32>) -> vec3<f32> {
    let age = e.k.y;
    let t = clamp(age / e.k.z, 0.0, 1.0);
    var col = vec3<f32>(0.0);
    let rs = ray_segment(v.dir, e.p.xyz, e.q.xyz);
    if (rs.y < v.dist + 0.5) {
        let reach = smoothstep(0.0, 0.25, age);
        let w = 0.35 + 0.5 * rs.z;
        let x = rs.x / w;
        let band = exp(-x * x) * step(rs.z, reach);
        let ghosts = 0.75 + 0.25 * sin(rs.z * 24.0 - age * 18.0);
        let core = exp(-x * x * 6.0) * 0.6;
        col = col + (hue(rs.z * 1.3 - age * 1.5) + vec3<f32>(core)) * band * ghosts * 1.3 * pow(1.0 - t, 2.0);
    }
    if (age < 0.6) {
        let ang = atan2(ndc.y, ndc.x);
        let id = floor(ang * 70.0);
        let line = step(0.9, hash21(vec2<f32>(id, floor(age * 30.0))));
        let edge = smoothstep(0.45, 1.1, length(ndc));
        col = col + vec3<f32>(0.85, 0.95, 1.0) * line * edge * 0.35 * (1.0 - age / 0.6);
    }
    return col;
}

@fragment
fn fs_glow(in: VsOut) -> @location(0) vec4<f32> {
    let v = view_of(in.pos, in.ndc);
    let t = frame.a.x;
    var col = vec3<f32>(0.0);
    let night = frame.a.y;
    if (!v.has_surface && night > 0.0) {
        let jitter = hash21(floor(in.pos.xy));
        col = col + (aurora(v.dir, t, frame.b.zw, jitter) + shooting_stars(v.dir, t) + twinkles(v.dir, t)) * night;
    }
    let count = i32(frame.c.x);
    for (var i = 0; i < 24; i = i + 1) {
        if (i >= count) {
            break;
        }
        let e = frame.effects[i];
        let kind = i32(e.k.x + 0.5);
        if (kind == SHOCKWAVE) {
            col = col + shockwave(v, e);
        } else if (kind == PUNCH) {
            col = col + punch(v, e);
        } else if (kind == LASER) {
            col = col + laser(v, e);
        } else if (kind == LIGHTNING) {
            col = col + lightning(v, e);
        } else if (kind == FROST) {
            col = col + frost(v, e);
        } else if (kind == GRAVITY) {
            col = col + gravity(v, e);
        } else if (kind == DASH) {
            col = col + dash(v, e, in.ndc);
        }
    }
    col = col + vec3<f32>(1.0, 0.95, 0.85) * frame.c.y;
    return vec4<f32>(col, 0.0);
}

// Where a camera-relative direction lands on the screen, as a texture coordinate.
fn screen_uv(dir: vec3<f32>) -> vec2<f32> {
    let clip = frame.view_proj * vec4<f32>(dir * 10.0, 1.0);
    let ndc = clip.xy / max(clip.w, 1.0e-4);
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

@fragment
fn fs_lens(in: VsOut) -> @location(0) vec4<f32> {
    let v = view_of(in.pos, in.ndc);
    var dir = v.dir;
    var dark = 0.0;
    var ripple = vec2<f32>(0.0);
    var frost_tint = 0.0;
    var ground_frost = 0.0;
    let count = i32(frame.c.x);
    for (var i = 0; i < 24; i = i + 1) {
        if (i >= count) {
            break;
        }
        let e = frame.effects[i];
        let kind = i32(e.k.x + 0.5);
        if (kind == GRAVITY) {
            let age = e.k.y;
            let grow = smoothstep(0.0, 0.7, age) * (1.0 - smoothstep(e.k.z - 0.35, e.k.z, age));
            let rs = e.p.w * grow;
            let center = e.p.xyz;
            let rp = ray_point(dir, center);
            // only what lies behind the hole is bent: the hole hides behind hills
            if (rs > 0.01 && rp.y > 0.0 && v.dist > rp.y - rs) {
                if (rp.x < rs) {
                    dark = max(dark, smoothstep(rs, rs * 0.85, rp.x));
                } else {
                    // light passing at distance b is bent towards the hole by about rs/b
                    let toward = normalize(center - dir * rp.y);
                    let bend = 1.6 * rs / rp.x * (rs / max(rp.y, 1.0)) * 4.0;
                    let fall = exp(-(rp.x - rs) / (rs * 6.0));
                    dir = normalize(dir + toward * min(bend, 0.9) * fall);
                }
            }
        } else if (kind == SHOCKWAVE) {
            let t = clamp(e.k.y / e.k.z, 0.0, 1.0);
            let r = e.p.w * (1.0 - pow(1.0 - t, 3.0));
            let rp = ray_point(v.dir, e.p.xyz);
            if (rp.y > 0.0) {
                let x = (rp.x - r) / (0.6 + r * 0.06);
                let c = screen_uv(normalize(e.p.xyz));
                let away = v.uv - c;
                ripple = ripple + normalize(away + vec2<f32>(1.0e-5, 0.0)) * exp(-x * x) * pow(1.0 - t, 2.0) * 0.012;
            }
        } else if (kind == FROST) {
            frost_tint = max(frost_tint, frame.c.z);
            if (v.has_surface) {
                let r = e.p.w * (1.0 - pow(1.0 - min(e.k.y / 0.9, 1.0), 3.0));
                let d = length(v.surface - e.p.xyz);
                let world = v.surface + vec3<f32>(frame.b.z, 0.0, frame.b.w);
                let rime = smoothstep(0.3, 0.75, fbm(world.xz * 0.9 + vec2<f32>(world.y * 0.37, 0.0)));
                ground_frost = max(ground_frost, step(d, r) * (0.2 + 0.45 * rime) * (1.0 - smoothstep(0.7, 1.0, e.k.y / e.k.z)));
            }
        }
    }
    var uv = v.uv + ripple;
    if (any(dir != v.dir)) {
        uv = screen_uv(dir) + ripple;
    }
    let sampled = textureSampleLevel(scene, scene_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    var col = sampled.rgb;
    col = mix(col, vec3<f32>(0.78, 0.9, 1.0) * (0.35 + 0.65 * max(max(col.r, col.g), col.b) * 1.6), ground_frost);
    if (frost_tint > 0.0) {
        let edge = smoothstep(0.8, 1.35, length(in.ndc * vec2<f32>(1.0, 0.85)));
        let crystals = fbm(v.uv * vec2<f32>(40.0, 24.0)) * 0.6 + fbm(v.uv * vec2<f32>(140.0, 90.0)) * 0.4;
        let ice = smoothstep(0.55, 0.85, crystals * 0.8 + edge * 0.5) * edge;
        col = mix(col, vec3<f32>(0.8, 0.93, 1.0), ice * frost_tint * 0.6);
    }
    col = col * (1.0 - dark);
    return vec4<f32>(col, sampled.a);
}
"#;
