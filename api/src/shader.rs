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
    // effect count, white flash, frost on the screen's edges, speed lines
    c: vec4<f32>,
    // the storms' air: rain, darkness, heat shimmer, a red-orange cast
    d: vec4<f32>,
    // how many trail points there are
    e: vec4<f32>,
    effects: array<Effect, 48>,
    // vapour and smoke trails: camera-relative points, w = kind * 2 + how fresh (below 0 a break)
    trails: array<vec4<f32>, 96>,
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
const ORB: i32 = 8;
const TORNADO: i32 = 9;
const HURRICANE: i32 = 10;
const TSUNAMI: i32 = 11;
const MUSHROOM: i32 = 12;
const SMOKE: i32 = 13;
const FLARE: i32 = 14;
const METEOR: i32 = 15;
const BOX: i32 = 16;
const TETHER: i32 = 17;
const MARKER: i32 = 18;
const FLAME: i32 = 19;
const HAZE: i32 = 20;
const VAPOR: i32 = 21;
const TRACER: i32 = 22;
const RETICLE: i32 = 23;
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

// A fireball: a ball of light with a white-hot heart and a turbulent glow, growing then fading.
fn orb(v: View, e: Effect) -> vec3<f32> {
    let age = e.k.y;
    let t = clamp(age / e.k.z, 0.0, 1.0);
    let r = e.p.w * (0.35 + 0.65 * (1.0 - pow(1.0 - min(t * 3.0, 1.0), 3.0)));
    let rp = ray_point(v.dir, e.p.xyz);
    if (rp.y <= 0.0 || rp.y - r > v.dist) {
        return vec3<f32>(0.0);
    }
    let x = rp.x / r;
    let boil = 0.7 + 0.3 * fbm(vec2<f32>(atan2(v.dir.y, v.dir.x) * 6.0 + frame.a.x, x * 3.0 - frame.a.x * 1.5));
    let core = exp(-x * x * 6.0) * 2.2;
    let glow = exp(-x * 2.2) * 0.8;
    let col = mix(e.c.rgb, vec3<f32>(1.0, 0.97, 0.9), exp(-x * x * 10.0));
    return col * (core + glow) * boil * pow(1.0 - t, 1.5);
}

// ---- the Storm Scepter's skies, and the Gravity Gun ---------------------------------------------

// How lit the world's air is: dim at night and under a storm.
fn daylight() -> f32 {
    return mix(1.0, 0.16, frame.a.y) * (1.0 - 0.45 * frame.d.y);
}

// The view ray's span inside the upright cylinder round `c` (radius r, heights y0 to y1, all
// camera-relative): (enter, leave), leave below enter when it misses.
fn cylinder(dir: vec3<f32>, c: vec3<f32>, r: f32, y0: f32, y1: f32) -> vec2<f32> {
    var s0 = 0.0;
    var s1 = 1.0e9;
    let d2 = dot(dir.xz, dir.xz);
    let k = dot(c.xz, c.xz) - r * r;
    if (d2 > 1.0e-6) {
        let b = dot(dir.xz, c.xz);
        let disc = b * b - d2 * k;
        if (disc < 0.0) {
            return vec2<f32>(1.0, -1.0);
        }
        let q = sqrt(disc);
        s0 = (b - q) / d2;
        s1 = (b + q) / d2;
    } else if (k > 0.0) {
        return vec2<f32>(1.0, -1.0);
    }
    if (abs(dir.y) > 1.0e-6) {
        let ta = y0 / dir.y;
        let tb = y1 / dir.y;
        s0 = max(s0, min(ta, tb));
        s1 = min(s1, max(ta, tb));
    } else if (y0 > 0.0 || y1 < 0.0) {
        return vec2<f32>(1.0, -1.0);
    }
    return vec2<f32>(max(s0, 0.0), s1);
}

fn rotate2(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c);
}

// Front to back: `col` with opacity `a` behind what is already there.
fn over(acc: vec4<f32>, col: vec3<f32>, a: f32) -> vec4<f32> {
    return acc + vec4<f32>(col * a, a) * (1.0 - acc.a);
}

// A tornado: a funnel snaking down from a wall cloud, spinning, a ring of debris at its foot.
fn tornado(v: View, e: Effect, jitter: f32) -> vec4<f32> {
    let age = e.k.y;
    let dur = e.k.z;
    let env = smoothstep(0.0, 2.5, age) * (1.0 - smoothstep(dur - 3.0, dur, age));
    if (env <= 0.001) {
        return vec4<f32>(0.0);
    }
    let base = e.p.xyz;
    let r0 = e.p.w * (0.4 + 0.6 * env);
    let height = e.q.x;
    let rt = e.q.y;
    let ef = e.q.z;
    let t = frame.a.x;
    let touch = smoothstep(0.0, 3.0, age);
    let bound = max(rt * 2.2, r0 * 3.4) + height * 0.1;
    let span = cylinder(v.dir, base, bound, base.y - 4.0, base.y + height);
    let far = min(span.y, v.dist);
    if (far <= span.x) {
        return vec4<f32>(0.0);
    }
    let steps = 40;
    let ds = (far - span.x) / f32(steps);
    var acc = vec4<f32>(0.0);
    var s = span.x + ds * jitter;
    for (var i = 0; i < steps; i = i + 1) {
        let p = v.dir * s;
        let h = p.y - base.y;
        let hn = clamp(h / height, 0.0, 1.0);
        let axis = base.xz + vec2<f32>(sin(h * 0.035 + t * 0.6 + e.k.w), cos(h * 0.028 + t * 0.45 + e.k.w * 1.3)) * h * 0.09;
        let local = p.xz - axis;
        let d = length(local);
        let r = mix(r0 * 0.6, rt, pow(hn, 2.2));
        let spun = rotate2(local, t * (2.5 + ef * 0.5) - h * 0.04);
        let n = fbm(spun * (2.6 / max(r, 1.0)) + vec2<f32>(h * 0.06, -t * 0.4 + h * 0.02));
        // the funnel, forming downwards as it touches down
        var dens = smoothstep(r * 1.25, r * 0.7, d + (n - 0.5) * r * 0.8) * (0.55 + 0.9 * n);
        dens = dens * smoothstep(1.0 - touch, 1.0 - touch + 0.08, hn + 0.04);
        // the debris cloud whirling round its foot
        let foot_h = 12.0 + ef * 4.0;
        let foot_r = r0 * (3.4 - 1.8 * clamp(h / foot_h, 0.0, 1.0));
        let foot = smoothstep(foot_h, 0.0, h) * smoothstep(foot_r, foot_r * 0.35, d + (n - 0.5) * r0 * 2.2) * touch;
        dens = max(dens, foot * (0.1 + 1.1 * n * n));
        // the wall cloud it hangs from
        let cloud = smoothstep(0.8, 0.97, hn) * smoothstep(rt * 2.2, rt * 0.7, d + (n - 0.5) * rt);
        dens = max(dens, cloud * (0.5 + 0.6 * n)) * env;
        if (dens > 0.01) {
            let a = 1.0 - exp(-dens * ds * 0.25);
            let shade = mix(0.55, 1.05, hn) * (0.7 + 0.5 * n);
            let tint = mix(vec3<f32>(0.46, 0.38, 0.29), e.c.rgb, smoothstep(0.0, 0.2, hn));
            acc = over(acc, tint * shade * daylight(), a);
            if (acc.a > 0.97) {
                break;
            }
        }
        s = s + ds;
    }
    return acc;
}

// A storm's cloud deck: a slab of cloud overhead, wound into spiral arms round a clear eye for
// a hurricane (`q.w` 1), towering at the eye wall.
fn storm_deck(v: View, e: Effect, jitter: f32) -> vec4<f32> {
    let age = e.k.y;
    let dur = e.k.z;
    let env = smoothstep(0.0, 4.0, age) * (1.0 - smoothstep(dur - 5.0, dur, age));
    if (env <= 0.001) {
        return vec4<f32>(0.0);
    }
    let c = e.p.xyz;
    let big = e.p.w;
    let base = c.y + e.q.x;
    let thick = e.q.y;
    let eye = e.q.z;
    let spiral = e.q.w;
    let t = frame.a.x;
    let span = cylinder(v.dir, c, big, base, base + thick * 1.7);
    let far = min(span.y, v.dist);
    if (far <= span.x) {
        return vec4<f32>(0.0);
    }
    let steps = 24;
    let ds = (far - span.x) / f32(steps);
    var acc = vec4<f32>(0.0);
    var s = span.x + ds * jitter;
    for (var i = 0; i < steps; i = i + 1) {
        let p = v.dir * s;
        let off = p.xz - c.xz;
        let d = length(off);
        let ang = atan2(off.y, off.x);
        let arms = mix(1.0, 0.5 + 0.5 * sin(2.0 * ang + log(max(d, 1.0)) * 5.0 - t * 0.35), spiral);
        let turned = rotate2(off, t * 0.12 * spiral * 60.0 / max(d, 20.0));
        let n = fbm(turned * 0.025 + vec2<f32>(e.k.w, 0.0));
        let cover = smoothstep(0.32, 0.62, n * 0.8 + arms * 0.45);
        var top = base + thick;
        var clear = 1.0;
        if (eye > 0.0) {
            clear = smoothstep(eye, eye * 1.5, d);
            top = top + thick * 0.7 * exp(-pow((d - eye * 1.6) / eye, 2.0));
        }
        let hy = (p.y - base) / max(top - base, 1.0);
        let profile = smoothstep(0.0, 0.25, hy) * smoothstep(1.0, 0.55, hy);
        let edge = 1.0 - smoothstep(big * 0.7, big, d);
        let dens = cover * clear * edge * profile * env;
        if (dens > 0.01) {
            let a = 1.0 - exp(-dens * ds * 0.07);
            let shade = mix(0.45, 1.0, clamp(hy, 0.0, 1.0)) * (0.75 + 0.4 * n);
            acc = over(acc, e.c.rgb * shade * daylight(), a);
            if (acc.a > 0.97) {
                break;
            }
        }
        s = s + ds;
    }
    return acc;
}

// The tsunami's water height over its base, `a` blocks ahead of the crest and `b` across.
fn wave_height(a: f32, b: f32, height: f32, width: f32, crash: f32, t: f32) -> f32 {
    let across = smoothstep(width * 0.5, width * 0.5 - 30.0, abs(b));
    // the crest rises and dips along the wave's length
    let crest = height * (0.82 + 0.18 * noise(vec2<f32>(b * 0.035, t * 0.2)));
    // and runs a little ahead in places, as a wave breaks unevenly
    let lead = (noise(vec2<f32>(b * 0.05 + 3.0, t * 0.3)) - 0.5) * 6.0;
    let x = a - lead;
    var h = 0.0;
    if (x > 0.0) {
        // the face: nearly sheer at the top, flaring out at its foot
        let f = clamp(x / (6.0 + 22.0 * crash), 0.0, 1.0);
        h = crest * pow(1.0 - f, 2.2);
    } else {
        h = crest * exp(-(x * x) / 420.0) + 4.0 * smoothstep(-80.0, -50.0, x) * (1.0 - smoothstep(-12.0, 0.0, x));
    }
    let ripple = (noise(vec2<f32>(a * 0.15 + t * 0.8, b * 0.08)) - 0.5) * 2.2 + (noise(vec2<f32>(a * 0.5 - t * 1.7, b * 0.4)) - 0.5) * 0.8;
    return max(h * across + ripple * across, 0.0);
}

// The tsunami: a wall of water rolling at the caller, foam along its crest, then breaking.
fn tsunami(v: View, e: Effect, jitter: f32) -> vec4<f32> {
    let age = e.k.y;
    let dur = e.k.z;
    let fade = 1.0 - smoothstep(dur - 2.0, dur, age);
    if (fade <= 0.001) {
        return vec4<f32>(0.0);
    }
    let front = e.p.xyz;
    let height = e.p.w * smoothstep(0.0, 1.5, age);
    let fwd = normalize(vec3<f32>(e.q.x, 0.0, e.q.z));
    let side = vec3<f32>(-fwd.z, 0.0, fwd.x);
    let crash = e.q.w;
    let width = e.c.w;
    let t = frame.a.x;
    // the ray in the wave's frame: a along its way, b across, y up from its base
    let o = -front;
    let oa = dot(o, fwd);
    let ob = dot(o, side);
    let da = dot(v.dir, fwd);
    let db = dot(v.dir, side);
    let lo = vec3<f32>(-80.0, -2.0, -width * 0.5);
    let hi = vec3<f32>(14.0, e.p.w + 4.0, width * 0.5);
    let ro = vec3<f32>(oa, o.y, ob);
    let rd = vec3<f32>(da, v.dir.y, db);
    let inv = 1.0 / select(rd, vec3<f32>(1.0e-6), abs(rd) < vec3<f32>(1.0e-6));
    let t0 = (lo - ro) * inv;
    let t1 = (hi - ro) * inv;
    let tmin = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), max(min(t0.z, t1.z), 0.0));
    let tmax = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    let far = min(tmax, v.dist);
    if (far <= tmin) {
        return vec4<f32>(0.0);
    }
    let steps = 56;
    let ds = (far - tmin) / f32(steps);
    var s = tmin + ds * jitter;
    var hit = -1.0;
    var mist = 0.0;
    for (var i = 0; i < steps; i = i + 1) {
        let p = ro + rd * s;
        let wh = wave_height(p.x, p.z, height, width, crash, t);
        if (p.y < wh) {
            hit = s;
            break;
        }
        // spray torn off the crest
        let gap = p.y - wh;
        if (p.x > -8.0 && p.x < 6.0 && gap < 7.0 && wh > height * 0.5) {
            let puff = noise(vec2<f32>(p.z * 0.3 + t * 1.3, p.y * 0.4 - t * 2.0));
            mist = mist + (1.0 - gap / 7.0) * smoothstep(0.35, 0.8, puff) * ds * 0.09;
        }
        s = s + ds;
    }
    let spray = clamp(mist, 0.0, 0.8) * fade;
    let spray_col = vec3<f32>(0.9, 0.95, 1.0) * daylight();
    if (hit < 0.0) {
        return vec4<f32>(spray_col * spray, spray);
    }
    // close in on the surface
    var a0 = hit - ds;
    var a1 = hit;
    for (var i = 0; i < 5; i = i + 1) {
        let m = (a0 + a1) * 0.5;
        let p = ro + rd * m;
        if (p.y < wave_height(p.x, p.z, height, width, crash, t)) {
            a1 = m;
        } else {
            a0 = m;
        }
    }
    let p = ro + rd * a1;
    let h0 = wave_height(p.x, p.z, height, width, crash, t);
    let ex = wave_height(p.x + 0.6, p.z, height, width, crash, t) - wave_height(p.x - 0.6, p.z, height, width, crash, t);
    let ez = wave_height(p.x, p.z + 0.6, height, width, crash, t) - wave_height(p.x, p.z - 0.6, height, width, crash, t);
    let nl = normalize(vec3<f32>(-ex / 1.2, 1.0, -ez / 1.2));
    let n = normalize(fwd * nl.x + vec3<f32>(0.0, nl.y, 0.0) + side * nl.z);
    let view = -v.dir;
    let fres = pow(1.0 - max(dot(n, view), 0.0), 4.0);
    let sun = normalize(vec3<f32>(0.4, 0.8, 0.3));
    let spec = pow(max(dot(reflect(-sun, n), view), 0.0), 60.0);
    let lit = daylight();
    let deep = vec3<f32>(0.01, 0.1, 0.15);
    var col = mix(deep, e.c.rgb, smoothstep(0.0, height * 0.8, h0)) * (0.5 + 0.5 * max(dot(n, sun), 0.0));
    // light through the thin water under the lip: bright turquoise
    let lip = smoothstep(height * 0.45, height * 0.85, h0) * smoothstep(-3.0, 2.0, p.x);
    col = mix(col, vec3<f32>(0.12, 0.62, 0.58), lip * 0.7);
    col = mix(col, vec3<f32>(0.55, 0.68, 0.78), fres * 0.6);
    // foam: a broken line along the crest, streaks down the face, a churned skirt at the foot
    let churn = fbm(vec2<f32>(p.x * 0.35 - t * 2.2, p.z * 0.25));
    let streaks = fbm(vec2<f32>(p.z * 0.6, h0 * 0.25 + t * 1.5));
    let crest = smoothstep(height * 0.7, height * 0.92, h0) * smoothstep(-6.0, 0.5, p.x) * smoothstep(0.3, 0.55, churn + 0.1);
    let face = smoothstep(0.5, 3.0, p.x) * smoothstep(0.62, 0.8, streaks) * 0.7;
    let skirt = smoothstep(height * 0.25, 0.0, h0) * smoothstep(-2.0, 6.0, p.x) * smoothstep(0.35, 0.6, churn);
    let foam = clamp(crest + face + skirt + crash * smoothstep(0.4, 0.6, churn), 0.0, 1.0);
    col = mix(col, vec3<f32>(0.9, 0.95, 1.0), foam);
    col = (col + vec3<f32>(1.0) * spec * 0.8) * lit;
    let water = vec4<f32>(col * 0.95, 0.95) * fade;
    // the spray in front of the water
    return vec4<f32>(spray_col * spray, spray) + water * (1.0 - spray);
}

// A nuke's cloud: the stem, the rolling cap with fire glowing in it, the dust skirt spreading.
fn mushroom(v: View, e: Effect, jitter: f32) -> vec4<f32> {
    let age = e.k.y;
    let dur = e.k.z;
    let fade = 1.0 - smoothstep(dur - 10.0, dur, age);
    let size = e.p.w;
    let g = e.p.xyz;
    let rise = 1.0 - exp(-age * 0.16);
    let top = g.y + size * (0.4 + 3.4 * rise);
    let cap_r = size * (0.4 + 0.8 * rise);
    let tube = size * (0.32 + 0.28 * rise);
    let skirt = size * (0.9 + age * 0.22);
    let t = frame.a.x;
    let span = cylinder(v.dir, g, max(cap_r + tube + 4.0, skirt), g.y - 2.0, top + tube * 1.4);
    let far = min(span.y, v.dist);
    if (far <= span.x) {
        return vec4<f32>(0.0);
    }
    let heat = exp(-age * 0.2);
    let steps = 48;
    let ds = (far - span.x) / f32(steps);
    var acc = vec4<f32>(0.0);
    var s = span.x + ds * jitter;
    for (var i = 0; i < steps; i = i + 1) {
        let p = v.dir * s;
        let off = p.xz - g.xz;
        let d = length(off);
        let y = p.y;
        let n = fbm(off * 0.06 + vec2<f32>(y * 0.05 - age * 0.15, age * 0.05 + e.k.w));
        let cap = length(vec2<f32>(max(d - cap_r * 0.5, 0.0), (y - top) * 1.4)) - tube;
        let dcap = smoothstep(2.0, -tube * 0.4, cap + (n - 0.5) * tube * 0.9);
        let stem_r = size * 0.17 * (1.0 + 1.6 * smoothstep(g.y + size * 0.7, g.y, y));
        let dstem = step(y, top) * smoothstep(stem_r * 1.25, stem_r * 0.4, d + (n - 0.5) * stem_r);
        let dskirt = smoothstep(size * 0.5 + age * 0.3, 0.0, y - g.y) * smoothstep(skirt, skirt * 0.3, d + (n - 0.5) * size) * 0.6;
        let dens = max(dcap, max(dstem * 0.9, dskirt)) * fade * (0.45 + 0.8 * n);
        if (dens > 0.01) {
            let a = 1.0 - exp(-dens * ds * 0.16);
            let up = clamp((y - (top - tube)) / (tube * 2.0), 0.0, 1.0);
            let smoke = e.c.rgb * (0.55 + 0.6 * up) * (0.8 + 0.4 * n) * daylight();
            let inner = smoothstep(-tube * 0.1, -tube * 0.7, cap) + dstem * 0.5 * smoothstep(top - tube * 2.0, top, y);
            let under = smoothstep(0.5, 0.0, up) * step(top - tube * 1.2, y);
            let fire = mix(vec3<f32>(1.0, 0.28, 0.05), vec3<f32>(1.0, 0.7, 0.3), n) * heat * (inner * 1.6 + under * 0.5);
            acc = over(acc, smoke + fire * 1.5, a);
            if (acc.a > 0.97) {
                break;
            }
        }
        s = s + ds;
    }
    return acc;
}

// A column of smoke, dust or ash, rising and leaning with the wind; glowing at its foot when
// `q.w` says (a volcano's vent, a fault's lava).
fn smoke(v: View, e: Effect, jitter: f32) -> vec4<f32> {
    let age = e.k.y;
    let dur = e.k.z;
    let fade = smoothstep(0.0, 1.0, age) * (1.0 - smoothstep(dur - 4.0, dur, age));
    if (fade <= 0.001) {
        return vec4<f32>(0.0);
    }
    let base = e.p.xyz;
    let r0 = e.p.w;
    let full = e.q.x;
    let rt = e.q.y;
    let speed = e.q.z;
    let glow = e.q.w;
    let top = full * (1.0 - exp(-age * speed / full * 1.5));
    let lean = vec2<f32>(0.18, 0.06);
    let span = cylinder(v.dir, base + vec3<f32>(lean.x * top * 0.5, 0.0, lean.y * top * 0.5), rt * 1.4 + top * 0.12 + 4.0, base.y - 2.0, base.y + top);
    let far = min(span.y, v.dist);
    if (far <= span.x) {
        return vec4<f32>(0.0);
    }
    let steps = 36;
    let ds = (far - span.x) / f32(steps);
    var acc = vec4<f32>(0.0);
    var s = span.x + ds * jitter;
    for (var i = 0; i < steps; i = i + 1) {
        let p = v.dir * s;
        let h = p.y - base.y;
        let hn = clamp(h / max(top, 1.0), 0.0, 1.0);
        let local = p.xz - base.xz - lean * h * hn;
        let d = length(local);
        let r = mix(r0, rt, pow(hn, 1.7));
        let n = fbm(local * (2.0 / max(r, 1.0)) + vec2<f32>(h * 0.07 - age * speed * 0.05, e.k.w));
        let dens = smoothstep(r, r * 0.3, d + (n - 0.5) * r * 0.9) * (0.35 + n) * e.c.w * fade * smoothstep(1.0, 0.8, hn);
        if (dens > 0.01) {
            let a = 1.0 - exp(-dens * ds * 0.2);
            let smoke_col = e.c.rgb * (0.6 + 0.6 * hn) * daylight();
            let fire = vec3<f32>(1.0, 0.38, 0.08) * glow * smoothstep(0.35, 0.0, hn) * (0.5 + n) * 1.6;
            acc = over(acc, smoke_col + fire, a);
            if (acc.a > 0.97) {
                break;
            }
        }
        s = s + ds;
    }
    return acc;
}

// A solar flare: a column of the sun's fire reaching down to the ground, and the ground white-hot.
fn flare(v: View, e: Effect) -> vec3<f32> {
    let age = e.k.y;
    let dur = e.k.z;
    let env = smoothstep(0.0, 0.6, age) * (1.0 - smoothstep(dur - 2.5, dur, age));
    let g = e.p.xyz;
    let r = e.p.w;
    let t = frame.a.x;
    var col = vec3<f32>(0.0);
    let rs = ray_segment(v.dir, g, g + vec3<f32>(0.0, 900.0, 0.0));
    if (rs.y < v.dist + r) {
        let width = r * (0.6 + 2.5 * rs.z);
        let x = rs.x / width;
        let plasma = 0.6 + 0.4 * sin(rs.z * 70.0 - t * 9.0 + noise(vec2<f32>(rs.z * 20.0, t)) * 4.0);
        col = col + (vec3<f32>(1.0, 0.97, 0.88) * exp(-x * x * 3.0) * 2.4 + e.c.rgb * exp(-abs(x) * 1.1) * 0.7 * plasma) * env;
    }
    if (v.has_surface) {
        let d = length(v.surface.xz - g.xz);
        let burn = 0.7 + 0.3 * noise(v.surface.xz * 0.5 + vec2<f32>(t * 2.0, 0.0));
        col = col + (vec3<f32>(1.0, 0.85, 0.6) * exp(-d * d / (r * r * 1.5)) * 2.2 + e.c.rgb * exp(-d / (r * 3.0)) * 0.5) * burn * env;
    }
    return col;
}

// A meteor: a white-hot head and a burning trail behind it, lighting the land as it comes.
fn meteor(v: View, e: Effect) -> vec3<f32> {
    let head = e.p.xyz;
    let size = e.p.w;
    let way = normalize(e.q.xyz);
    let t = frame.a.x;
    let tail = head - way * 180.0;
    var col = vec3<f32>(0.0);
    let rs = ray_segment(v.dir, tail, head);
    if (rs.y < v.dist) {
        let w = size * (0.35 + 2.2 * (1.0 - rs.z));
        let x = rs.x / max(w, 0.1);
        let hot = pow(rs.z, 2.0);
        let tint = mix(vec3<f32>(0.4, 0.3, 0.28), mix(vec3<f32>(1.0, 0.32, 0.06), vec3<f32>(1.0, 0.95, 0.8), hot), smoothstep(0.15, 0.6, rs.z));
        let flick = 0.8 + 0.2 * noise(vec2<f32>(rs.z * 30.0 - t * 20.0, e.k.w));
        col = col + tint * exp(-x * x) * (0.35 + 2.2 * hot) * flick;
    }
    let rp = ray_point(v.dir, head);
    if (rp.y > 0.0 && rp.y < v.dist + size) {
        col = col + vec3<f32>(1.0, 0.97, 0.9) * exp(-pow(rp.x / size, 2.0)) * 3.0 + e.c.rgb * exp(-rp.x / (size * 5.0)) * 0.9;
    }
    if (v.has_surface) {
        let d2 = dot(v.surface - head, v.surface - head);
        col = col + e.c.rgb * 900.0 / (d2 + 900.0) * 0.6;
    }
    return col;
}

// The Gravity Gun's pick: the cube's edges glowing, its block grid faint on its faces.
fn box_glow(v: View, e: Effect) -> vec3<f32> {
    let lo = e.p.xyz;
    let n = e.p.w;
    let hi = lo + vec3<f32>(n);
    let inv = 1.0 / select(v.dir, vec3<f32>(1.0e-6), abs(v.dir) < vec3<f32>(1.0e-6));
    let t0 = lo * inv;
    let t1 = hi * inv;
    let tmin = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let tmax = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    if (tmax < max(tmin, 0.0)) {
        return vec3<f32>(0.0);
    }
    let pulse = 0.75 + 0.25 * sin(frame.a.x * 4.0);
    var col = vec3<f32>(0.0);
    for (var k = 0; k < 2; k = k + 1) {
        let s = select(tmin, tmax, k == 1);
        if (s <= 0.0 || s > v.dist + 0.06) {
            continue;
        }
        let local = (v.dir * s - lo) / n;
        let f = min(local, vec3<f32>(1.0) - local) * n;
        // the distance to the nearest edge is the middle of the three
        let mid = max(min(f.x, f.y), min(max(f.x, f.y), f.z));
        let edge = exp(-pow(mid / 0.05, 2.0)) * 1.3 + exp(-mid / 0.22) * 0.25;
        let cells = abs(fract(local * n + 0.5) - 0.5);
        let grid = exp(-pow(min(min(max(cells.x, cells.y), max(cells.y, cells.z)), max(cells.x, cells.z)) / 0.03, 2.0));
        col = col + e.c.rgb * (edge + grid * 0.12 * step(1.5, n) + 0.05) * pulse;
    }
    return col;
}

// The Gravity Gun's beam: three crackling strands from the hand to the cube, and a ring of force
// round what it holds.
fn tether(v: View, e: Effect) -> vec3<f32> {
    let a = e.p.xyz;
    let b = e.q.xyz;
    let size = e.q.w;
    let t = frame.a.x;
    let line = b - a;
    let len = length(line);
    let along = line / max(len, 1.0e-3);
    let p1 = normalize(cross(along, vec3<f32>(0.0, 1.0, 0.0)) + vec3<f32>(1.0e-4, 0.0, 0.0));
    let p2 = cross(p1, along);
    let flick = 0.8 + 0.2 * noise(vec2<f32>(t * 40.0, 1.7));
    var col = vec3<f32>(0.0);
    for (var k = 0; k < 3; k = k + 1) {
        let fk = f32(k);
        // each strand a wave along the beam, in four pieces
        for (var j = 0; j < 4; j = j + 1) {
            let u0 = f32(j) / 4.0;
            let u1 = f32(j + 1) / 4.0;
            let w0 = sin(u0 * 3.14159) * sin(u0 * 9.0 - t * 14.0 + fk * 2.1);
            let w1 = sin(u1 * 3.14159) * sin(u1 * 9.0 - t * 14.0 + fk * 2.1);
            let twist0 = p1 * cos(fk * 2.09 + t * 3.0) + p2 * sin(fk * 2.09 + t * 3.0);
            let amp = len * 0.025 * select(1.0, 0.0, k == 0);
            let from_p = a + line * u0 + twist0 * w0 * amp;
            let to_p = a + line * u1 + twist0 * w1 * amp;
            let rs = ray_segment(v.dir, from_p, to_p);
            if (rs.y < v.dist + 0.2) {
                let near = max(rs.y, 0.05);
                let x = (rs.x / near) / min(0.03 / near, 0.0035);
                let tint = select(vec3<f32>(1.0, 0.55, 0.15), vec3<f32>(1.0, 0.9, 0.6), k == 0);
                col = col + tint * (exp(-x * x) * 0.9 + exp(-abs(x) / 3.0) * 0.1) * flick;
            }
        }
    }
    let rp = ray_point(v.dir, b);
    if (rp.y > 0.0) {
        let r = 0.5 + size * 0.5;
        let ring = exp(-pow((rp.x - r) / (0.05 * r + 0.04), 2.0)) * 0.3 + exp(-rp.x / (r * 0.6)) * 0.05;
        col = col + e.c.rgb * ring * flick;
    }
    return col;
}

// Where the Storm Scepter points: a turning ring the size of what it will strike, and a pillar
// of light so it shows from far away.
fn marker(v: View, e: Effect) -> vec3<f32> {
    let at = e.p.xyz;
    let r = e.p.w;
    let t = frame.a.x;
    var col = vec3<f32>(0.0);
    if (v.has_surface) {
        let off = v.surface - at;
        let d = length(off.xz);
        if (abs(off.y) < r * 0.6 + 6.0) {
            let ang = atan2(off.z, off.x);
            let dashes = step(0.45, fract(ang / 6.2831853 * 24.0 + t * 0.25));
            let w = 0.35 + r * 0.012;
            let ring = exp(-pow((d - r) / w, 2.0)) * (0.5 + 0.5 * dashes);
            let inner = exp(-pow((d - 1.2) / 0.25, 2.0));
            col = col + e.c.rgb * (ring + inner * 0.8 + exp(-d / (r * 0.35)) * 0.05) * (0.8 + 0.2 * sin(t * 5.0));
        }
    }
    let rs = ray_segment(v.dir, at, at + vec3<f32>(0.0, 60.0, 0.0));
    if (rs.y < v.dist + 0.5) {
        let x = rs.x / (0.12 + rs.y * 0.002);
        col = col + e.c.rgb * exp(-x * x) * 0.9 * (1.0 - rs.z);
    }
    return col;
}

// ---- the jets ---------------------------------------------------------------------------------------

// An engine's flame: a cone of fire behind the nozzle; with the afterburner (c.w) long and blue at
// its root, with bright shock diamonds down its length.
fn flame(v: View, e: Effect) -> vec3<f32> {
    let origin = e.p.xyz;
    let r = e.p.w;
    let dir = normalize(e.q.xyz);
    let len = max(e.q.w, 0.1);
    let ab = e.c.w;
    let rs = ray_segment(v.dir, origin, origin + dir * len);
    if (rs.y > v.dist + r * 2.0) {
        return vec3<f32>(0.0);
    }
    let t = frame.a.x;
    let width = r * (1.0 - 0.8 * rs.z) + 0.03;
    let x = rs.x / width;
    let flick = 0.8 + 0.2 * noise(vec2<f32>(rs.z * 9.0 - t * 45.0, e.k.w + rs.x));
    let core = exp(-x * x * 2.0);
    let fade = 1.0 - rs.z;
    let dry = e.c.rgb * core * fade * 0.9;
    let diamonds = pow(0.5 + 0.5 * cos(rs.z * len / max(r * 1.7, 0.1) * 6.2831853), 3.0);
    let root = mix(vec3<f32>(0.55, 0.7, 1.0), vec3<f32>(1.0, 0.5, 0.18), smoothstep(0.05, 0.6, rs.z));
    let burner = root * core * fade * (0.8 + 2.2 * diamonds * smoothstep(0.03, 0.15, rs.z));
    let halo = exp(-abs(x) * 0.7) * 0.08 * fade;
    return (mix(dry, burner * 1.5, ab) + e.c.rgb * halo) * flick;
}

// A vapour cone round a jet passing the speed of sound.
fn vapor(v: View, e: Effect) -> vec3<f32> {
    let c = e.p.xyz;
    let r = e.p.w;
    let axis = normalize(e.q.xyz);
    let rs = ray_segment(v.dir, c + axis * r * 0.4, c - axis * r * 1.4);
    if (rs.y > v.dist) {
        return vec3<f32>(0.0);
    }
    let cone = r * (0.25 + 0.85 * rs.z);
    let shell = exp(-pow((rs.x - cone) / (0.3 * cone + 0.2), 2.0));
    let fill = smoothstep(cone, cone * 0.4, rs.x) * 0.3;
    let n = 0.7 + 0.3 * noise(vec2<f32>(rs.z * 12.0, frame.a.x * 20.0));
    return vec3<f32>(1.0) * (shell + fill) * (1.0 - rs.z) * e.q.w * 0.35 * n;
}

// A cannon round's tracer: a short bright streak.
fn tracer(v: View, e: Effect) -> vec3<f32> {
    let head = e.p.xyz;
    let dir = normalize(e.q.xyz);
    let rs = ray_segment(v.dir, head - dir * e.q.w, head);
    if (rs.y > v.dist + 0.5) {
        return vec3<f32>(0.0);
    }
    let near = max(rs.y, 0.1);
    let x = rs.x / max(0.05, near * 0.0016);
    return e.c.rgb * exp(-x * x) * (0.4 + 1.8 * rs.z * rs.z) * 1.8;
}

// The chase camera's markers: a ring where the nose points, a diamond on a lock.
fn reticle(v: View, e: Effect) -> vec3<f32> {
    let pd = normalize(e.p.xyz);
    let facing = dot(v.dir, pd);
    if (facing < 0.5) {
        return vec3<f32>(0.0);
    }
    let right = normalize(cross(pd, vec3<f32>(0.0, 1.0, 0.0)) + vec3<f32>(1.0e-5, 0.0, 0.0));
    let upv = cross(right, pd);
    let local = vec2<f32>(dot(v.dir, right), dot(v.dir, upv)) / facing;
    let size = e.p.w;
    let w = 0.0012;
    var d = 0.0;
    if (e.q.w > 0.5) {
        d = abs(abs(local.x) + abs(local.y) - size);
    } else {
        d = min(abs(length(local) - size), length(local) * 3.0);
    }
    let blink = select(1.0, step(0.5, fract(frame.a.x * 6.0)), e.c.w < 0.9 && e.q.w > 0.5);
    return e.c.rgb * exp(-pow(d / w, 2.0)) * 1.4 * blink;
}

// Rain: streaks falling down the screen in three layers.
fn rain_streaks(ndc: vec2<f32>, t: f32) -> f32 {
    var s = 0.0;
    for (var l = 0; l < 3; l = l + 1) {
        let fl = f32(l);
        let scale = 70.0 + fl * 55.0;
        let p = vec2<f32>(ndc.x * scale + ndc.y * scale * 0.12, ndc.y * (4.0 + fl * 2.0) + t * (5.0 + fl * 2.5));
        let id = floor(p.x);
        let h = hash21(vec2<f32>(id, fl));
        let y = fract(p.y + h * 7.3);
        let drop = smoothstep(0.0, 0.03, y) * smoothstep(0.25, 0.0, y);
        let lane = pow(1.0 - abs(fract(p.x) - 0.5) * 2.0, 6.0);
        s = s + drop * lane * step(0.55, h) * (0.5 - fl * 0.12);
    }
    return s;
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
    for (var i = 0; i < 48; i = i + 1) {
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
        } else if (kind == ORB) {
            col = col + orb(v, e);
        } else if (kind == FLARE) {
            col = col + flare(v, e);
        } else if (kind == METEOR) {
            col = col + meteor(v, e);
        } else if (kind == BOX) {
            col = col + box_glow(v, e);
        } else if (kind == TETHER) {
            col = col + tether(v, e);
        } else if (kind == MARKER) {
            col = col + marker(v, e);
        } else if (kind == FLAME) {
            col = col + flame(v, e);
        } else if (kind == VAPOR) {
            col = col + vapor(v, e);
        } else if (kind == TRACER) {
            col = col + tracer(v, e);
        } else if (kind == RETICLE) {
            col = col + reticle(v, e);
        }
    }
    // speed lines streaming past the view's edges
    if (frame.c.w > 0.0) {
        let ang = atan2(in.ndc.y, in.ndc.x);
        let line = step(0.93, hash21(vec2<f32>(floor(ang * 140.0), floor(frame.a.x * 24.0))));
        let edge = smoothstep(0.55, 1.05, length(in.ndc * vec2<f32>(0.6, 1.0)));
        col = col + vec3<f32>(0.85, 0.95, 1.0) * line * edge * 0.3 * frame.c.w;
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
    for (var i = 0; i < 48; i = i + 1) {
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
        } else if (kind == HAZE) {
            // the air shimmers in a jet's exhaust
            let dir_e = normalize(e.q.xyz);
            let rs = ray_segment(v.dir, e.p.xyz, e.p.xyz + dir_e * e.q.w);
            let width = e.p.w * (1.0 + 2.0 * rs.z);
            if (rs.y < v.dist && rs.x < width) {
                let k = (1.0 - rs.x / width) * (1.0 - rs.z) * e.c.w;
                let wob = vec2<f32>(noise(vec2<f32>(rs.z * 20.0 - frame.a.x * 30.0, rs.x * 9.0)), noise(vec2<f32>(rs.x * 11.0, rs.z * 17.0 - frame.a.x * 26.0))) - 0.5;
                ripple = ripple + wob * k * 0.012;
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
    let t = frame.a.x;
    let heat = frame.d.z;
    if (heat > 0.0) {
        // the air shimmers over the burning land
        let wobble = vec2<f32>(noise(v.uv * vec2<f32>(30.0, 50.0) + vec2<f32>(0.0, t * 4.0)), noise(v.uv * vec2<f32>(30.0, 50.0) + vec2<f32>(5.2, t * 4.3))) - 0.5;
        uv = uv + wobble * 0.006 * heat;
    }
    let sampled = textureSampleLevel(scene, scene_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    var col = sampled.rgb;
    // a storm darkens the land and greys the sky; fire and heat cast it red
    let gloom = frame.d.y;
    if (!v.has_surface) {
        col = mix(col, vec3<f32>(0.33, 0.35, 0.38) * mix(1.0, 0.16, frame.a.y), gloom * 0.75);
    }
    col = col * (1.0 - 0.5 * gloom);
    col = mix(col, col * vec3<f32>(1.35, 0.82, 0.55), frame.d.w * 0.5);
    col = mix(col, vec3<f32>(0.78, 0.9, 1.0) * (0.35 + 0.65 * max(max(col.r, col.g), col.b) * 1.6), ground_frost);
    if (frost_tint > 0.0) {
        let edge = smoothstep(0.8, 1.35, length(in.ndc * vec2<f32>(1.0, 0.85)));
        let crystals = fbm(v.uv * vec2<f32>(40.0, 24.0)) * 0.6 + fbm(v.uv * vec2<f32>(140.0, 90.0)) * 0.4;
        let ice = smoothstep(0.55, 0.85, crystals * 0.8 + edge * 0.5) * edge;
        col = mix(col, vec3<f32>(0.8, 0.93, 1.0), ice * frost_tint * 0.6);
    }
    col = col * (1.0 - dark);
    // the storms' clouds, funnels, waves and smoke, over the land behind them
    let jitter = hash21(floor(in.pos.xy) + vec2<f32>(fract(t * 7.0) * 13.0, 0.0));
    for (var i = 0; i < 48; i = i + 1) {
        if (i >= count) {
            break;
        }
        let e = frame.effects[i];
        let kind = i32(e.k.x + 0.5);
        var vol = vec4<f32>(0.0);
        if (kind == TORNADO) {
            vol = tornado(v, e, jitter);
        } else if (kind == HURRICANE) {
            vol = storm_deck(v, e, jitter);
        } else if (kind == TSUNAMI) {
            vol = tsunami(v, e, jitter);
        } else if (kind == MUSHROOM) {
            vol = mushroom(v, e, jitter);
        } else if (kind == SMOKE) {
            vol = smoke(v, e, jitter);
        }
        col = col * (1.0 - vol.a) + vol.rgb;
    }
    // vapour and smoke trails
    let trail_count = i32(frame.e.x);
    for (var i = 0; i < 95; i = i + 1) {
        if (i + 1 >= trail_count) {
            break;
        }
        let a = frame.trails[i];
        let b = frame.trails[i + 1];
        if (a.w < 0.0 || b.w < 0.0) {
            continue;
        }
        let kind_t = floor(a.w * 0.5);
        let fresh = a.w - kind_t * 2.0;
        let rs = ray_segment(v.dir, a.xyz, b.xyz);
        if (rs.y > v.dist || rs.y <= 0.0) {
            continue;
        }
        var width = 0.22;
        var strength = 0.45;
        var tint = vec3<f32>(1.0);
        if (kind_t > 1.5) {
            width = 0.6;
            strength = 0.7;
            tint = vec3<f32>(0.62, 0.6, 0.58);
        } else if (kind_t > 0.5) {
            width = 0.9;
            strength = 0.55;
        }
        width = width * (1.0 + (1.0 - fresh) * 3.0);
        let x = rs.x / width;
        let alpha = exp(-x * x) * fresh * strength;
        col = mix(col, tint * daylight(), clamp(alpha, 0.0, 0.85));
    }
    let rain = frame.d.x;
    if (rain > 0.0) {
        col = mix(col, vec3<f32>(0.72, 0.78, 0.84) * daylight(), clamp(rain_streaks(in.ndc, t) * rain * 0.45, 0.0, 1.0));
    }
    return vec4<f32>(col, sampled.a);
}
"#;
