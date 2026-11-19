//! The jets' flight model: a rigid body in six degrees of freedom, flown by forces alone.
//!
//! Every lifting surface (each wing half, the tailplanes, the fins, canards) is its own little
//! wing: it sees the air as the jet's velocity plus its own motion from the jet's rotation, works
//! out its angle of attack, and makes lift and drag from a lift curve that stalls. Control
//! surfaces change that angle. Thrust, gravity, body drag (with a rise near the speed of sound),
//! landing gear on springs and the air thinning with height do the rest: stability, stalls,
//! spins, energy bleeding in turns, takeoffs and landings come out of the sums.
//!
//! Units: blocks (metres), seconds, kilograms. The body frame is right-handed like the world's:
//! x to the jet's LEFT, y up, z forward; `Q` turns it into the world's. Controls count positive
//! as nose up, roll right (right wing down) and yaw right.

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

/// Sink rates (blocks a second) the airframe and the landing gear take, and how fast it may
/// scrape along a wall before that is a crash.
pub const CRASH_SINK: f64 = 10.0;
pub const CRASH_GEAR: f64 = 13.0;
pub const CRASH_WALL: f64 = 22.0;

pub const GRAVITY: f64 = 9.81;
/// Air at sea level, thicker than Earth's: Minecraft's skies are small, so jets fly slower here
/// and need more air under their wings.
pub const RHO0: f64 = 5.0;
pub const SEA_LEVEL: f64 = 63.0;
/// Air thins by e every this many blocks of height above the sea.
const SCALE_HEIGHT: f64 = 480.0;
/// The speed of sound, in blocks a second.
pub const SOUND: f64 = 85.0;

// ---- vectors and rotations ----------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub const fn v(x: f64, y: f64, z: f64) -> V {
    V { x, y, z }
}

impl V {
    pub const ZERO: V = v(0.0, 0.0, 0.0);
    pub const UP: V = v(0.0, 1.0, 0.0);

    pub fn dot(self, o: V) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: V) -> V {
        v(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn norm(self) -> V {
        let l = self.len();
        if l > 1e-12 { self * (1.0 / l) } else { V::ZERO }
    }

    pub fn from(a: [f64; 3]) -> V {
        v(a[0], a[1], a[2])
    }

    pub fn arr(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    pub fn f32(self) -> [f32; 3] {
        [self.x as f32, self.y as f32, self.z as f32]
    }

    pub fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn lerp(self, o: V, t: f64) -> V {
        self + (o - self) * t
    }
}

impl Add for V {
    type Output = V;
    fn add(self, o: V) -> V {
        v(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for V {
    type Output = V;
    fn sub(self, o: V) -> V {
        v(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for V {
    type Output = V;
    fn mul(self, s: f64) -> V {
        v(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for V {
    type Output = V;
    fn neg(self) -> V {
        v(-self.x, -self.y, -self.z)
    }
}

impl AddAssign for V {
    fn add_assign(&mut self, o: V) {
        *self = *self + o;
    }
}

/// A rotation (unit quaternion).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Q {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Q {
    pub const IDENTITY: Q = Q { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };

    pub fn axis_angle(axis: V, angle: f64) -> Q {
        let a = axis.norm();
        let (s, c) = (angle * 0.5).sin_cos();
        Q { w: c, x: a.x * s, y: a.y * s, z: a.z * s }
    }

    pub fn mul(self, o: Q) -> Q {
        Q {
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
        }
    }

    pub fn conj(self) -> Q {
        Q { w: self.w, x: -self.x, y: -self.y, z: -self.z }
    }

    /// Body to world.
    pub fn rotate(self, p: V) -> V {
        let u = v(self.x, self.y, self.z);
        let t = u.cross(p) * 2.0;
        p + t * self.w + u.cross(t)
    }

    /// World to body.
    pub fn unrotate(self, p: V) -> V {
        self.conj().rotate(p)
    }

    pub fn norm(self) -> Q {
        let l = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if l < 1e-12 {
            return Q::IDENTITY;
        }
        Q { w: self.w / l, x: self.x / l, y: self.y / l, z: self.z / l }
    }

    /// The body's left, up and forward axes in the world.
    pub fn basis(self) -> (V, V, V) {
        (self.rotate(v(1.0, 0.0, 0.0)), self.rotate(V::UP), self.rotate(v(0.0, 0.0, 1.0)))
    }

    /// The rotation whose body axes (left, up, forward) are these world vectors.
    pub fn from_basis(right: V, up: V, fwd: V) -> Q {
        let (m00, m11, m22) = (right.x, up.y, fwd.z);
        let trace = m00 + m11 + m22;
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            Q { w: 0.25 * s, x: (up.z - fwd.y) / s, y: (fwd.x - right.z) / s, z: (right.y - up.x) / s }
        } else if m00 > m11 && m00 > m22 {
            let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
            Q { w: (up.z - fwd.y) / s, x: 0.25 * s, y: (up.x + right.y) / s, z: (fwd.x + right.z) / s }
        } else if m11 > m22 {
            let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
            Q { w: (fwd.x - right.z) / s, x: (up.x + right.y) / s, y: 0.25 * s, z: (fwd.y + up.z) / s }
        } else {
            let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
            Q { w: (right.y - up.x) / s, x: (fwd.x + right.z) / s, y: (fwd.y + up.z) / s, z: 0.25 * s }
        };
        q.norm()
    }

    /// Level, facing `yaw` degrees as Minecraft counts them (0 south, 90 west).
    pub fn facing(yaw_degrees: f64) -> Q {
        let a = yaw_degrees.to_radians();
        let fwd = v(-a.sin(), 0.0, a.cos());
        Q::from_basis(V::UP.cross(fwd).norm(), V::UP, fwd)
    }

    pub fn slerp(self, mut o: Q, t: f64) -> Q {
        let mut d = self.w * o.w + self.x * o.x + self.y * o.y + self.z * o.z;
        if d < 0.0 {
            o = Q { w: -o.w, x: -o.x, y: -o.y, z: -o.z };
            d = -d;
        }
        if d > 0.9995 {
            return Q {
                w: self.w + (o.w - self.w) * t,
                x: self.x + (o.x - self.x) * t,
                y: self.y + (o.y - self.y) * t,
                z: self.z + (o.z - self.z) * t,
            }
            .norm();
        }
        let a = d.acos();
        let s = a.sin();
        let (k0, k1) = (((1.0 - t) * a).sin() / s, (t * a).sin() / s);
        Q { w: self.w * k0 + o.w * k1, x: self.x * k0 + o.x * k1, y: self.y * k0 + o.y * k1, z: self.z * k0 + o.z * k1 }
    }
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How thick the air is at height `y`.
pub fn density(y: f64) -> f64 {
    RHO0 * (-(y - SEA_LEVEL).max(0.0) / SCALE_HEIGHT).exp()
}

/// How much more a body drags near and past the speed of sound.
pub fn mach_drag(mach: f64) -> f64 {
    1.0 + 0.6 * smoothstep(0.86, 1.02, mach) - 0.22 * smoothstep(1.1, 1.6, mach)
}

// ---- what a jet is made of ------------------------------------------------------------------------

/// Which stick, pedal or computer a surface answers to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Fixed,
    /// Ailerons: deflection is `sign * roll`.
    Roll(f64),
    /// Tailplanes and canards: `sign * pitch`.
    Pitch(f64),
    /// Rudders: `sign * yaw`.
    Yaw(f64),
    /// A tailless wing's elevons: pitch and roll mixed.
    Elevon { pitch: f64, roll: f64 },
    /// A flying wing's split drag rudders: drag on one side for yaw `sign`.
    Drag(f64),
}

/// One lifting surface.
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    /// Where its centre of lift sits, from the centre of mass.
    pub pos: V,
    /// Which way its lift points at zero angle of attack, and its chord (towards its leading edge).
    pub normal: V,
    pub chord: V,
    pub area: f64,
    pub aspect: f64,
    /// The angle of attack where it stalls, in radians.
    pub stall: f64,
    pub cd0: f64,
    pub incidence: f64,
    pub control: Control,
    /// How far its control surface moves, in radians, and how much of that turns the flow.
    pub throw: f64,
    pub tau: f64,
    /// The model part it moves (0: none).
    pub part: u8,
}

/// A jet's design, from which its flying follows.
#[derive(Clone, Debug)]
pub struct Spec {
    pub name: &'static str,
    pub about: &'static str,
    pub mass: f64,
    pub length: f64,
    pub span: f64,
    /// Resistance to pitching, yawing and rolling.
    pub inertia: V,
    pub wing_area: f64,
    pub surfaces: Vec<Surface>,
    /// The same surfaces as the model draws them.
    pub shapes: Vec<Shape>,
    /// Dry thrust and with afterburner, in newtons.
    pub thrust: f64,
    pub afterburner: f64,
    /// How far the nozzles can steer the thrust, radians (0: they can't).
    pub vectoring: f64,
    /// Drag area (Cd x area) side-on, top-down and head-on.
    pub drag_area: V,
    /// Where the pilot's eyes are, from the centre of mass.
    pub eye: V,
    /// The engines' nozzles, and their radius.
    pub engines: Vec<V>,
    pub nozzle: f64,
    /// The bottoms of the nose wheel and the two mains, gear down.
    pub gear: [V; 3],
    /// Points of the airframe that mustn't touch the world: nose, tail, wingtips, belly, fin.
    pub hull: Vec<V>,
    /// Third-person chase distance.
    pub chase: f64,
    /// Fastest roll and pitch rates the fly-by-wire asks for, radians a second.
    pub max_roll: f64,
    pub max_pitch: f64,
    /// Highest angle of attack the limiter allows.
    pub alpha_limit: f64,
    pub guns: bool,
    pub missiles: u8,
    pub bombs: u8,
    /// Bombs dropped in a stick (the Leviathan's carpet), rather than one per click.
    pub carpet: bool,
    /// Whether the afterburner shows a flame (the stealth jet hides it).
    pub flame: bool,
    pub paint: [f32; 3],
    pub trim: [f32; 3],
}

pub const FALCON: usize = 0;
pub const SPECTER: usize = 1;
pub const VIPER: usize = 2;
pub const TALON: usize = 3;
pub const LEVIATHAN: usize = 4;
pub const COUNT: usize = 5;

/// Model parts the flight model moves (the mesh tags its vertices with these).
pub mod part {
    pub const AILERON_L: u8 = 1;
    pub const AILERON_R: u8 = 2;
    pub const STAB_L: u8 = 3;
    pub const STAB_R: u8 = 4;
    pub const RUDDER_L: u8 = 5;
    pub const RUDDER_R: u8 = 6;
    pub const GEAR: u8 = 7;
    pub const CANARD: u8 = 8;
    pub const BAY: u8 = 9;
    pub const AIRBRAKE: u8 = 10;
    pub const NOZZLE: u8 = 11;
    pub const STICK_PITCH: u8 = 12;
    pub const STICK_ROLL: u8 = 13;
    pub const THROTTLE: u8 = 14;
    pub const PILOT: u8 = 15;
    /// Missiles and bombs on pylons: 16 onwards, one part each.
    pub const STORES: u8 = 16;
    pub const COUNT: usize = 32;
}

/// A tapered wing half, its lift lumped at 40% of its span and a quarter of its mean chord.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    /// Leading edge at the root and the tip, from the centre of mass.
    pub root: V,
    pub tip: V,
    pub root_chord: f64,
    pub tip_chord: f64,
}

/// What a surface is, for drawing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    /// A wing; with ailerons along the outer trailing edge.
    Wing { ailerons: bool },
    /// An all-moving tailplane or canard, turning about a third of its chord.
    AllMoving,
    /// A strip of trailing edge that moves (a delta's elevons).
    Elevon,
    /// A fin, with a rudder along its trailing edge.
    Fin,
}

/// A surface as the model draws it: `panel` is the left half when `mirror`.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub panel: Panel,
    /// The way its upper side faces (the left half's, when mirrored).
    pub up: V,
    pub role: Role,
    /// The model parts it moves: the right half's, then the left's.
    pub parts: (u8, u8),
    pub mirror: bool,
    /// Thickness for its chord.
    pub thickness: f64,
}

impl Panel {
    fn area(&self) -> f64 {
        let span = (self.tip - self.root).len();
        span * (self.root_chord + self.tip_chord) * 0.5
    }

    fn center(&self) -> V {
        let t = 0.42;
        let le = self.root.lerp(self.tip, t);
        let chord = self.root_chord + (self.tip_chord - self.root_chord) * t;
        le - v(0.0, 0.0, chord * 0.25)
    }

    fn aspect(&self) -> f64 {
        let span = (self.tip - self.root).len() * 2.0;
        span * span / (self.area() * 2.0)
    }
}

#[allow(clippy::too_many_arguments)]
fn surface(p: &Panel, normal: V, control: Control, throw: f64, tau: f64, part: u8, stall: f64, incidence: f64) -> Surface {
    Surface {
        pos: p.center(),
        normal: normal.norm(),
        chord: v(0.0, 0.0, 1.0),
        area: p.area(),
        aspect: p.aspect(),
        stall,
        cd0: 0.006,
        incidence,
        control,
        throw,
        tau,
        part,
    }
}

/// Both halves of a horizontal surface, mirrored; `dihedral` tilts them up (radians). `p` is
/// the left half (+x); the first of the two is the right half.
#[allow(clippy::too_many_arguments)]
fn pair(shapes: &mut Vec<Shape>, role: Role, p: &Panel, dihedral: f64, right: Control, left: Control, throw: f64, tau: f64, parts: (u8, u8), stall: f64, incidence: f64) -> [Surface; 2] {
    let thickness = if role == Role::AllMoving || role == Role::Elevon { 0.06 } else { 0.07 };
    shapes.push(Shape { panel: *p, up: v(-dihedral.sin(), dihedral.cos(), 0.0), role, parts, mirror: true, thickness });
    let n_left = v(-dihedral.sin(), dihedral.cos(), 0.0);
    let n_right = v(dihedral.sin(), dihedral.cos(), 0.0);
    let mirrored = Panel { root: v(-p.root.x, p.root.y, p.root.z), tip: v(-p.tip.x, p.tip.y, p.tip.z), root_chord: p.root_chord, tip_chord: p.tip_chord };
    [
        surface(&mirrored, n_right, right, throw, tau, parts.0, stall, incidence),
        surface(p, n_left, left, throw, tau, parts.1, stall, incidence),
    ]
}

/// A fin: `cant` leans it out from upright (radians), on side `side` (1 left, -1 right, 0 middle).
#[allow(clippy::too_many_arguments)]
fn fin(shapes: &mut Vec<Shape>, root: V, height: f64, sweep: f64, root_chord: f64, tip_chord: f64, cant: f64, side: f64, part: u8, throw: f64) -> Surface {
    let up = v(side * cant.sin(), cant.cos(), 0.0);
    let tip = root + up * height - v(0.0, 0.0, height * sweep.tan());
    let p = Panel { root, tip, root_chord, tip_chord };
    // the fin's lift points to the jet's left: a right yaw pushes the tail that way
    let normal = v(cant.cos(), -side * cant.sin(), 0.0);
    shapes.push(Shape { panel: p, up: normal, role: Role::Fin, parts: (part, part), mirror: false, thickness: 0.06 });
    let mut s = surface(&p, normal, Control::Yaw(1.0), throw, 0.45, part, 0.38, 0.0);
    s.aspect = (height * height / p.area()).max(1.0) * 1.6;
    s
}

/// Moves the wing fore or aft so the whole jet's neutral point sits `margin` of the mean chord
/// behind its centre of mass: stable, but not stubborn.
fn place_wing(surfaces: &mut [Surface], wing: &[usize], margin: f64, chord: f64) -> f64 {
    let slope = |s: &Surface| 2.0 * std::f64::consts::PI * s.aspect / (s.aspect + 2.0) * s.area;
    let mut weight = 0.0;
    let mut moment = 0.0;
    let mut wing_weight = 0.0;
    for (i, s) in surfaces.iter().enumerate() {
        if s.normal.y.abs() < 0.5 {
            continue;
        }
        let w = slope(s) * if wing.contains(&i) { 1.0 } else { 0.75 };
        weight += w;
        if wing.contains(&i) {
            wing_weight += w;
        } else {
            moment += w * s.pos.z;
        }
    }
    let neutral = -margin * chord;
    let wing_z = (neutral * weight - moment) / wing_weight;
    let now: f64 = wing.iter().map(|&i| surfaces[i].pos.z * slope(&surfaces[i])).sum::<f64>() / wing.iter().map(|&i| slope(&surfaces[i])).sum::<f64>();
    let shift = wing_z - now;
    for &i in wing {
        surfaces[i].pos.z += shift;
    }
    shift
}

/// Moves the drawn wings along with their lift.
fn shift_shapes(shapes: &mut [Shape], shift: f64, all: bool) {
    for s in shapes {
        if all || matches!(s.role, Role::Wing { .. }) {
            s.panel.root.z += shift;
            s.panel.tip.z += shift;
        }
    }
}

/// The five jets.
pub fn specs() -> &'static [Spec; COUNT] {
    static SPECS: std::sync::OnceLock<[Spec; COUNT]> = std::sync::OnceLock::new();
    SPECS.get_or_init(|| [falcon(), specter(), viper(), talon(), leviathan()])
}

fn inertia(mass: f64, length: f64, span: f64) -> V {
    // pitch, yaw, roll
    v(mass * length * length / 12.0 * 0.55, mass * (length * length + span * span) / 12.0 * 0.5, mass * span * span / 12.0 * 0.35)
}

fn falcon() -> Spec {
    let (mass, length, span) = (9000.0, 15.0, 10.0);
    let wing = Panel { root: v(0.9, -0.1, 2.2), tip: v(5.0, -0.1, -1.2), root_chord: 4.6, tip_chord: 1.4 };
    let stab = Panel { root: v(0.8, 0.0, -4.9), tip: v(3.1, 0.0, -6.4), root_chord: 2.1, tip_chord: 0.9 };
    let mut shapes = Vec::new();
    let [wl, wr] = pair(&mut shapes, Role::Wing { ailerons: true }, &wing, 0.0, Control::Roll(-1.0), Control::Roll(1.0), 0.35, 0.28, (part::AILERON_R, part::AILERON_L), 0.33, 0.02);
    let [sl, sr] = pair(&mut shapes, Role::AllMoving, &stab, -0.05, Control::Pitch(-1.0), Control::Pitch(-1.0), 0.4, 1.0, (part::STAB_R, part::STAB_L), 0.35, 0.0);
    let f = fin(&mut shapes, v(0.0, 0.9, -3.9), 3.1, 0.75, 3.2, 1.3, 0.0, 0.0, part::RUDDER_L, 0.4);
    let mut surfaces = vec![wl, wr, sl, sr, f];
    let shift = place_wing(&mut surfaces, &[0, 1], 0.08, 3.2);
    shift_shapes(&mut shapes, shift, false);
    Spec {
        name: "Falcon",
        about: "light fighter: balanced and easy",
        mass,
        length,
        span,
        inertia: inertia(mass, length, span),
        wing_area: wing.area() * 2.0,
        surfaces,
        shapes,
        thrust: 76_000.0,
        afterburner: 128_000.0,
        vectoring: 0.0,
        drag_area: v(9.0, 22.0, 2.9),
        eye: v(0.0, 1.05, 4.2),
        engines: vec![v(0.0, 0.0, -7.4)],
        nozzle: 0.55,
        gear: [v(0.0, -2.1, 4.2), v(-1.25, -2.1, -0.9), v(1.25, -2.1, -0.9)],
        hull: vec![v(0.0, 0.0, 7.5), v(0.0, 0.2, -7.5), v(-5.0, -0.1, -1.6), v(5.0, -0.1, -1.6), v(0.0, -0.95, 1.0), v(0.0, 3.9, -6.2)],
        chase: 18.0,
        max_roll: 4.0,
        max_pitch: 0.75,
        alpha_limit: 0.42,
        guns: true,
        missiles: 4,
        bombs: 2,
        carpet: false,
        flame: true,
        paint: [0.56, 0.6, 0.64],
        trim: [0.12, 0.25, 0.55],
    }
}

fn specter() -> Spec {
    let (mass, length, span) = (13_000.0, 19.0, 13.5);
    let wing = Panel { root: v(1.2, 0.0, 2.6), tip: v(6.75, 0.0, -2.7), root_chord: 6.8, tip_chord: 1.6 };
    let stab = Panel { root: v(1.6, 0.0, -6.2), tip: v(4.4, 0.0, -8.0), root_chord: 2.6, tip_chord: 1.1 };
    let mut shapes = Vec::new();
    let [wl, wr] = pair(&mut shapes, Role::Wing { ailerons: true }, &wing, 0.0, Control::Roll(-1.0), Control::Roll(1.0), 0.35, 0.28, (part::AILERON_R, part::AILERON_L), 0.36, 0.02);
    let [sl, sr] = pair(&mut shapes, Role::AllMoving, &stab, 0.0, Control::Pitch(-1.0), Control::Pitch(-1.0), 0.4, 1.0, (part::STAB_R, part::STAB_L), 0.36, 0.0);
    let f1 = fin(&mut shapes, v(-1.6, 0.9, -4.9), 2.8, 0.65, 3.2, 1.3, 0.45, -1.0, part::RUDDER_R, 0.4);
    let f2 = fin(&mut shapes, v(1.6, 0.9, -4.9), 2.8, 0.65, 3.2, 1.3, 0.45, 1.0, part::RUDDER_L, 0.4);
    let mut surfaces = vec![wl, wr, sl, sr, f1, f2];
    let shift = place_wing(&mut surfaces, &[0, 1], 0.06, 4.6);
    shift_shapes(&mut shapes, shift, false);
    Spec {
        name: "Specter",
        about: "stealth fighter: quick, quiet, no flame",
        mass,
        length,
        span,
        inertia: inertia(mass, length, span),
        wing_area: wing.area() * 2.0,
        surfaces,
        shapes,
        thrust: 112_000.0,
        afterburner: 172_000.0,
        vectoring: 0.0,
        drag_area: v(12.0, 34.0, 3.9),
        eye: v(0.0, 1.1, 5.4),
        engines: vec![v(-1.0, 0.1, -8.9), v(1.0, 0.1, -8.9)],
        nozzle: 0.6,
        gear: [v(0.0, -2.3, 5.6), v(-1.6, -2.3, -1.2), v(1.6, -2.3, -1.2)],
        hull: vec![v(0.0, 0.0, 9.5), v(0.0, 0.2, -9.4), v(-6.75, 0.0, -3.5), v(6.75, 0.0, -3.5), v(0.0, -1.0, 1.0), v(-2.8, 3.5, -7.0), v(2.8, 3.5, -7.0)],
        chase: 22.0,
        max_roll: 3.6,
        max_pitch: 0.7,
        alpha_limit: 0.45,
        guns: true,
        missiles: 6,
        bombs: 0,
        carpet: false,
        flame: false,
        paint: [0.17, 0.18, 0.2],
        trim: [0.3, 0.31, 0.33],
    }
}

fn viper() -> Spec {
    let (mass, length, span) = (10_500.0, 16.0, 9.5);
    // a cranked delta: the wing runs nearly to the tail, and the elevons along its trailing edge
    // work as its tail
    let wing = Panel { root: v(0.9, -0.2, 3.4), tip: v(4.75, -0.2, -4.0), root_chord: 8.4, tip_chord: 0.9 };
    let elevon = Panel { root: v(0.9, -0.2, -4.4), tip: v(4.4, -0.2, -5.0), root_chord: 2.0, tip_chord: 1.1 };
    let mut shapes = Vec::new();
    let [wr, wl] = pair(&mut shapes, Role::Wing { ailerons: false }, &wing, -0.03, Control::Fixed, Control::Fixed, 0.0, 0.0, (0, 0), 0.5, 0.0);
    let [er, el] = pair(
        &mut shapes,
        Role::Elevon,
        &elevon,
        -0.03,
        Control::Elevon { pitch: -1.0, roll: -1.0 },
        Control::Elevon { pitch: -1.0, roll: 1.0 },
        0.4,
        1.0,
        (part::AILERON_R, part::AILERON_L),
        0.45,
        0.0,
    );
    let f = fin(&mut shapes, v(0.0, 0.9, -3.6), 3.6, 0.9, 4.0, 1.2, 0.0, 0.0, part::RUDDER_L, 0.4);
    let mut surfaces = vec![wr, wl, er, el, f];
    let shift = place_wing(&mut surfaces, &[0, 1], 0.07, 4.8);
    shift_shapes(&mut shapes, shift, false);
    Spec {
        name: "Viper",
        about: "delta interceptor: fastest, wide turns",
        mass,
        length,
        span,
        inertia: inertia(mass, length, span),
        wing_area: wing.area() * 2.0,
        surfaces,
        shapes,
        thrust: 92_000.0,
        afterburner: 160_000.0,
        vectoring: 0.0,
        drag_area: v(9.0, 26.0, 3.0),
        eye: v(0.0, 1.05, 4.6),
        engines: vec![v(0.0, 0.0, -7.9)],
        nozzle: 0.6,
        gear: [v(0.0, -2.2, 4.4), v(-1.4, -2.2, -1.6), v(1.4, -2.2, -1.6)],
        hull: vec![v(0.0, 0.0, 8.0), v(0.0, 0.2, -8.0), v(-4.75, -0.2, -5.4), v(4.75, -0.2, -5.4), v(0.0, -0.95, 1.0), v(0.0, 4.4, -7.0)],
        chase: 19.0,
        max_roll: 3.4,
        max_pitch: 0.6,
        alpha_limit: 0.5,
        guns: true,
        missiles: 6,
        bombs: 0,
        carpet: false,
        flame: true,
        paint: [0.86, 0.87, 0.89],
        trim: [0.75, 0.08, 0.08],
    }
}

fn talon() -> Spec {
    let (mass, length, span) = (14_000.0, 19.0, 13.5);
    let wing = Panel { root: v(1.4, 0.0, 2.0), tip: v(6.75, 0.0, -2.6), root_chord: 6.2, tip_chord: 1.5 };
    let stab = Panel { root: v(1.9, 0.0, -6.1), tip: v(4.6, 0.0, -7.6), root_chord: 2.4, tip_chord: 1.0 };
    let canard = Panel { root: v(0.9, 0.3, 5.8), tip: v(2.6, 0.3, 4.9), root_chord: 1.6, tip_chord: 0.7 };
    let mut shapes = Vec::new();
    let [wl, wr] = pair(&mut shapes, Role::Wing { ailerons: true }, &wing, 0.0, Control::Roll(-1.0), Control::Roll(1.0), 0.35, 0.28, (part::AILERON_R, part::AILERON_L), 0.4, 0.02);
    let [sl, sr] = pair(&mut shapes, Role::AllMoving, &stab, 0.0, Control::Pitch(-1.0), Control::Pitch(-1.0), 0.45, 1.0, (part::STAB_R, part::STAB_L), 0.4, 0.0);
    let [cl, cr] = pair(&mut shapes, Role::AllMoving, &canard, 0.0, Control::Pitch(1.0), Control::Pitch(1.0), 0.35, 1.0, (part::CANARD, part::CANARD), 0.45, 0.0);
    let f1 = fin(&mut shapes, v(-2.0, 0.8, -4.7), 2.7, 0.7, 3.0, 1.2, 0.35, -1.0, part::RUDDER_R, 0.45);
    let f2 = fin(&mut shapes, v(2.0, 0.8, -4.7), 2.7, 0.7, 3.0, 1.2, 0.35, 1.0, part::RUDDER_L, 0.45);
    let mut surfaces = vec![wl, wr, sl, sr, cl, cr, f1, f2];
    // relaxed stability: the fly-by-wire keeps it pointed
    let shift = place_wing(&mut surfaces, &[0, 1], 0.03, 4.3);
    shift_shapes(&mut shapes, shift, false);
    Spec {
        name: "Talon",
        about: "thrust-vectoring dogfighter: Cobra on C",
        mass,
        length,
        span,
        inertia: inertia(mass, length, span),
        wing_area: wing.area() * 2.0,
        surfaces,
        shapes,
        thrust: 120_000.0,
        afterburner: 196_000.0,
        vectoring: 0.3,
        drag_area: v(12.0, 34.0, 3.6),
        eye: v(0.0, 1.15, 5.9),
        engines: vec![v(-1.25, 0.0, -8.9), v(1.25, 0.0, -8.9)],
        nozzle: 0.62,
        gear: [v(0.0, -2.3, 5.4), v(-1.7, -2.3, -1.3), v(1.7, -2.3, -1.3)],
        hull: vec![v(0.0, 0.0, 9.5), v(0.0, 0.2, -9.4), v(-6.75, 0.0, -3.4), v(6.75, 0.0, -3.4), v(0.0, -1.0, 1.0), v(-2.9, 3.3, -6.5), v(2.9, 3.3, -6.5)],
        chase: 22.0,
        max_roll: 4.4,
        max_pitch: 0.95,
        alpha_limit: 0.55,
        guns: true,
        missiles: 4,
        bombs: 0,
        carpet: false,
        flame: true,
        paint: [0.36, 0.43, 0.52],
        trim: [0.2, 0.25, 0.33],
    }
}

fn leviathan() -> Spec {
    let (mass, length, span) = (60_000.0, 20.0, 52.0);
    // the flying wing: an inner and an outer panel each side
    let inner = Panel { root: v(2.5, 0.0, 9.0), tip: v(10.0, -0.1, 2.0), root_chord: 17.0, tip_chord: 9.0 };
    let outer = Panel { root: v(10.0, -0.1, 2.0), tip: v(26.0, 0.0, -9.5), root_chord: 9.0, tip_chord: 2.2 };
    let mut shapes = Vec::new();
    let [ir, il] = pair(&mut shapes, Role::Wing { ailerons: false }, &inner, 0.0, Control::Fixed, Control::Fixed, 0.0, 0.0, (0, 0), 0.38, 0.0);
    let [or, ol] = pair(&mut shapes, Role::Wing { ailerons: false }, &outer, 0.04, Control::Fixed, Control::Fixed, 0.0, 0.0, (0, 0), 0.36, -0.03);
    // elevons along the trailing edge: the inner ones pitch, the outer ones pitch and roll
    let inner_elevon = Panel { root: v(3.0, 0.0, -6.4), tip: v(10.0, -0.1, -5.4), root_chord: 2.4, tip_chord: 2.0 };
    let outer_elevon = Panel { root: v(12.0, -0.1, -5.8), tip: v(24.5, 0.0, -10.0), root_chord: 3.0, tip_chord: 2.0 };
    let [ier, iel] = pair(&mut shapes, Role::Elevon, &inner_elevon, 0.0, Control::Elevon { pitch: -1.0, roll: -0.2 }, Control::Elevon { pitch: -1.0, roll: 0.2 }, 0.4, 1.0, (0, 0), 0.4, 0.0);
    let [oer, oel] = pair(&mut shapes, Role::Elevon, &outer_elevon, 0.04, Control::Elevon { pitch: -0.7, roll: -1.0 }, Control::Elevon { pitch: -0.7, roll: 1.0 }, 0.4, 1.0, (part::AILERON_R, part::AILERON_L), 0.4, 0.0);
    // split drag rudders at the tips: the right one opens to yaw right
    let mut drag_r = or;
    drag_r.area = 0.0;
    drag_r.control = Control::Drag(1.0);
    drag_r.part = part::RUDDER_R;
    let mut drag_l = ol;
    drag_l.area = 0.0;
    drag_l.control = Control::Drag(-1.0);
    drag_l.part = part::RUDDER_L;
    let mut surfaces = vec![ir, il, or, ol, ier, iel, oer, oel, drag_r, drag_l];
    // the whole wing sits so its neutral point is a little behind the centre of mass
    let before: Vec<f64> = surfaces.iter().map(|s| s.pos.z).collect();
    place_wing(&mut surfaces, &[0, 1, 2, 3], 0.05, 9.0);
    let shift = surfaces[0].pos.z - before[0];
    for s in surfaces.iter_mut().skip(4) {
        s.pos.z += shift;
    }
    shift_shapes(&mut shapes, shift, true);
    Spec {
        name: "Leviathan",
        about: "flying-wing bomber: carpet bombing",
        mass,
        length,
        span,
        inertia: inertia(mass, length, span),
        wing_area: (inner.area() + outer.area()) * 2.0,
        surfaces,
        shapes,
        thrust: 300_000.0,
        afterburner: 300_000.0,
        vectoring: 0.0,
        drag_area: v(30.0, 140.0, 13.0),
        eye: v(0.0, 1.6, 6.5),
        engines: vec![v(-3.2, 0.9, -6.5), v(-5.8, 0.8, -5.0), v(3.2, 0.9, -6.5), v(5.8, 0.8, -5.0)],
        nozzle: 0.7,
        gear: [v(0.0, -3.0, 6.5), v(-4.0, -3.0, -1.0), v(4.0, -3.0, -1.0)],
        hull: vec![v(0.0, 0.0, 10.0), v(-26.0, 0.0, -10.0), v(26.0, 0.0, -10.0), v(0.0, -1.3, 2.0), v(-12.0, -0.5, -2.0), v(12.0, -0.5, -2.0)],
        chase: 50.0,
        max_roll: 1.2,
        max_pitch: 0.35,
        alpha_limit: 0.36,
        guns: false,
        missiles: 0,
        bombs: 20,
        carpet: true,
        flame: false,
        paint: [0.23, 0.24, 0.26],
        trim: [0.32, 0.33, 0.35],
    }
}

// ---- the state of a jet in flight ---------------------------------------------------------------

/// What the pilot (or the fly-by-wire) asks of the jet: stick, pedals, throttle, switches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// -1 to 1: nose up, roll right, yaw right.
    pub pitch: f64,
    pub roll: f64,
    pub yaw: f64,
    pub throttle: f64,
    pub afterburner: bool,
    pub airbrake: bool,
    pub brake: bool,
    pub gear_down: bool,
    /// Bomb bay doors open.
    pub bay: bool,
}

/// What the world under the jet is like, as the window sees it.
pub trait Ground {
    /// The top of the solid ground (or water) under `(x, z)`, looking down from `y` a few blocks.
    fn height(&mut self, x: f64, y: f64, z: f64) -> Option<f64>;
    /// Whether the point is inside something solid.
    fn solid(&mut self, x: f64, y: f64, z: f64) -> bool;
}

/// Flat ground at one height, for tests.
#[cfg_attr(not(test), allow(dead_code))]
pub struct Flat(pub f64);

impl Ground for Flat {
    fn height(&mut self, _x: f64, y: f64, _z: f64) -> Option<f64> {
        (y >= self.0 - 4.0).then_some(self.0)
    }

    fn solid(&mut self, _x: f64, y: f64, _z: f64) -> bool {
        y < self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Crash {
    Ground,
    Gear,
    Obstacle,
}

#[derive(Clone, Debug)]
pub struct Sim {
    pub kind: usize,
    pub pos: V,
    pub vel: V,
    pub rot: Q,
    /// Angular velocity in the body frame (x: pitch down, y: yaw right, z: roll left).
    pub omega: V,
    /// Where the controls actually are (they move at a limited rate).
    pub ctl: Controls,
    /// Engine spool (0 to 1) and afterburner light (0 to 1).
    pub engine: f64,
    pub burner: f64,
    /// Gear: 0 up, 1 down. Bay doors: 0 shut, 1 open.
    pub gear: f64,
    pub bay: f64,
    pub on_ground: bool,
    pub crashed: Option<Crash>,
    /// For the instruments: angle of attack and sideslip (radians), load factor (g), Mach.
    pub alpha: f64,
    pub beta: f64,
    pub g: f64,
    pub mach: f64,
    pub stalled: bool,
    /// Each moving model part's angle (radians), by `part`.
    pub parts: [f32; part::COUNT],
    /// The nozzles' steer (pitch, yaw), radians.
    pub nozzle: (f64, f64),
}

impl Sim {
    /// A jet at `pos`, facing `rot`, moving at `speed` along its nose.
    pub fn new(kind: usize, pos: V, rot: Q, speed: f64) -> Self {
        let (_, _, fwd) = rot.basis();
        let mut s = Sim {
            kind,
            pos,
            vel: fwd * speed,
            rot,
            omega: V::ZERO,
            ctl: Controls { throttle: 0.6, gear_down: speed < 1.0, ..Default::default() },
            engine: if speed > 1.0 { 0.6 } else { 0.0 },
            burner: 0.0,
            gear: if speed < 1.0 { 1.0 } else { 0.0 },
            bay: 0.0,
            on_ground: false,
            crashed: None,
            alpha: 0.0,
            beta: 0.0,
            g: 1.0,
            mach: 0.0,
            stalled: false,
            parts: [0.0; part::COUNT],
            nozzle: (0.0, 0.0),
        };
        s.mach = speed / SOUND;
        s
    }

    pub fn spec(&self) -> &'static Spec {
        &specs()[self.kind]
    }

    /// How fast the air flows over the jet.
    pub fn airspeed(&self) -> f64 {
        self.vel.len()
    }

    /// The pilot's eyes in the world.
    pub fn eye(&self) -> V {
        self.pos + self.rot.rotate(self.spec().eye)
    }

    /// One step of `dt` seconds, the controls moving towards `want`.
    pub fn step(&mut self, want: &Controls, ground: &mut dyn Ground, dt: f64) {
        if self.crashed.is_some() {
            return;
        }
        let spec = self.spec();
        // controls move at a limited rate; the switches follow at once
        let rate = 4.5 * dt;
        let toward = |now: f64, to: f64| now + (to - now).clamp(-rate, rate);
        self.ctl.pitch = toward(self.ctl.pitch, want.pitch.clamp(-1.0, 1.0));
        self.ctl.roll = toward(self.ctl.roll, want.roll.clamp(-1.0, 1.0));
        self.ctl.yaw = toward(self.ctl.yaw, want.yaw.clamp(-1.0, 1.0));
        self.ctl.throttle = want.throttle.clamp(0.0, 1.0);
        self.ctl.afterburner = want.afterburner;
        self.ctl.airbrake = want.airbrake;
        self.ctl.brake = want.brake;
        self.ctl.gear_down = want.gear_down;
        self.ctl.bay = want.bay;
        // the engine spools up slowly, the afterburner lights fast
        self.engine += (self.ctl.throttle - self.engine) * (dt / 1.4).min(1.0);
        let lit = if self.ctl.afterburner && self.engine > 0.8 { 1.0 } else { 0.0 };
        self.burner += (lit - self.burner) * (dt / 0.25).min(1.0);
        let gear_to = if self.ctl.gear_down { 1.0 } else { 0.0 };
        self.gear += (gear_to - self.gear).clamp(-dt * 0.45, dt * 0.45);
        let bay_to = if self.ctl.bay { 1.0 } else { 0.0 };
        self.bay += (bay_to - self.bay).clamp(-dt * 2.0, dt * 2.0);

        let (force, torque) = self.forces(spec, ground);
        let (contact_force, contact_torque) = self.contacts(spec, ground);
        if self.crashed.is_some() {
            return;
        }
        let total = force + contact_force;
        let accel = total * (1.0 / spec.mass);
        let up = self.rot.rotate(V::UP);
        // load factor: what the pilot feels along the jet's up, in g
        self.g = (accel + V::UP * GRAVITY).dot(up) / GRAVITY;
        self.vel += accel * dt;
        let tau = torque + contact_torque;
        let i = spec.inertia;
        let w = self.omega;
        let iw = v(i.x * w.x, i.y * w.y, i.z * w.z);
        let gyro = w.cross(iw);
        // a touch of extra damping, as the real airframe's many small effects give
        let damp = v(i.x * w.x, i.y * w.y, i.z * w.z) * 0.12;
        let net = tau - gyro - damp;
        self.omega += v(net.x / i.x, net.y / i.y, net.z / i.z) * dt;
        self.pos += self.vel * dt;
        let spin = Q { w: 0.0, x: self.omega.x, y: self.omega.y, z: self.omega.z };
        let d = self.rot.mul(spin);
        self.rot = Q {
            w: self.rot.w + d.w * 0.5 * dt,
            x: self.rot.x + d.x * 0.5 * dt,
            y: self.rot.y + d.y * 0.5 * dt,
            z: self.rot.z + d.z * 0.5 * dt,
        }
        .norm();
        if !(self.pos.finite() && self.vel.finite() && self.omega.finite()) {
            self.vel = V::ZERO;
            self.omega = V::ZERO;
            self.crashed = Some(Crash::Ground);
        }
        self.parts[part::GEAR as usize] = ((1.0 - self.gear) * 1.65) as f32;
        self.parts[part::BAY as usize] = (self.bay * 1.4) as f32;
        self.parts[part::AIRBRAKE as usize] = if self.ctl.airbrake { 0.9 } else { 0.0 };
        self.parts[part::STICK_PITCH as usize] = (-self.ctl.pitch * 0.35) as f32;
        self.parts[part::STICK_ROLL as usize] = (self.ctl.roll * 0.35) as f32;
        self.parts[part::THROTTLE as usize] = (self.ctl.throttle * 0.7 + self.burner * 0.2) as f32;
    }

    /// Aerodynamic, thrust and gravity forces (world) and torques (body).
    fn forces(&mut self, spec: &Spec, ground: &mut dyn Ground) -> (V, V) {
        let _ = ground;
        let rho = density(self.pos.y);
        let vb = self.rot.unrotate(self.vel);
        let speed = vb.len();
        self.mach = speed / SOUND;
        let mach = mach_drag(self.mach);
        let mut force_b = V::ZERO;
        let mut torque = V::ZERO;
        // the jet's own angle of attack and sideslip, for the instruments
        self.alpha = if speed > 1.0 { (-vb.y).atan2(vb.z) } else { 0.0 };
        self.beta = if speed > 1.0 { (vb.x / speed).asin() } else { 0.0 };
        let mut stalled_area = 0.0;
        let mut wing_area = 0.0;
        for s in &spec.surfaces {
            let deflect = match s.control {
                Control::Fixed | Control::Drag(_) => 0.0,
                Control::Roll(k) => k * self.ctl.roll,
                Control::Pitch(k) => k * self.ctl.pitch,
                Control::Yaw(k) => k * self.ctl.yaw,
                Control::Elevon { pitch, roll } => (pitch * self.ctl.pitch + roll * self.ctl.roll).clamp(-1.0, 1.0),
            } * s.throw;
            if s.part != 0 {
                self.parts[s.part as usize] = deflect as f32;
            }
            if let Control::Drag(side) = s.control {
                let open = (side * self.ctl.yaw).max(0.0);
                if s.part != 0 {
                    self.parts[s.part as usize] = (open * 0.9) as f32;
                }
                let u = vb + self.omega.cross(s.pos);
                let q = 0.5 * rho * u.dot(u);
                let drag = u.norm() * (-q * 1.6 * open * mach);
                force_b += drag;
                torque += s.pos.cross(drag);
                continue;
            }
            let u = vb + self.omega.cross(s.pos);
            let un = u.dot(s.normal);
            let uc = u.dot(s.chord);
            let planar = s.normal * un + s.chord * uc;
            let sp2 = planar.dot(planar);
            if sp2 < 1e-4 {
                continue;
            }
            let alpha = (-un).atan2(uc) + s.incidence + s.tau * deflect;
            let (cl, cd) = lift_drag(alpha, s);
            if s.normal.y.abs() > 0.5 && s.area > 0.0 {
                wing_area += s.area;
                if alpha.abs() > s.stall {
                    stalled_area += s.area;
                }
            }
            let flow = planar * (1.0 / sp2.sqrt());
            let lift_dir = (s.normal - flow * s.normal.dot(flow)).norm();
            let q = 0.5 * rho * sp2 * s.area;
            let f = lift_dir * (q * cl) - flow * (q * (cd + s.cd0 * (mach - 1.0)));
            force_b += f;
            torque += s.pos.cross(f);
        }
        self.stalled = wing_area > 0.0 && stalled_area > wing_area * 0.45 && speed > 3.0;
        // the body's own drag, more with the airbrake out
        let brake = if self.ctl.airbrake { 1.9 } else { 1.0 };
        let da = spec.drag_area;
        let body = v(-vb.x * vb.x.abs() * da.x, -vb.y * vb.y.abs() * da.y, -vb.z * vb.z.abs() * da.z * brake * mach) * (0.5 * rho);
        force_b += body;
        // thrust: thinner air gives less; the nozzles may steer it
        let air = (rho / RHO0).powf(0.7);
        let thrust = (spec.thrust * self.engine + (spec.afterburner - spec.thrust) * self.burner) * air;
        let (np, ny) = if spec.vectoring > 0.0 {
            (-self.ctl.pitch * spec.vectoring, self.ctl.yaw * spec.vectoring * 0.5)
        } else {
            (0.0, 0.0)
        };
        self.nozzle = (np, ny);
        self.parts[part::NOZZLE as usize] = np as f32;
        // nose up wants the tail pushed down: the thrust points down there
        let dir = v(ny.sin(), np.sin(), 1.0).norm();
        let each = thrust / spec.engines.len().max(1) as f64;
        for e in &spec.engines {
            let f = dir * each;
            force_b += f;
            torque += e.cross(f);
        }
        let force = self.rot.rotate(force_b) - V::UP * (GRAVITY * spec.mass);
        (force, torque)
    }

    /// The landing gear and anything else touching the world: forces (world) and torques (body).
    fn contacts(&mut self, spec: &Spec, ground: &mut dyn Ground) -> (V, V) {
        let mut force = V::ZERO;
        let mut torque = V::ZERO;
        self.on_ground = false;
        let share = spec.mass / 3.0;
        // springs that settle the jet a few centimetres
        let k = share * GRAVITY / 0.12;
        let c = 2.0 * (k * share).sqrt() * 0.75;
        let (left, _, fwd) = self.rot.basis();
        if self.gear > 0.97 {
            for (i, wheel) in spec.gear.iter().enumerate() {
                let p = self.pos + self.rot.rotate(*wheel);
                let Some(h) = ground.height(p.x, p.y + 1.0, p.z) else {
                    continue;
                };
                if p.y >= h {
                    continue;
                }
                let pen = (h - p.y).min(1.0);
                let pv = self.vel + self.rot.rotate(self.omega.cross(*wheel));
                if pv.y < -CRASH_GEAR {
                    self.crashed = Some(Crash::Gear);
                    return (V::ZERO, V::ZERO);
                }
                self.on_ground = true;
                let n = (k * pen - c * pv.y).max(0.0);
                // the nose wheel steers with the rudder
                let mut heading = v(fwd.x, 0.0, fwd.z).norm();
                if i == 0 {
                    heading = (heading - v(left.x, 0.0, left.z) * (self.ctl.yaw * 0.5)).norm();
                }
                let side = heading.cross(V::UP);
                let along = pv.dot(heading);
                let across = pv.dot(side);
                let brake = if self.ctl.brake { 0.55 } else { 0.025 };
                let f_along = (-along * share * 2.0).clamp(-brake * n, brake * n);
                let f_side = (-across * share * 10.0).clamp(-0.9 * n, 0.9 * n);
                let f = V::UP * n + heading * f_along + side * f_side;
                force += f;
                torque += wheel.cross(self.rot.unrotate(f));
            }
        }
        // the airframe itself: a scrape when slow, a crash when not
        for point in &spec.hull {
            let p = self.pos + self.rot.rotate(*point);
            if !ground.solid(p.x, p.y, p.z) {
                continue;
            }
            let pv = self.vel + self.rot.rotate(self.omega.cross(*point));
            // A crash is a real impact: coming down hard, or flying into a wall. A tail strike, a belly landing or a bump over a step at
            // speed is a scrape. A wall: the terrain stands more than a step and a half above the
            // point (or so high its top is out of reach).
            let wall = ground.height(p.x, p.y + 2.5, p.z).is_none_or(|h| h - p.y > 1.6);
            let across = v(pv.x, 0.0, pv.z).len();
            if pv.y < -CRASH_SINK || wall && across > CRASH_WALL {
                self.crashed = Some(if wall { Crash::Obstacle } else { Crash::Ground });
                return (V::ZERO, V::ZERO);
            }
            let h = ground.height(p.x, p.y + 1.5, p.z).unwrap_or(p.y + 0.3);
            let pen = (h - p.y).clamp(0.0, 1.0);
            let n = (k * 2.0 * pen - c * pv.y).max(0.0);
            let flat = v(pv.x, 0.0, pv.z);
            let f = V::UP * n - flat.norm() * (0.6 * n).min(flat.len() * share * 4.0);
            force += f;
            torque += point.cross(self.rot.unrotate(f));
            self.on_ground = true;
        }
        (force, torque)
    }
}

/// A surface's lift and drag coefficients at angle of attack `alpha`: a straight lift slope that
/// stalls, then a flat plate's lift and drag beyond.
fn lift_drag(alpha: f64, s: &Surface) -> (f64, f64) {
    let a = alpha.sin().atan2(alpha.cos());
    let slope = 2.0 * std::f64::consts::PI * s.aspect / (s.aspect + 2.0);
    let post = smoothstep(s.stall, s.stall + 0.14, a.abs());
    let linear = slope * a;
    let plate = 1.1 * (2.0 * a).sin();
    let cl = linear * (1.0 - post) + plate * post;
    let induced = linear * linear / (std::f64::consts::PI * 0.8 * s.aspect) * (1.0 - post);
    let cd = induced + post * 1.25 * a.sin() * a.sin() + 0.04 * post;
    (cl, cd)
}

// ---- the pilot and the fly-by-wire ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// The nose follows where the pilot looks.
    #[default]
    MouseAim,
    /// The mouse is the stick, the keys the pedals.
    Direct,
}

/// What the pilot does with their hands and feet this tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pilot {
    pub mode: Mode,
    pub assist: bool,
    /// Where they want the nose (mouse aim), a world direction.
    pub aim: V,
    /// The stick (-1 to 1: pull, roll right) and pedals (yaw right) in direct mode; extra roll
    /// from A and D in mouse aim.
    pub stick_pitch: f64,
    pub stick_roll: f64,
    pub rudder: f64,
    pub throttle: f64,
    pub afterburner: bool,
    pub airbrake: bool,
    pub cobra: bool,
    pub bay: bool,
}

/// The fly-by-wire computer's memory.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fcs {
    pitch_i: f64,
    roll_i: f64,
}

/// How far up from the ground the jet is, as its radar altimeter would say (given by the caller).
pub fn fly(sim: &Sim, pilot: &Pilot, fcs: &mut Fcs, radar_altitude: f64, dt: f64) -> Controls {
    let spec = sim.spec();
    let speed = sim.airspeed();
    let rho = density(sim.pos.y);
    let q = 0.5 * rho * speed * speed;
    // gains fall as the air pushes harder on the surfaces
    let q_ref = 0.5 * RHO0 * 70.0 * 70.0;
    let sched = (q_ref / q.max(1.0)).clamp(0.3, 5.0);
    let w = sim.omega;
    let pitch_rate = -w.x;
    let roll_rate = w.z;
    let yaw_rate = -w.y;
    let mut out = Controls {
        throttle: pilot.throttle,
        afterburner: pilot.afterburner,
        airbrake: pilot.airbrake,
        brake: sim.on_ground && pilot.throttle < 0.05,
        bay: pilot.bay,
        ..Default::default()
    };
    // gear: down when slow and low, up once climbing away
    let stall_ish = (2.0 * spec.mass * GRAVITY / (rho * spec.wing_area * 1.1)).sqrt();
    out.gear_down = if sim.on_ground {
        true
    } else if sim.ctl.gear_down {
        !(radar_altitude > 12.0 && speed > stall_ish * 1.3)
    } else {
        radar_altitude < 25.0 && speed < stall_ish * 1.7
    };
    let (pitch_cmd, roll_cmd, yaw_cmd) = match pilot.mode {
        Mode::MouseAim => {
            let t = sim.rot.unrotate(pilot.aim.norm());
            let off = t.z.clamp(-1.0, 1.0).acos();
            let pitch_err = t.y.atan2(t.z.max(0.05));
            // (body x is the jet's left: a target to the right has t.x below 0)
            let yaw_err = (-t.x).atan2(t.z.max(0.05));
            let up = sim.rot.unrotate(V::UP);
            let level = (-up.x).atan2(up.y);
            let aim_roll = if off > 0.7 && t.y < -0.3 { (-t.x).atan2(t.y) } else { (-t.x).atan2(t.y.abs() + 0.03) };
            let k = smoothstep(0.035, 0.22, off);
            let roll_err = level * (1.0 - k) + aim_roll * k;
            let p = (roll_err * 3.2 + pilot.stick_roll * spec.max_roll).clamp(-spec.max_roll, spec.max_roll);
            let qc = (pitch_err * 2.6).clamp(-spec.max_pitch * 0.45, spec.max_pitch);
            let r = (yaw_err * 1.6 * (1.0 - 0.7 * k)).clamp(-0.35, 0.35);
            (qc, p, r)
        }
        Mode::Direct => (pilot.stick_pitch * spec.max_pitch, pilot.stick_roll * spec.max_roll, pilot.rudder * 0.5),
    };
    let raw = pilot.mode == Mode::Direct && !pilot.assist;
    if raw {
        out.pitch = pilot.stick_pitch;
        out.roll = pilot.stick_roll;
        out.yaw = pilot.rudder;
    } else {
        let e_pitch = pitch_cmd - pitch_rate;
        let e_roll = roll_cmd - roll_rate;
        let e_yaw = yaw_cmd - yaw_rate;
        let pitch = (1.6 * e_pitch + fcs.pitch_i) * sched;
        let roll = (0.55 * e_roll + fcs.roll_i) * sched;
        let mut yaw = 1.2 * e_yaw * sched;
        // keep the jet pointed into the wind
        yaw -= sim.beta * 2.0;
        // integrators trim out what the gains alone can't, unless the output is pinned
        if pitch.abs() < 1.0 && !sim.on_ground {
            fcs.pitch_i = (fcs.pitch_i + e_pitch * 1.8 * dt).clamp(-1.5, 1.5);
        }
        if roll.abs() < 1.0 {
            fcs.roll_i = (fcs.roll_i + e_roll * 0.6 * dt).clamp(-0.5, 0.5);
        }
        out.pitch = pitch;
        out.roll = roll;
        out.yaw = yaw;
        // the limiter: no more angle of attack than the wing can take, no more than 9 g
        let cobra = pilot.cobra && spec.vectoring > 0.0;
        if !cobra && (pilot.assist || pilot.mode == Mode::MouseAim) {
            if sim.alpha > spec.alpha_limit {
                out.pitch -= (sim.alpha - spec.alpha_limit) * 8.0;
            }
            if sim.g > 9.0 {
                out.pitch -= (sim.g - 9.0) * 0.4;
            }
            if sim.g < -3.0 {
                out.pitch += (-3.0 - sim.g) * 0.4;
            }
        }
        if cobra {
            out.pitch = 1.0;
            let up = sim.rot.unrotate(V::UP);
            out.roll = (-up.x).atan2(up.y);
        }
    }
    if sim.on_ground {
        fcs.pitch_i *= 0.9;
        // on the runway the rudder steers the nose wheel
        out.yaw = (out.yaw + pilot.rudder).clamp(-1.0, 1.0);
    }
    out.pitch = out.pitch.clamp(-1.0, 1.0);
    out.roll = out.roll.clamp(-1.0, 1.0);
    out.yaw = out.yaw.clamp(-1.0, 1.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flies `kind` for `seconds` with the pilot aiming level ahead, from `speed`.
    fn cruise(kind: usize, speed: f64, throttle: f64, seconds: f64) -> Sim {
        let mut sim = Sim::new(kind, v(0.0, 200.0, 0.0), Q::IDENTITY, speed);
        sim.ctl.gear_down = false;
        sim.gear = 0.0;
        let mut fcs = Fcs::default();
        let pilot = Pilot { aim: v(0.0, 0.0, 1.0), throttle, assist: true, ..Default::default() };
        let mut ground = Flat(-1000.0);
        let dt = 0.01;
        for _ in 0..(seconds / dt) as usize {
            let ctl = fly(&sim, &pilot, &mut fcs, 500.0, dt);
            sim.step(&ctl, &mut ground, dt);
        }
        sim
    }

    #[test]
    fn rotations_hold_together() {
        let q = Q::axis_angle(v(0.3, 1.0, -0.2), 1.1);
        let p = v(1.0, 2.0, 3.0);
        let back = q.unrotate(q.rotate(p));
        assert!((back - p).len() < 1e-9);
        let (l, u, f) = q.basis();
        let q2 = Q::from_basis(l, u, f);
        assert!((q2.rotate(p) - q.rotate(p)).len() < 1e-9);
        assert!((l.cross(u) - f).len() < 1e-9);
        // facing south, the jet's left is east
        let (l, _, f) = Q::facing(0.0).basis();
        assert!((f - v(0.0, 0.0, 1.0)).len() < 1e-9 && (l - v(1.0, 0.0, 0.0)).len() < 1e-9);
    }

    #[test]
    fn every_jet_holds_level_flight() {
        for kind in 0..COUNT {
            let spec = &specs()[kind];
            let sim = cruise(kind, 75.0, 0.75, 25.0);
            assert!(sim.crashed.is_none(), "{} crashed", spec.name);
            let climb = sim.pos.y - 200.0;
            let speed = sim.airspeed();
            let (_, _, fwd) = sim.rot.basis();
            println!("{}: speed {speed:.1}, height change {climb:.1}, alpha {:.1} deg, g {:.2}, nose y {:.3}", spec.name, sim.alpha.to_degrees(), sim.g, fwd.y);
            assert!(climb.abs() < 90.0, "{} drifted {climb:.1} blocks", spec.name);
            assert!(speed > 35.0 && speed < 140.0, "{} at {speed:.1}", spec.name);
            assert!(fwd.y.abs() < 0.15 && sim.omega.len() < 0.1, "{} not steady: {:?}", spec.name, sim.omega);
        }
    }

    #[test]
    fn full_power_goes_fast_and_idle_slows() {
        for kind in 0..COUNT {
            let spec = &specs()[kind];
            let fast = {
                let mut s = Sim::new(kind, v(0.0, 150.0, 0.0), Q::IDENTITY, 80.0);
                s.gear = 0.0;
                let mut fcs = Fcs::default();
                let pilot = Pilot { aim: v(0.0, 0.0, 1.0), throttle: 1.0, afterburner: true, assist: true, ..Default::default() };
                let mut ground = Flat(-1000.0);
                for _ in 0..4000 {
                    let ctl = fly(&s, &pilot, &mut fcs, 500.0, 0.01);
                    s.step(&ctl, &mut ground, 0.01);
                }
                s
            };
            let slow = cruise(kind, 80.0, 0.0, 15.0);
            println!("{}: top {:.1} b/s (Mach {:.2}), idle after 15 s {:.1}", spec.name, fast.airspeed(), fast.mach, slow.airspeed());
            assert!(fast.airspeed() > 70.0 && fast.airspeed() < 135.0, "{}", spec.name);
            assert!(slow.airspeed() < 70.0, "{}", spec.name);
        }
    }

    #[test]
    fn it_rolls_and_pulls_when_asked() {
        for kind in 0..COUNT {
            let spec = &specs()[kind];
            let mut sim = Sim::new(kind, v(0.0, 200.0, 0.0), Q::IDENTITY, 80.0);
            sim.gear = 0.0;
            let mut fcs = Fcs::default();
            let mut ground = Flat(-1000.0);
            let pilot = Pilot { mode: Mode::Direct, assist: true, stick_roll: 1.0, throttle: 0.8, aim: v(0.0, 0.0, 1.0), ..Default::default() };
            let mut most: f64 = 0.0;
            let mut left_up = 0.0;
            for step in 0..150 {
                let ctl = fly(&sim, &pilot, &mut fcs, 500.0, 0.01);
                sim.step(&ctl, &mut ground, 0.01);
                most = most.max(sim.omega.z);
                if step == 40 {
                    left_up = sim.rot.basis().0.y;
                }
            }
            println!("{}: roll rate {:.0} deg/s", spec.name, most.to_degrees());
            assert!(most > 0.35, "{} rolls at {most}", spec.name);
            assert!(left_up > 0.0, "{} rolled the wrong way", spec.name);

            let mut sim = Sim::new(kind, v(0.0, 200.0, 0.0), Q::IDENTITY, 80.0);
            sim.gear = 0.0;
            let mut fcs = Fcs::default();
            let pilot = Pilot { mode: Mode::Direct, assist: true, stick_pitch: 1.0, throttle: 0.8, aim: v(0.0, 0.0, 1.0), ..Default::default() };
            let mut most_g: f64 = 0.0;
            for _ in 0..200 {
                let ctl = fly(&sim, &pilot, &mut fcs, 500.0, 0.01);
                sim.step(&ctl, &mut ground, 0.01);
                most_g = most_g.max(sim.g);
            }
            let (_, _, fwd) = sim.rot.basis();
            println!("{}: pull to {most_g:.1} g", spec.name);
            assert!(fwd.y > 0.2, "{} didn't pitch up", spec.name);
            assert!(most_g > 2.0 && most_g < 12.0, "{} pulled {most_g}", spec.name);
        }
    }

    #[test]
    fn it_turns_toward_where_you_look() {
        for kind in 0..COUNT {
            let spec = &specs()[kind];
            let mut sim = Sim::new(kind, v(0.0, 200.0, 0.0), Q::IDENTITY, 80.0);
            sim.gear = 0.0;
            let mut fcs = Fcs::default();
            let mut ground = Flat(-1000.0);
            // look 90 degrees to the right
            let (left, _, _) = sim.rot.basis();
            let right = left * -1.0;
            let pilot = Pilot { aim: right, throttle: 0.9, assist: true, ..Default::default() };
            for _ in 0..1500 {
                let ctl = fly(&sim, &pilot, &mut fcs, 500.0, 0.01);
                sim.step(&ctl, &mut ground, 0.01);
            }
            let (_, _, fwd) = sim.rot.basis();
            println!("{}: after 15 s the nose points {:.2} along the aim, height {:.0}", spec.name, fwd.dot(right), sim.pos.y);
            assert!(sim.crashed.is_none());
            assert!(fwd.dot(right) > 0.9, "{} didn't come round", spec.name);
        }
    }

    #[test]
    fn it_takes_off_from_the_ground() {
        for kind in 0..COUNT {
            let spec = &specs()[kind];
            let ground_y = 64.0;
            let wheel = spec.gear[1].y;
            let mut sim = Sim::new(kind, v(0.0, ground_y - wheel + 0.05, 0.0), Q::IDENTITY, 0.0);
            let mut fcs = Fcs::default();
            let mut ground = Flat(ground_y);
            // full power, aim 10 degrees up
            let aim = v(0.0, 0.17, 1.0).norm();
            let pilot = Pilot { aim, throttle: 1.0, afterburner: true, assist: true, ..Default::default() };
            let mut run = None;
            for step in 0..6000 {
                let ctl = fly(&sim, &pilot, &mut fcs, (sim.pos.y - ground_y).max(0.0), 0.01);
                sim.step(&ctl, &mut ground, 0.01);
                if run.is_none() && !sim.on_ground && sim.pos.y > ground_y - wheel + 3.0 {
                    run = Some((sim.pos.z, step as f64 * 0.01));
                }
                assert!(sim.crashed.is_none(), "{} crashed on takeoff: {:?} at {:?}", spec.name, sim.crashed, sim.pos);
            }
            let (distance, time) = run.unwrap_or_else(|| panic!("{} never left the ground", spec.name));
            println!("{}: off the ground after {distance:.0} blocks, {time:.1} s; at 60 s height {:.0}, gear {:.1}", spec.name, sim.pos.y - ground_y, sim.gear);
            assert!(distance < 900.0);
            assert!(sim.pos.y > ground_y + 50.0);
        }
    }

    #[test]
    fn a_hard_dive_into_the_ground_is_a_crash() {
        let mut sim = Sim::new(FALCON, v(0.0, 80.0, 0.0), Q::axis_angle(v(1.0, 0.0, 0.0), 0.8), 90.0);
        let mut ground = Flat(64.0);
        let ctl = Controls { throttle: 1.0, ..Default::default() };
        for _ in 0..300 {
            sim.step(&ctl, &mut ground, 0.01);
        }
        assert!(sim.crashed.is_some());
    }

    /// Ground with a one-block step at `z` = 40 (and, if `wall`, a cliff at `z` = 200).
    struct Steps {
        wall: bool,
    }

    impl Ground for Steps {
        fn height(&mut self, _x: f64, y: f64, z: f64) -> Option<f64> {
            let top = if self.wall && z >= 200.0 { 120.0 } else if z >= 40.0 { 65.0 } else { 64.0 };
            (y >= top - 4.0).then_some(top)
        }

        fn solid(&mut self, _x: f64, y: f64, z: f64) -> bool {
            let top = if self.wall && z >= 200.0 { 120.0 } else if z >= 40.0 { 65.0 } else { 64.0 };
            y < top
        }
    }

    #[test]
    fn rolling_over_a_step_and_a_tail_strike_are_not_crashes() {
        for kind in 0..COUNT {
            let spec = &specs()[kind];
            let wheel = spec.gear[1].y;
            let mut sim = Sim::new(kind, v(0.0, 64.0 - wheel + 0.05, 0.0), Q::IDENTITY, 0.0);
            sim.vel = v(0.0, 0.0, 30.0);
            let mut ground = Steps { wall: false };
            let ctl = Controls { throttle: 0.3, ..Default::default() };
            for _ in 0..300 {
                sim.step(&ctl, &mut ground, 0.01);
            }
            assert!(sim.crashed.is_none(), "{} blew up on a step: {:?}", spec.name, sim.crashed);
            // the tail dragged on the runway while rotating hard
            let mut sim = Sim::new(kind, v(0.0, 64.0 - wheel + 0.05, 0.0), Q::axis_angle(v(1.0, 0.0, 0.0), -0.35), 0.0);
            sim.vel = v(0.0, -1.0, 60.0);
            let mut ground = Flat(64.0);
            for _ in 0..100 {
                sim.step(&ctl, &mut ground, 0.01);
            }
            assert!(sim.crashed.is_none(), "{} blew up on a tail strike: {:?}", spec.name, sim.crashed);
        }
    }

    #[test]
    fn flying_into_a_cliff_is_a_crash() {
        let mut sim = Sim::new(FALCON, v(0.0, 100.0, 0.0), Q::IDENTITY, 80.0);
        sim.gear = 0.0;
        sim.ctl.gear_down = false;
        let mut ground = Steps { wall: true };
        let ctl = Controls { throttle: 0.8, ..Default::default() };
        for _ in 0..400 {
            sim.step(&ctl, &mut ground, 0.01);
            if sim.crashed.is_some() {
                break;
            }
        }
        assert_eq!(sim.crashed, Some(Crash::Obstacle));
    }
}
