//! Superflight's physics: a free orientation in 3D (no "up" it must keep), steered by the mouse
//! in the flyer's own frame, rolled with the strafe keys, with auto barrel rolls, and the
//! velocity it gives. Pure math: the hooks feed it input and read back the view and velocity.
//!
//! Conventions are vanilla's: x east, y up, z south; a yaw of 0 looks south (+z), 90 west; a
//! positive pitch looks down. Roll is positive with the right wing down.

pub type V3 = [f64; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn normalize(a: V3) -> V3 {
    let l = length(a);
    if l < 1e-12 { [0.0, 0.0, 1.0] } else { scale(a, 1.0 / l) }
}

/// Rotates `v` by `angle` radians about the unit `axis` (right-handed).
pub fn rotate(v: V3, axis: V3, angle: f64) -> V3 {
    let (s, c) = angle.sin_cos();
    add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)))
}

/// Where vanilla's yaw and pitch (degrees) look.
pub fn look(yaw: f64, pitch: f64) -> V3 {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}

/// Vanilla's yaw and pitch (degrees) for a direction.
pub fn yaw_pitch(forward: V3) -> (f64, f64) {
    let f = normalize(forward);
    let yaw = (-f[0]).atan2(f[2]).to_degrees();
    let pitch = (-f[1]).clamp(-1.0, 1.0).asin().to_degrees();
    (yaw, pitch)
}

/// The camera's up for a yaw and pitch with no roll.
pub fn level_up(yaw: f64, pitch: f64) -> V3 {
    let f = look(yaw, pitch);
    // right = forward x world up; up = right x forward
    let right = normalize(cross(f, [0.0, 1.0, 0.0]));
    let right = if length(cross(f, [0.0, 1.0, 0.0])) < 1e-6 {
        // straight up or down: the right vector from yaw alone
        let y = yaw.to_radians();
        [-y.cos(), 0.0, -y.sin()]
    } else {
        right
    };
    normalize(cross(right, f))
}

/// The flyer: where its nose points and where its back faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attitude {
    pub forward: V3,
    pub up: V3,
}

impl Attitude {
    pub fn from_view(yaw: f64, pitch: f64) -> Self {
        Self { forward: look(yaw, pitch), up: level_up(yaw, pitch) }
    }

    pub fn right(&self) -> V3 {
        normalize(cross(self.forward, self.up))
    }

    /// Keeps the two axes unit length and at right angles.
    fn orthonormalize(&mut self) {
        self.forward = normalize(self.forward);
        let right = normalize(cross(self.forward, self.up));
        self.up = normalize(cross(right, self.forward));
    }

    /// Turns the nose left or right about the flyer's own up (degrees, positive to the right).
    pub fn yaw_by(&mut self, degrees: f64) {
        self.forward = rotate(self.forward, self.up, -degrees.to_radians());
        self.orthonormalize();
    }

    /// Pitches the nose about the flyer's own right (degrees, positive pulls the nose down).
    pub fn pitch_by(&mut self, degrees: f64) {
        let right = self.right();
        let angle = -degrees.to_radians();
        self.forward = rotate(self.forward, right, angle);
        self.up = rotate(self.up, right, angle);
        self.orthonormalize();
    }

    /// Rolls about the nose (degrees, positive puts the right wing down).
    pub fn roll_by(&mut self, degrees: f64) {
        self.up = rotate(self.up, self.forward, degrees.to_radians());
        self.orthonormalize();
    }

    /// The view vanilla can draw: yaw and pitch for the nose, and the roll to add about the
    /// view axis (degrees, positive with the right wing down), in -180..180.
    pub fn view(&self) -> (f64, f64, f64) {
        let (yaw, pitch) = yaw_pitch(self.forward);
        let level = level_up(yaw, pitch);
        let roll = dot(cross(level, self.up), self.forward).atan2(dot(level, self.up)).to_degrees();
        (yaw, pitch, roll)
    }
}

/// What the player does this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// -1 (S) to 1 (W).
    pub thrust: f64,
    /// -1 (A) to 1 (D).
    pub roll: f64,
    /// Space climbs (along the flyer's up), sneak sinks.
    pub lift: f64,
    pub boost: bool,
}

/// Flying, tick by tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    pub attitude: Attitude,
    pub velocity: V3,
    /// A barrel roll in progress: degrees still to turn (signed).
    pub auto_roll: f64,
    /// A loop in progress: degrees still to pitch up.
    pub auto_loop: f64,
}

/// Cruise and boost speeds, blocks a tick.
pub const CRUISE: f64 = 1.1;
pub const BOOST: f64 = 3.0;
pub const ROLL_RATE: f64 = 9.0; // degrees a tick with A/D held
pub const BARREL_ROLL_RATE: f64 = 30.0; // a full roll in 12 ticks
pub const LOOP_RATE: f64 = 15.0; // a full loop in 24 ticks

impl Flight {
    pub fn start(yaw: f64, pitch: f64, velocity: V3) -> Self {
        Self { attitude: Attitude::from_view(yaw, pitch), velocity, auto_roll: 0.0, auto_loop: 0.0 }
    }

    pub fn barrel_roll(&mut self, right: bool) {
        if self.auto_roll == 0.0 {
            self.auto_roll = if right { 360.0 } else { -360.0 };
        }
    }

    pub fn do_loop(&mut self) {
        if self.auto_loop == 0.0 {
            self.auto_loop = 360.0;
        }
    }

    /// One tick: roll and loop, then the velocity eases towards where the nose points.
    pub fn tick(&mut self, controls: Controls) {
        self.attitude.roll_by(controls.roll * ROLL_RATE);
        if self.auto_roll != 0.0 {
            let step = self.auto_roll.signum() * BARREL_ROLL_RATE.min(self.auto_roll.abs());
            self.attitude.roll_by(step);
            self.auto_roll -= step;
        }
        if self.auto_loop > 0.0 {
            let step = LOOP_RATE.min(self.auto_loop);
            self.attitude.pitch_by(-step);
            self.auto_loop -= step;
        }
        let target_speed = if controls.thrust < 0.0 {
            0.15
        } else if controls.boost {
            BOOST
        } else {
            CRUISE * (0.6 + 0.4 * controls.thrust.max(0.0))
        };
        let along = scale(self.attitude.forward, target_speed);
        let climb = scale(self.attitude.up, controls.lift * 0.6);
        let target = add(along, climb);
        // Banked turns carve: the velocity follows the nose quickly, faster at low speed.
        let grip = if controls.boost { 0.12 } else { 0.22 };
        self.velocity = add(scale(self.velocity, 1.0 - grip), scale(target, grip));
    }

    pub fn speed(&self) -> f64 {
        length(self.velocity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_level_view_round_trips() {
        for (yaw, pitch) in [(0.0, 0.0), (90.0, 30.0), (-135.0, -60.0), (179.0, 10.0)] {
            let a = Attitude::from_view(yaw, pitch);
            let (y, p, r) = a.view();
            assert!(close(y, yaw) && close(p, pitch) && close(r, 0.0), "{yaw} {pitch}: {y} {p} {r}");
        }
    }

    #[test]
    fn rolling_right_puts_the_right_wing_down() {
        let mut a = Attitude::from_view(0.0, 0.0); // looking south, +z; right is west (-x)
        assert!(close(a.right()[0], -1.0));
        a.roll_by(90.0);
        // right wing down: the flyer's up now points to its old right (west)
        assert!(close(a.up[0], -1.0), "{:?}", a.up);
        let (_, _, roll) = a.view();
        assert!(close(roll, 90.0), "{roll}");
    }

    #[test]
    fn a_barrel_roll_comes_back_level() {
        let mut f = Flight::start(30.0, 0.0, [0.0; 3]);
        f.barrel_roll(true);
        let mut max_roll: f64 = 0.0;
        for _ in 0..12 {
            f.tick(Controls::default());
            max_roll = max_roll.max(f.attitude.view().2.abs());
        }
        assert!(f.auto_roll == 0.0);
        assert!(max_roll > 170.0);
        let (y, p, r) = f.attitude.view();
        assert!(close(y, 30.0) && close(p, 0.0) && r.abs() < 1e-6, "{y} {p} {r}");
    }

    #[test]
    fn a_loop_goes_over_the_top_and_back() {
        let mut f = Flight::start(0.0, 0.0, [0.0; 3]);
        f.do_loop();
        let mut went_inverted = false;
        for _ in 0..24 {
            f.tick(Controls::default());
            went_inverted |= f.attitude.up[1] < -0.9;
        }
        assert!(went_inverted, "upside down at the top of the loop");
        let (y, p, r) = f.attitude.view();
        assert!(close(y, 0.0) && close(p, 0.0) && r.abs() < 1e-6, "{y} {p} {r}");
    }

    #[test]
    fn steering_is_in_the_flyers_own_frame() {
        // Facing south, rolled 90 degrees right: the flyer's up is west, so pulling up swings
        // the nose west, level with the ground, as a banked turn does.
        let mut a = Attitude::from_view(0.0, 0.0);
        a.roll_by(90.0);
        a.pitch_by(-30.0);
        assert!(a.forward[0] < -0.4, "nose swung west (the flyer's up): {:?}", a.forward);
        assert!(a.forward[1].abs() < 1e-6);
    }

    #[test]
    fn boost_is_fast_and_braking_hovers() {
        let mut f = Flight::start(0.0, 0.0, [0.0; 3]);
        for _ in 0..60 {
            f.tick(Controls { thrust: 1.0, boost: true, ..Default::default() });
        }
        assert!((f.speed() - BOOST).abs() < 0.05);
        for _ in 0..60 {
            f.tick(Controls { thrust: -1.0, ..Default::default() });
        }
        assert!(f.speed() < 0.2);
    }
}
