//! The WGSL that draws the jets: lit, painted models with moving parts, glass, glowing nozzles
//! and lights, the cockpit's screens, and the heads-up display drawn on its glass plate. They draw
//! into a layer of their own (antialiased), hidden behind the world by its depth, which a last
//! pass lays over the picture.

pub const WGSL: &str = r#"
struct Frame {
    view_proj: mat4x4<f32>,
    // sun direction, night 0..1
    sun: vec4<f32>,
    // picture width, height, depth width, height
    screen: vec4<f32>,
    // seconds, heads-up display on, storm darkness, spare
    misc: vec4<f32>,
    // the flown jet's axes (left, up, forward) in the world, for the instruments
    left: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // its velocity's direction, and speed (blocks a second)
    vel: vec4<f32>,
    // where the pilot aims (mouse aim), and the mode (0 mouse aim, 1 direct)
    aim: vec4<f32>,
    // the locked target's direction, and the lock (0 none, 0.5 locking, 1 locked)
    lock: vec4<f32>,
    // altitude, heading (degrees), throttle 0..1, afterburner 0..1
    d1: vec4<f32>,
    // g, Mach, angle of attack (degrees), stall warning
    d2: vec4<f32>,
    // rounds, missiles, bombs, gear 0..1
    d3: vec4<f32>,
    // where a bomb would land (direction), and whether it shows
    d4: vec4<f32>,
    // roll and pitch (degrees), radar altitude, pull-up warning
    d5: vec4<f32>,
    // assists on, stick pitch, stick roll, spare
    d6: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    parts: array<vec4<f32>, 8>,
    // engine glow, pilot hidden, test against the world's depth, pass (0 solid, 1 glass)
    params: vec4<f32>,
    // the cockpit's opening in the hull: half width, sill height, back, front (body frame)
    cut: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<uniform> object: Object;
@group(0) @binding(2) var world_depth: texture_2d<f32>;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) material: f32,
    @location(4) part: f32,
    @location(5) pivot: vec3<f32>,
    @location(6) axis: vec3<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) @interpolate(flat) material: f32,
    @location(4) local: vec3<f32>,
};

const PAINT: i32 = 0;
const METAL: i32 = 1;
const GLASS: i32 = 2;
const NAV_RED: i32 = 3;
const NAV_GREEN: i32 = 4;
const STROBE: i32 = 5;
const HOT: i32 = 6;
const RUBBER: i32 = 7;
const INTERIOR: i32 = 8;
const HUD: i32 = 9;
const SCREEN: i32 = 10;
const BUTTONS: i32 = 11;
const INTAKE: i32 = 12;
const PI: f32 = 3.14159265;

fn part_value(i: i32) -> f32 {
    let v = object.parts[i / 4];
    let k = i % 4;
    if (k == 0) {
        return v.x;
    } else if (k == 1) {
        return v.y;
    } else if (k == 2) {
        return v.z;
    }
    return v.w;
}

// Rodrigues: `p` turned `a` radians about the unit `axis`.
fn turn(p: vec3<f32>, axis: vec3<f32>, a: f32) -> vec3<f32> {
    let c = cos(a);
    let s = sin(a);
    return p * c + cross(axis, p) * s + axis * dot(axis, p) * (1.0 - c);
}

@vertex
fn vs_jet(v: VIn) -> VOut {
    var p = v.pos;
    var n = v.normal;
    let id = i32(v.part + 0.5);
    if (id > 0) {
        let a = part_value(id);
        if (id == 15) {
            // the pilot, hidden from their own eyes
            if (object.params.y > 0.5) {
                p = v.pivot;
            }
        } else if (id >= 16) {
            // a store already fired
            if (a > 0.5) {
                p = v.pivot;
            }
        } else if (abs(a) > 1.0e-5 && dot(v.axis, v.axis) > 0.5) {
            p = v.pivot + turn(p - v.pivot, v.axis, a);
            n = turn(n, v.axis, a);
        }
    }
    var out: VOut;
    let world = object.model * vec4<f32>(p, 1.0);
    out.world = world.xyz;
    out.clip = frame.view_proj * world;
    out.normal = (object.model * vec4<f32>(n, 0.0)).xyz;
    out.color = v.color;
    out.material = v.material;
    out.local = p;
    return out;
}

// ---- light --------------------------------------------------------------------------------------

fn sky_color(dir_y: f32) -> vec3<f32> {
    let night = frame.sun.w;
    let day = mix(vec3<f32>(0.28, 0.26, 0.22), mix(vec3<f32>(0.62, 0.72, 0.85), vec3<f32>(0.35, 0.52, 0.85), clamp(dir_y, 0.0, 1.0)), smoothstep(-0.2, 0.05, dir_y));
    let dark = mix(vec3<f32>(0.01, 0.012, 0.02), vec3<f32>(0.03, 0.04, 0.08), clamp(dir_y, 0.0, 1.0));
    return mix(day, dark, night) * (1.0 - 0.45 * frame.misc.z);
}

fn shade(base: vec3<f32>, n: vec3<f32>, view: vec3<f32>, rough: f32, metal: f32) -> vec3<f32> {
    let night = frame.sun.w;
    let sun = normalize(frame.sun.xyz);
    let l = select(sun, -sun, sun.y < 0.0);
    let storm = 1.0 - 0.6 * frame.misc.z;
    let light = mix(vec3<f32>(1.0, 0.95, 0.86) * 2.4, vec3<f32>(0.32, 0.38, 0.55) * 0.35, night) * storm;
    let ndl = max(dot(n, l), 0.0);
    let ambient = mix(sky_color(-1.0) * 0.6, sky_color(0.6), n.y * 0.5 + 0.5) * 0.9;
    let h = normalize(l + view);
    let power = mix(260.0, 10.0, rough);
    let spec = pow(max(dot(n, h), 0.0), power) * (power + 8.0) / (8.0 * PI) * ndl;
    let nv = max(dot(n, view), 0.0);
    let fres = 0.04 + 0.96 * pow(1.0 - nv, 5.0);
    let f0 = mix(vec3<f32>(0.04), base, metal);
    let r = reflect(-view, n);
    let env = sky_color(r.y);
    let diffuse = base * (ambient + light * ndl) * (1.0 - metal * 0.6);
    let specular = (light * spec + env * (1.0 - rough) * 0.8) * mix(f0, vec3<f32>(1.0), fres);
    return diffuse + specular;
}

fn noise1(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// ---- seven-segment text, for the displays ---------------------------------------------------------

// Coverage of the code's segments at `p` in a cell 0.6 wide and 1 tall; `w` half line width,
// `aa` the edge's softness.
fn seg7(p: vec2<f32>, code: u32, w: f32, aa: f32) -> f32 {
    var d = 1.0e3;
    let x0 = 0.05;
    let x1 = 0.55;
    // a top, b top right, c bottom right, d bottom, e bottom left, f top left, g middle
    if ((code & 1u) != 0u) { d = min(d, dist_seg(p, vec2<f32>(x0, 1.0), vec2<f32>(x1, 1.0))); }
    if ((code & 2u) != 0u) { d = min(d, dist_seg(p, vec2<f32>(x1, 1.0), vec2<f32>(x1, 0.5))); }
    if ((code & 4u) != 0u) { d = min(d, dist_seg(p, vec2<f32>(x1, 0.5), vec2<f32>(x1, 0.0))); }
    if ((code & 8u) != 0u) { d = min(d, dist_seg(p, vec2<f32>(x0, 0.0), vec2<f32>(x1, 0.0))); }
    if ((code & 16u) != 0u) { d = min(d, dist_seg(p, vec2<f32>(x0, 0.5), vec2<f32>(x0, 0.0))); }
    if ((code & 32u) != 0u) { d = min(d, dist_seg(p, vec2<f32>(x0, 1.0), vec2<f32>(x0, 0.5))); }
    if ((code & 64u) != 0u) { d = min(d, dist_seg(p, vec2<f32>(x0, 0.5), vec2<f32>(x1, 0.5))); }
    return 1.0 - smoothstep(w - aa, w + aa, d);
}

fn dist_seg(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / dot(ab, ab), 0.0, 1.0);
    return length(p - a - ab * t);
}

fn digit_code(d: i32) -> u32 {
    switch (d) {
        case 0: { return 0x3Fu; }
        case 1: { return 0x06u; }
        case 2: { return 0x5Bu; }
        case 3: { return 0x4Fu; }
        case 4: { return 0x66u; }
        case 5: { return 0x6Du; }
        case 6: { return 0x7Du; }
        case 7: { return 0x07u; }
        case 8: { return 0x7Fu; }
        default: { return 0x6Fu; }
    }
}

// The whole number `value`, its right edge at `at`, digits `h` tall, in the same units as `p`.
fn number(p: vec2<f32>, at: vec2<f32>, h: f32, value: i32, aa: f32) -> f32 {
    let cell = h * 0.8;
    var v = abs(value);
    var cover = 0.0;
    for (var k = 0; k < 5; k = k + 1) {
        let origin = at - vec2<f32>(cell * f32(k + 1), 0.0);
        let q = (p - origin) / h;
        if (q.x > -0.1 && q.x < 0.7 && q.y > -0.15 && q.y < 1.15) {
            cover = max(cover, seg7(q, digit_code(v % 10), 0.075, aa / h));
        }
        v = v / 10;
        if (v == 0) {
            if (value < 0) {
                let mq = (p - (origin - vec2<f32>(cell, 0.0))) / h;
                if (mq.x > -0.1 && mq.x < 0.7) {
                    cover = max(cover, seg7(mq, 0x40u, 0.075, aa / h));
                }
            }
            break;
        }
    }
    return cover;
}

// Up to four letters (seven-segment codes), starting at `at`.
fn word(p: vec2<f32>, at: vec2<f32>, h: f32, codes: vec4<u32>, aa: f32) -> f32 {
    var cover = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let q = (p - at - vec2<f32>(h * 0.8 * f32(k), 0.0)) / h;
        if (q.x > -0.1 && q.x < 0.7 && q.y > -0.15 && q.y < 1.15) {
            cover = max(cover, seg7(q, codes[k], 0.075, aa / h));
        }
    }
    return cover;
}

fn line(d: f32, w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(w - aa, w + aa, d);
}

// ---- the heads-up display ------------------------------------------------------------------------

// A direction as the HUD sees it: degrees right and up of the jet's nose.
fn hud_angles(d: vec3<f32>) -> vec2<f32> {
    let x = dot(d, -frame.left.xyz);
    let y = dot(d, frame.up.xyz);
    let z = max(dot(d, frame.fwd.xyz), 1.0e-3);
    return vec2<f32>(atan2(x, z), atan2(y, z)) * (180.0 / PI);
}

fn hud(dir: vec3<f32>, a: vec2<f32>, aa: f32) -> f32 {
    let lw = 0.07;
    var s = 0.0;
    let t = frame.misc.x;
    // the pitch ladder: lines of the world's own elevation, crossing the view as the jet rolls
    let elev = asin(clamp(dir.y, -1.0, 1.0)) * 180.0 / PI;
    let flat_fwd = normalize(vec3<f32>(frame.fwd.x, 0.0, frame.fwd.z) + vec3<f32>(1.0e-5, 0.0, 0.0));
    let flat_right = vec3<f32>(-flat_fwd.z, 0.0, flat_fwd.x);
    let fd = normalize(vec3<f32>(dir.x, 0.0, dir.z) + vec3<f32>(1.0e-5, 0.0, 0.0));
    let across = atan2(dot(fd, flat_right), dot(fd, flat_fwd)) * 180.0 / PI;
    let rung = round(elev / 5.0);
    let off = abs(elev - rung * 5.0);
    let reach = abs(across);
    if (rung == 0.0) {
        // the horizon, long, with a gap in the middle
        s = max(s, line(off, lw, aa) * step(2.0, reach) * step(reach, 11.0));
    } else {
        let dashed = select(1.0, step(0.5, fract(reach * 1.5)), rung < 0.0);
        s = max(s, line(off, lw, aa) * step(1.6, reach) * step(reach, 5.0) * dashed);
        // the rung's ends turn towards the horizon
        let tick = abs(reach - 5.0);
        let toward = (elev - rung * 5.0) * sign(rung);
        s = max(s, line(tick, lw, aa) * step(-0.8, -toward) * step(toward, 0.0));
        // and carry its number
        let label = vec2<f32>(reach - 5.6, elev - rung * 5.0 + 0.4);
        s = max(s, number(label, vec2<f32>(1.8, 0.0), 0.8, i32(abs(rung) * 5.0), aa));
    }
    // the nose: a W
    let w = vec2<f32>(abs(a.x), a.y);
    s = max(s, line(dist_seg(w, vec2<f32>(0.0, 0.0), vec2<f32>(0.5, -0.5)), lw, aa));
    s = max(s, line(dist_seg(w, vec2<f32>(0.5, -0.5), vec2<f32>(1.0, 0.0)), lw, aa));
    s = max(s, line(dist_seg(w, vec2<f32>(1.0, 0.0), vec2<f32>(1.7, 0.0)), lw, aa));
    // the flight path marker: where the jet is really going
    let fp = a - hud_angles(normalize(frame.vel.xyz));
    let fr = length(fp);
    s = max(s, line(abs(fr - 0.55), lw, aa));
    s = max(s, line(dist_seg(vec2<f32>(abs(fp.x), fp.y), vec2<f32>(0.55, 0.0), vec2<f32>(1.4, 0.0)), lw, aa));
    s = max(s, line(dist_seg(fp, vec2<f32>(0.0, 0.55), vec2<f32>(0.0, 1.0)), lw, aa));
    // where the pilot aims, in mouse aim: a small diamond
    if (frame.aim.w < 0.5) {
        let am = a - hud_angles(normalize(frame.aim.xyz));
        s = max(s, line(abs(abs(am.x) + abs(am.y) - 0.6), lw * 0.8, aa));
    }
    // in direct mode, where the stick is: a dot in a small square under the nose
    if (frame.aim.w > 0.5) {
        let box_c = vec2<f32>(0.0, -6.5);
        let bd = abs(a - box_c) - vec2<f32>(1.2, 1.2);
        s = max(s, line(abs(max(bd.x, bd.y)), lw * 0.7, aa));
        let stick = box_c + vec2<f32>(frame.d6.z, frame.d6.y) * 1.2;
        s = max(s, line(length(a - stick), 0.22, aa));
    }
    // the lock: a diamond on the target, a box round it once locked
    if (frame.lock.w > 0.1) {
        let tg = a - hud_angles(normalize(frame.lock.xyz));
        let blink = select(1.0, step(0.5, fract(t * 6.0)), frame.lock.w < 0.9);
        s = max(s, line(abs(abs(tg.x) + abs(tg.y) - 1.0), lw, aa) * blink);
        if (frame.lock.w > 0.9) {
            let box = max(abs(tg.x), abs(tg.y));
            s = max(s, line(abs(box - 1.5), lw, aa));
            s = max(s, word(a, vec2<f32>(-1.6, -3.3), 0.9, vec4<u32>(0x38u, 0x5Cu, 0x39u, 0u), aa));
        }
    }
    // where a bomb would fall
    if (frame.d4.w > 0.5) {
        let bp = a - hud_angles(normalize(frame.d4.xyz));
        s = max(s, line(abs(length(bp) - 0.8), lw, aa));
        s = max(s, line(length(bp), 0.18, aa));
    }
    // the heading tape across the top
    let heading = frame.d1.y + a.x;
    if (a.y > 6.2 && a.y < 8.6 && abs(a.x) < 8.0) {
        let mark = abs(fract(heading / 5.0 + 0.5) - 0.5) * 5.0;
        let big = abs(fract(heading / 10.0 + 0.5) - 0.5) * 10.0;
        s = max(s, line(mark, lw, aa) * step(a.y, 6.9));
        s = max(s, line(big, lw, aa) * step(a.y, 7.3));
        let ten = round(heading / 10.0);
        let label = vec2<f32>(heading - ten * 10.0, a.y - 7.5);
        var shown = i32(ten) % 36;
        if (shown < 0) {
            shown = shown + 36;
        }
        s = max(s, number(label, vec2<f32>(0.75, 0.0), 0.8, shown, aa));
    }
    s = max(s, line(dist_seg(a, vec2<f32>(-0.3, 5.7), vec2<f32>(0.0, 6.1)), lw, aa));
    s = max(s, line(dist_seg(a, vec2<f32>(0.3, 5.7), vec2<f32>(0.0, 6.1)), lw, aa));
    // the speed box on the left, the height on the right
    let sb = vec2<f32>(-10.0, -1.0);
    let sd = abs(a - sb - vec2<f32>(0.0, 0.5)) - vec2<f32>(2.0, 0.85);
    s = max(s, line(abs(max(sd.x, sd.y)), lw, aa));
    s = max(s, number(a, sb + vec2<f32>(1.6, 0.0), 1.0, i32(frame.vel.w), aa));
    let ab = vec2<f32>(10.0, -1.0);
    let ad = abs(a - ab - vec2<f32>(0.0, 0.5)) - vec2<f32>(2.0, 0.85);
    s = max(s, line(abs(max(ad.x, ad.y)), lw, aa));
    s = max(s, number(a, ab + vec2<f32>(1.8, 0.0), 1.0, i32(frame.d1.x), aa));
    // under them: g, Mach, angle of attack, throttle; and the weapons
    let lh = 0.8;
    s = max(s, word(a, vec2<f32>(-12.0, -4.0), lh, vec4<u32>(0x3Du, 0u, 0u, 0u), aa));
    s = max(s, number(a, vec2<f32>(-8.3, -4.0), lh, i32(frame.d2.x * 10.0), aa));
    s = max(s, line(length(a - vec2<f32>(-9.15, -3.95)), 0.08, aa));
    s = max(s, word(a, vec2<f32>(-12.0, -5.3), lh, vec4<u32>(0x77u, 0u, 0u, 0u), aa));
    s = max(s, number(a, vec2<f32>(-8.3, -5.3), lh, i32(frame.d2.z), aa));
    s = max(s, word(a, vec2<f32>(9.0, -4.0), lh, vec4<u32>(0x37u, 0u, 0u, 0u), aa));
    s = max(s, number(a, vec2<f32>(12.8, -4.0), lh, i32(frame.d2.y * 100.0), aa));
    s = max(s, line(length(a - vec2<f32>(11.15, -3.95)), 0.08, aa));
    s = max(s, word(a, vec2<f32>(9.0, -5.3), lh, vec4<u32>(0x78u, 0u, 0u, 0u), aa));
    s = max(s, number(a, vec2<f32>(12.8, -5.3), lh, i32(frame.d1.z * 100.0 + frame.d1.w * 10.0), aa));
    s = max(s, word(a, vec2<f32>(-5.5, -9.2), lh, vec4<u32>(0x3Du, 0u, 0u, 0u), aa));
    s = max(s, number(a, vec2<f32>(-2.0, -9.2), lh, i32(frame.d3.x), aa));
    s = max(s, word(a, vec2<f32>(-1.2, -9.2), lh, vec4<u32>(0x50u, 0u, 0u, 0u), aa));
    s = max(s, number(a, vec2<f32>(1.6, -9.2), lh, i32(frame.d3.y), aa));
    s = max(s, word(a, vec2<f32>(2.4, -9.2), lh, vec4<u32>(0x7Cu, 0u, 0u, 0u), aa));
    s = max(s, number(a, vec2<f32>(5.2, -9.2), lh, i32(frame.d3.z), aa));
    // warnings, flashing
    let flash = step(0.5, fract(t * 2.5));
    if (frame.d2.w > 0.5) {
        s = max(s, word(a, vec2<f32>(-2.0, -7.0), 1.0, vec4<u32>(0x6Du, 0x78u, 0x77u, 0x38u), aa) * flash);
    }
    if (frame.d5.w > 0.5) {
        s = max(s, word(a, vec2<f32>(-3.4, 2.0), 1.2, vec4<u32>(0x73u, 0x3Eu, 0x38u, 0x38u), aa) * flash);
    }
    return s;
}

// ---- the cockpit's screens ------------------------------------------------------------------------

fn screen(id: i32, uv: vec2<f32>, aa: f32) -> vec3<f32> {
    let t = frame.misc.x;
    // (v runs down the screen)
    let p = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    var col = vec3<f32>(0.0);
    if (id == 0) {
        // attitude: sky over ground, turned by the roll, moved by the pitch
        let roll = frame.d5.x * PI / 180.0;
        let c = cos(roll);
        let sn = sin(roll);
        let q = vec2<f32>(p.x * c - p.y * sn, p.x * sn + p.y * c);
        let horizon = -frame.d5.y / 45.0;
        let sky = q.y > horizon;
        col = select(vec3<f32>(0.38, 0.22, 0.08), vec3<f32>(0.08, 0.32, 0.75), sky);
        for (var k = -3; k <= 3; k = k + 1) {
            if (k == 0) {
                continue;
            }
            let y = horizon + f32(k) * 10.0 / 45.0;
            let len = select(0.15, 0.3, k % 2 == 0);
            col = mix(col, vec3<f32>(1.0), line(abs(q.y - y), 0.008, aa) * step(abs(q.x), len));
        }
        col = mix(col, vec3<f32>(1.0), line(abs(q.y - horizon), 0.012, aa));
        // the jet's own symbol, fixed
        let w = vec2<f32>(abs(p.x), p.y);
        let sym = min(dist_seg(w, vec2<f32>(0.08, 0.0), vec2<f32>(0.4, 0.0)), dist_seg(w, vec2<f32>(0.0, 0.0), vec2<f32>(0.08, -0.06)));
        col = mix(col, vec3<f32>(1.0, 0.8, 0.1), line(sym, 0.02, aa));
        col = mix(col, vec3<f32>(1.0, 0.8, 0.1), line(length(p), 0.025, aa));
        // heading along the top
        col = mix(col, vec3<f32>(1.0), number(p, vec2<f32>(0.18, 0.72), 0.2, i32(frame.d1.y), aa * 2.0));
    } else if (id == 1) {
        // radar: rings, a turning sweep, the target
        col = vec3<f32>(0.0, 0.05, 0.02);
        let r = length(p);
        let ang = atan2(p.x, p.y);
        for (var k = 1; k <= 3; k = k + 1) {
            col = col + vec3<f32>(0.05, 0.35, 0.12) * line(abs(r - f32(k) * 0.3), 0.004, aa);
        }
        let sweep = fract((ang / (2.0 * PI)) - t * 0.4);
        col = col + vec3<f32>(0.1, 0.9, 0.3) * pow(sweep, 12.0) * step(r, 0.92);
        col = col + vec3<f32>(0.05, 0.4, 0.15) * line(abs(p.x), 0.003, aa) * step(r, 0.92);
        if (frame.lock.w > 0.1) {
            let tg = normalize(frame.lock.xyz);
            let bx = dot(tg, -frame.left.xyz);
            let bz = dot(tg, frame.fwd.xyz);
            let bp = vec2<f32>(bx, bz) * 0.85;
            col = col + vec3<f32>(1.0, 0.9, 0.3) * line(length(p - bp), 0.04, aa);
        }
        col = col * step(r, 0.95) + vec3<f32>(0.02) * step(0.95, r);
    } else {
        // engines and weapons: the throttle and afterburner bars, g, the gear, the stores
        col = vec3<f32>(0.01, 0.01, 0.015);
        let thr = frame.d1.z;
        let bar = step(abs(p.x + 0.65), 0.12) * step(-0.8, p.y) * step(p.y, -0.8 + 1.5 * thr);
        col = col + vec3<f32>(0.2, 0.9, 0.35) * bar;
        let ab = step(abs(p.x + 0.35), 0.08) * step(-0.8, p.y) * step(p.y, -0.8 + 1.5 * frame.d1.w);
        col = col + vec3<f32>(1.0, 0.45, 0.1) * ab;
        col = col + vec3<f32>(0.8) * number(p, vec2<f32>(0.85, 0.55), 0.22, i32(frame.d2.x * 10.0), aa * 2.0);
        col = col + vec3<f32>(0.8) * word(p, vec2<f32>(-0.05, 0.55), 0.22, vec4<u32>(0x3Du, 0u, 0u, 0u), aa * 2.0);
        // three gear lights: green down, red moving, dark up
        let gear = frame.d3.w;
        let lamp = select(select(vec3<f32>(0.9, 0.1, 0.05), vec3<f32>(0.1, 0.9, 0.2), gear > 0.97), vec3<f32>(0.05), gear < 0.03);
        for (var k = 0; k < 3; k = k + 1) {
            let c = vec2<f32>(0.1 + f32(k) * 0.25, 0.1);
            col = col + lamp * line(length(p - c), 0.07, aa);
        }
        col = col + vec3<f32>(0.9, 0.85, 0.3) * number(p, vec2<f32>(0.85, -0.6), 0.22, i32(frame.d3.y + frame.d3.z), aa * 2.0);
        col = col + vec3<f32>(0.9, 0.85, 0.3) * number(p, vec2<f32>(0.85, -0.25), 0.22, i32(frame.d3.x), aa * 2.0);
    }
    // a faint scanline grain
    col = col * (0.92 + 0.08 * sin(uv.y * 400.0));
    return col;
}

// ---- the jet's surfaces ---------------------------------------------------------------------------

@fragment
fn fs_jet(in: VOut) -> @location(0) vec4<f32> {
    // (derivatives first, while every pixel of the quad still runs)
    let ray = normalize(in.world);
    let hud_at = hud_angles(ray);
    let hud_aa = max(fwidth(hud_at.x), 0.01) * 0.8;
    let screen_aa = max(fwidth(in.color.y), 0.001) * 1.5;
    let m = i32(in.material + 0.5);
    let glassy = m == GLASS || m == HUD;
    let glass_pass = object.params.w;
    if ((glass_pass > 0.5) != glassy) {
        discard;
    }
    // hidden behind the world (the depth is reversed: larger is nearer)
    if (object.params.z > 0.5) {
        let px = vec2<i32>(in.clip.xy * frame.screen.zw / frame.screen.xy);
        let last = vec2<i32>(frame.screen.zw) - vec2<i32>(1, 1);
        let d = textureLoad(world_depth, clamp(px, vec2<i32>(0, 0), last), 0).r;
        if (d > in.clip.z * 1.0005 + 1.0e-7) {
            discard;
        }
    }
    // the hull is open where the cockpit sits, under the canopy
    let q = in.local;
    if (m == PAINT && object.cut.x > 0.0 && abs(q.x) < object.cut.x && q.y > object.cut.y && q.z > object.cut.z && q.z < object.cut.w) {
        discard;
    }
    let view = normalize(-in.world);
    var n = normalize(in.normal);
    let inside = dot(n, view) < 0.0;
    if (inside) {
        n = -n;
    }
    let t = frame.misc.x;
    let glow = object.params.x;
    var col = vec3<f32>(0.0);
    var alpha = 1.0;
    switch (m) {
        case 0: {
            var base = in.color;
            // panel lines and a little wear, fixed to the airframe
            let q = in.local;
            let fz = abs(fract(q.z * 0.62) - 0.5);
            let fx = abs(fract(q.x * 0.85 + 0.5 * floor(q.z * 0.62)) - 0.5);
            let seam = max(smoothstep(0.485, 0.497, fz), smoothstep(0.485, 0.497, fx) * step(0.35, abs(in.normal.y)));
            base = base * (1.0 - 0.18 * seam) * (0.94 + 0.06 * noise1(floor(q.xz * 3.0) + floor(q.y * 3.0)));
            if (inside) {
                base = vec3<f32>(0.15, 0.16, 0.17);
            }
            col = shade(base, n, view, 0.42, 0.25);
        }
        case 1: {
            col = shade(in.color, n, view, 0.32, 0.85);
        }
        case 2: {
            // canopy glass: tinted, mirror-like at grazing angles
            let nv = max(dot(n, view), 0.0);
            let fres = 0.08 + 0.92 * pow(1.0 - nv, 4.0);
            let r = reflect(-view, n);
            let sun = normalize(frame.sun.xyz);
            let glint = pow(max(dot(r, sun), 0.0), 400.0) * 6.0 * (1.0 - frame.sun.w);
            col = (sky_color(r.y) * fres + vec3<f32>(glint)) + in.color * 0.05;
            alpha = select(0.32 + 0.55 * fres, 0.12 + 0.2 * fres, inside);
            return vec4<f32>(col * alpha + vec3<f32>(glint) * 0.3, alpha);
        }
        case 3: {
            col = vec3<f32>(1.0, 0.08, 0.05) * 3.0;
        }
        case 4: {
            col = vec3<f32>(0.08, 1.0, 0.25) * 3.0;
        }
        case 5: {
            let on = step(fract(t * 1.1), 0.07);
            col = mix(shade(vec3<f32>(0.8), n, view, 0.3, 0.0), vec3<f32>(8.0), on);
        }
        case 6: {
            col = mix(vec3<f32>(0.6, 0.12, 0.03), vec3<f32>(1.0, 0.75, 0.45), clamp(glow * 0.6, 0.0, 1.0)) * (0.2 + glow * 2.5);
        }
        case 7: {
            col = shade(vec3<f32>(0.03), n, view, 0.9, 0.0);
        }
        case 8: {
            col = shade(in.color, n, view, 0.8, 0.0);
        }
        case 9: {
            // the heads-up display's glass: faintly green, the symbols bright on it
            var symbols = 0.0;
            if (frame.misc.y > 0.5) {
                symbols = hud(ray, hud_at, hud_aa);
            }
            let green = vec3<f32>(0.25, 1.0, 0.45) * (1.6 + 1.2 * (1.0 - frame.sun.w));
            return vec4<f32>(green * symbols + in.color * 0.02, 0.05);
        }
        case 10: {
            col = screen(i32(in.color.x + 0.5), in.color.yz, screen_aa) * 1.4;
        }
        case 11: {
            // switch panels: dark, with small lit legends
            let g = fract(in.local.xz * vec2<f32>(14.0, 9.0));
            let lit = step(0.75, noise1(floor(in.local.xz * vec2<f32>(14.0, 9.0)))) * step(abs(g.x - 0.5), 0.2) * step(abs(g.y - 0.5), 0.12);
            col = shade(in.color, n, view, 0.6, 0.1) + vec3<f32>(1.0, 0.7, 0.3) * lit * 0.8;
        }
        default: {
            col = in.color * 0.02;
        }
    }
    return vec4<f32>(col, alpha);
}

"#;

/// Lays the jets' resolved layer (premultiplied) over the picture.
pub const COMPOSITE: &str = r#"
@group(0) @binding(0) var layer: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;

struct Full {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> Full {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    var out: Full;
    out.pos = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, 1.0 - y);
    return out;
}

@fragment
fn fs_composite(in: Full) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(layer, layer_sampler, in.uv, 0.0);
    return c;
}
"#;
