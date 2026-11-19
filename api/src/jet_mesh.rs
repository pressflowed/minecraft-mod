//! The jets' models, built in code: lofted hulls (rings of rounded-square sections), airfoil
//! wings and tails from the flight model's own shapes, glass canopies, intakes, nozzles, landing
//! gear, the weapons on their pylons, and a cockpit inside with its instrument panel, screens and
//! the glass plate of the heads-up display.
//!
//! Positions are in the jet's body frame (x to its left, y up, z forward, the centre of mass at
//! the origin). Each vertex carries a material, and a part, pivot and axis when it moves (control
//! surfaces, gear, bay doors, the stick, the stores).

use crate::jet_sim::{self, Role, Shape, Spec, V, part, v};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub material: f32,
    pub part: f32,
    pub pivot: [f32; 3],
    pub axis: [f32; 3],
}

/// Floats in a vertex.
pub const VERTEX_FLOATS: usize = 17;

/// What a surface is made of (the shader's numbers).
pub mod mat {
    pub const PAINT: f32 = 0.0;
    pub const METAL: f32 = 1.0;
    pub const GLASS: f32 = 2.0;
    pub const NAV_RED: f32 = 3.0;
    pub const NAV_GREEN: f32 = 4.0;
    pub const STROBE: f32 = 5.0;
    pub const HOT: f32 = 6.0;
    pub const RUBBER: f32 = 7.0;
    pub const INTERIOR: f32 = 8.0;
    pub const HUD: f32 = 9.0;
    /// A cockpit screen: the colour holds (which screen, u, v).
    pub const SCREEN: f32 = 10.0;
    pub const BUTTONS: f32 = 11.0;
    pub const INTAKE: f32 = 12.0;
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// The vertices as plain floats, for the GPU.
    pub fn floats(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.vertices.len() * VERTEX_FLOATS);
        for vx in &self.vertices {
            out.extend_from_slice(&vx.pos);
            out.extend_from_slice(&vx.normal);
            out.extend_from_slice(&vx.color);
            out.push(vx.material);
            out.push(vx.part);
            out.extend_from_slice(&vx.pivot);
            out.extend_from_slice(&vx.axis);
        }
        out
    }
}

/// One hull section: a rounded square `w` wide each way and `top`/`bottom` high, centred on
/// (`x`, `y`) at `z`; `n` rounds it (2 an ellipse, higher squarer, below 2 a diamond).
#[derive(Clone, Copy, Debug)]
struct Ring {
    z: f64,
    x: f64,
    y: f64,
    w: f64,
    top: f64,
    bottom: f64,
    n: f64,
}

const fn ring(z: f64, y: f64, w: f64, top: f64, bottom: f64, n: f64) -> Ring {
    Ring { z, x: 0.0, y, w, top, bottom, n }
}

impl Ring {
    fn at(self, x: f64) -> Ring {
        Ring { x, ..self }
    }

    fn point(&self, theta: f64) -> V {
        let (s, c) = theta.sin_cos();
        let e = 2.0 / self.n;
        let px = self.w * c.signum() * c.abs().powf(e);
        let h = if s >= 0.0 { self.top } else { self.bottom };
        let py = h * s.signum() * s.abs().powf(e);
        v(self.x + px, self.y + py, self.z)
    }
}

struct Builder {
    mesh: Mesh,
    color: [f32; 3],
    material: f32,
    part: u8,
    pivot: V,
    axis: V,
}

impl Builder {
    fn new() -> Self {
        Self { mesh: Mesh::default(), color: [0.6; 3], material: mat::PAINT, part: 0, pivot: V::ZERO, axis: V::ZERO }
    }

    fn paint(&mut self, color: [f32; 3], material: f32) -> &mut Self {
        self.color = color;
        self.material = material;
        self
    }

    fn moving(&mut self, part: u8, pivot: V, axis: V) -> &mut Self {
        self.part = part;
        self.pivot = pivot;
        self.axis = axis.norm();
        self
    }

    fn still(&mut self) -> &mut Self {
        self.part = 0;
        self.pivot = V::ZERO;
        self.axis = V::ZERO;
        self
    }

    fn vert(&mut self, p: V, n: V) -> u32 {
        self.vert_colored(p, n, self.color)
    }

    fn vert_colored(&mut self, p: V, n: V, color: [f32; 3]) -> u32 {
        self.mesh.vertices.push(Vertex {
            pos: p.f32(),
            normal: n.norm().f32(),
            color,
            material: self.material,
            part: f32::from(self.part),
            pivot: self.pivot.f32(),
            axis: self.axis.f32(),
        });
        (self.mesh.vertices.len() - 1) as u32
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.mesh.indices.extend_from_slice(&[a, b, c]);
    }

    /// A surface through rows of points (each row a section), smooth or faceted. Normals point
    /// away from each row's middle; `closed` joins each row's ends.
    fn grid(&mut self, rows: &[Vec<V>], closed: bool, flat: bool) {
        if rows.len() < 2 {
            return;
        }
        let cols = rows[0].len();
        let centers: Vec<V> = rows.iter().map(|r| r.iter().fold(V::ZERO, |a, p| a + *p) * (1.0 / r.len() as f64)).collect();
        let along = |i: usize, j: usize| -> V {
            let a = rows[i.saturating_sub(1)][j];
            let b = rows[(i + 1).min(rows.len() - 1)][j];
            b - a
        };
        let across = |i: usize, j: usize| -> V {
            let (a, b) = if closed {
                (rows[i][(j + cols - 1) % cols], rows[i][(j + 1) % cols])
            } else {
                (rows[i][j.saturating_sub(1)], rows[i][(j + 1).min(cols - 1)])
            };
            b - a
        };
        let outward = |n: V, p: V, c: V| if n.dot(p - c) < 0.0 { n * -1.0 } else { n };
        let spans = if closed { cols } else { cols - 1 };
        if flat {
            for i in 0..rows.len() - 1 {
                for j in 0..spans {
                    let j1 = (j + 1) % cols;
                    let (a, b, c, d) = (rows[i][j], rows[i][j1], rows[i + 1][j1], rows[i + 1][j]);
                    let mut n = (c - a).cross(d - b).norm();
                    let mid = (a + b + c + d) * 0.25;
                    n = outward(n, mid, (centers[i] + centers[i + 1]) * 0.5);
                    let ia = self.vert(a, n);
                    let ib = self.vert(b, n);
                    let ic = self.vert(c, n);
                    let id = self.vert(d, n);
                    self.tri(ia, ib, ic);
                    self.tri(ia, ic, id);
                }
            }
            return;
        }
        let base = self.mesh.vertices.len() as u32;
        for (i, row) in rows.iter().enumerate() {
            for (j, p) in row.iter().enumerate() {
                let mut n = along(i, j).cross(across(i, j)).norm();
                if n.len() < 0.5 {
                    n = (*p - centers[i]).norm();
                }
                let n = outward(n, *p, centers[i]);
                self.vert(*p, n);
            }
        }
        for i in 0..rows.len() - 1 {
            for j in 0..spans {
                let j1 = (j + 1) % cols;
                let a = base + (i * cols + j) as u32;
                let b = base + (i * cols + j1) as u32;
                let c = base + ((i + 1) * cols + j1) as u32;
                let d = base + ((i + 1) * cols + j) as u32;
                self.tri(a, b, c);
                self.tri(a, c, d);
            }
        }
    }

    /// Closes a section with a fan, facing `normal`.
    fn cap(&mut self, row: &[V], normal: V) {
        let c = row.iter().fold(V::ZERO, |a, p| a + *p) * (1.0 / row.len() as f64);
        let ic = self.vert(c, normal);
        let first = self.mesh.vertices.len() as u32;
        for p in row {
            self.vert(*p, normal);
        }
        for j in 0..row.len() as u32 {
            let k = (j + 1) % row.len() as u32;
            self.tri(ic, first + j, first + k);
        }
    }

    /// A hull through `rings`, `segs` points round each.
    fn loft(&mut self, rings: &[Ring], segs: usize, flat: bool, caps: bool) {
        let rows: Vec<Vec<V>> = rings
            .iter()
            .map(|r| (0..segs).map(|j| r.point(j as f64 / segs as f64 * std::f64::consts::TAU)).collect())
            .collect();
        self.grid(&rows, true, flat);
        if caps {
            if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
                self.cap(first, v(0.0, 0.0, 1.0));
                self.cap(last, v(0.0, 0.0, -1.0));
            }
        }
    }

    /// The upper half of each ring only: a canopy.
    fn bubble(&mut self, rings: &[Ring], segs: usize) {
        let rows: Vec<Vec<V>> = rings
            .iter()
            .map(|r| (0..=segs).map(|j| r.point(j as f64 / segs as f64 * std::f64::consts::PI)).collect())
            .collect();
        self.grid(&rows, false, false);
    }

    /// A tube from `a` to `b`, radius `r0` to `r1`.
    fn tube(&mut self, a: V, b: V, r0: f64, r1: f64, segs: usize, caps: bool) {
        let axis = (b - a).norm();
        let helper = if axis.y.abs() < 0.9 { V::UP } else { v(1.0, 0.0, 0.0) };
        let u = axis.cross(helper).norm();
        let w = axis.cross(u);
        let ring = |c: V, r: f64| -> Vec<V> {
            (0..segs)
                .map(|j| {
                    let t = j as f64 / segs as f64 * std::f64::consts::TAU;
                    c + u * (t.cos() * r) + w * (t.sin() * r)
                })
                .collect()
        };
        let rows = vec![ring(a, r0), ring(b, r1)];
        self.grid(&rows, true, false);
        if caps {
            self.cap(&rows[0], axis * -1.0);
            self.cap(&rows[1], axis);
        }
    }

    /// An ellipsoid.
    fn blob(&mut self, c: V, r: V, segs: usize) {
        let rows: Vec<Vec<V>> = (0..=segs / 2)
            .map(|i| {
                let lat = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * i as f64 / (segs / 2) as f64;
                (0..segs)
                    .map(|j| {
                        let lon = j as f64 / segs as f64 * std::f64::consts::TAU;
                        c + v(lat.cos() * lon.cos() * r.x, lat.sin() * r.y, lat.cos() * lon.sin() * r.z)
                    })
                    .collect()
            })
            .collect();
        // normals from the centre, so the poles shade right
        let base = self.mesh.vertices.len() as u32;
        let cols = segs;
        for row in &rows {
            for p in row {
                let d = *p - c;
                self.vert(*p, v(d.x / (r.x * r.x), d.y / (r.y * r.y), d.z / (r.z * r.z)));
            }
        }
        for i in 0..rows.len() - 1 {
            for j in 0..cols {
                let j1 = (j + 1) % cols;
                let a = base + (i * cols + j) as u32;
                let b = base + (i * cols + j1) as u32;
                let cc = base + ((i + 1) * cols + j1) as u32;
                let d = base + ((i + 1) * cols + j) as u32;
                self.tri(a, b, cc);
                self.tri(a, cc, d);
            }
        }
    }

    /// A box from `lo` to `hi`, faceted.
    fn cuboid(&mut self, lo: V, hi: V) {
        let p = |x: f64, y: f64, z: f64| v(if x > 0.0 { hi.x } else { lo.x }, if y > 0.0 { hi.y } else { lo.y }, if z > 0.0 { hi.z } else { lo.z });
        let faces = [
            (v(1.0, 0.0, 0.0), [p(1., -1., -1.), p(1., 1., -1.), p(1., 1., 1.), p(1., -1., 1.)]),
            (v(-1.0, 0.0, 0.0), [p(-1., -1., 1.), p(-1., 1., 1.), p(-1., 1., -1.), p(-1., -1., -1.)]),
            (v(0.0, 1.0, 0.0), [p(-1., 1., -1.), p(-1., 1., 1.), p(1., 1., 1.), p(1., 1., -1.)]),
            (v(0.0, -1.0, 0.0), [p(-1., -1., 1.), p(-1., -1., -1.), p(1., -1., -1.), p(1., -1., 1.)]),
            (v(0.0, 0.0, 1.0), [p(-1., -1., 1.), p(1., -1., 1.), p(1., 1., 1.), p(-1., 1., 1.)]),
            (v(0.0, 0.0, -1.0), [p(1., -1., -1.), p(-1., -1., -1.), p(-1., 1., -1.), p(1., 1., -1.)]),
        ];
        for (n, q) in faces {
            self.quad(q, n);
        }
    }

    fn quad(&mut self, q: [V; 4], n: V) {
        let a = self.vert(q[0], n);
        let b = self.vert(q[1], n);
        let c = self.vert(q[2], n);
        let d = self.vert(q[3], n);
        self.tri(a, b, c);
        self.tri(a, c, d);
    }

    /// A quad whose colours carry a cockpit screen's number and its corners' u and v.
    fn screen(&mut self, q: [V; 4], n: V, id: f32) {
        let uv = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let mut idx = [0u32; 4];
        for k in 0..4 {
            idx[k] = self.vert_colored(q[k], n, [id, uv[k][0], uv[k][1]]);
        }
        self.tri(idx[0], idx[1], idx[2]);
        self.tri(idx[0], idx[2], idx[3]);
    }

    /// A slice of an airfoil-section surface: from `c0` to `c1` of the chord and `s0` to `s1`
    /// of the span, `thick` thick for its chord, its upper side facing `up`.
    #[allow(clippy::too_many_arguments)]
    fn foil(&mut self, p: &jet_sim::Panel, up: V, thick: f64, c0: f64, c1: f64, s0: f64, s1: f64) {
        let n = 9;
        let section = |s: f64| -> Vec<V> {
            let le = p.root.lerp(p.tip, s);
            let chord = p.root_chord + (p.tip_chord - p.root_chord) * s;
            let mut pts = Vec::with_capacity(n * 2);
            let station = |k: usize| -> f64 {
                let t = k as f64 / (n - 1) as f64;
                let x = c0 + (c1 - c0) * t;
                // closer together at the leading edge
                if c0 == 0.0 { c1 * (1.0 - (t * std::f64::consts::FRAC_PI_2).cos()) } else { x }
            };
            let half = |x: f64| -> f64 {
                let x = x.clamp(0.0, 1.0);
                5.0 * thick * (0.2969 * x.sqrt() - 0.126 * x - 0.3516 * x * x + 0.2843 * x * x * x - 0.1036 * x * x * x * x)
            };
            for k in 0..n {
                let x = station(k);
                pts.push(le - v(0.0, 0.0, chord * x) + up * (half(x).max(0.004) * chord));
            }
            for k in (0..n).rev() {
                let x = station(k);
                pts.push(le - v(0.0, 0.0, chord * x) - up * (half(x).max(0.004) * chord));
            }
            pts
        };
        let rows = vec![section(s0), section(s1)];
        self.grid(&rows, true, false);
        let span = (p.tip - p.root).norm();
        self.cap(&rows[0], span * -1.0);
        self.cap(&rows[1], span);
    }

    /// Repeats what was built since `from` mirrored to the other side (x to -x), the moving parts
    /// swapped left for right.
    fn mirror(&mut self, from: (usize, usize)) {
        let (v0, i0) = from;
        let offset = self.mesh.vertices.len() as u32 - v0 as u32;
        let added: Vec<Vertex> = self.mesh.vertices[v0..].to_vec();
        for mut vx in added {
            vx.pos[0] = -vx.pos[0];
            vx.normal[0] = -vx.normal[0];
            vx.pivot[0] = -vx.pivot[0];
            // a turning axis mirrors as an axial vector
            vx.axis = [vx.axis[0], -vx.axis[1], -vx.axis[2]];
            vx.part = f32::from(swap_side(vx.part as u8));
            self.mesh.vertices.push(vx);
        }
        let tris: Vec<u32> = self.mesh.indices[i0..].to_vec();
        for t in tris.chunks(3) {
            self.mesh.indices.extend_from_slice(&[t[0] + offset, t[2] + offset, t[1] + offset]);
        }
    }

    fn mark(&self) -> (usize, usize) {
        (self.mesh.vertices.len(), self.mesh.indices.len())
    }
}

fn swap_side(p: u8) -> u8 {
    match p {
        part::AILERON_L => part::AILERON_R,
        part::AILERON_R => part::AILERON_L,
        part::STAB_L => part::STAB_R,
        part::STAB_R => part::STAB_L,
        part::RUDDER_L => part::RUDDER_R,
        part::RUDDER_R => part::RUDDER_L,
        other => other,
    }
}

/// The axis a hinge along `along` turns about so a positive angle moves the trailing edge
/// toward `-up` (down, for a wing).
fn hinge_axis(along: V, up: V) -> V {
    let want = up.cross(v(0.0, 0.0, -1.0));
    if along.dot(want) < 0.0 { along * -1.0 } else { along }
}

/// A point `s` along the span and `c` along the chord of a panel.
fn on_panel(p: &jet_sim::Panel, s: f64, c: f64) -> V {
    let le = p.root.lerp(p.tip, s);
    let chord = p.root_chord + (p.tip_chord - p.root_chord) * s;
    le - v(0.0, 0.0, chord * c)
}

/// The flight model's surfaces, drawn: wings with ailerons, all-moving tails, elevons, fins with
/// rudders.
fn surfaces(b: &mut Builder, spec: &Spec) {
    for shape in &spec.shapes {
        let Shape { panel: p, up, role, parts, mirror, thickness } = *shape;
        let start = b.mark();
        let left_part = if mirror { parts.1 } else { parts.0 };
        let trim = spec.trim;
        b.paint(spec.paint, mat::PAINT).still();
        match role {
            Role::Wing { ailerons } => {
                if ailerons {
                    b.foil(&p, up, thickness, 0.0, 0.76, 0.0, 1.0);
                    b.foil(&p, up, thickness, 0.76, 1.0, 0.0, 0.42);
                    b.foil(&p, up, thickness, 0.76, 1.0, 0.92, 1.0);
                    let (h0, h1) = (on_panel(&p, 0.42, 0.76), on_panel(&p, 0.92, 0.76));
                    b.moving(left_part, h0, hinge_axis(h1 - h0, up));
                    b.foil(&p, up, thickness, 0.76, 1.0, 0.42, 0.92);
                    b.still();
                } else {
                    b.foil(&p, up, thickness, 0.0, 1.0, 0.0, 1.0);
                }
                // a stripe of trim at the tip
                b.paint(trim, mat::PAINT);
                let tip = jet_sim::Panel { root: p.root.lerp(p.tip, 0.96), tip: p.tip, root_chord: p.root_chord + (p.tip_chord - p.root_chord) * 0.96, tip_chord: p.tip_chord };
                b.foil(&tip, up, thickness * 1.15, 0.0, 1.0, 0.0, 1.0);
                // navigation lights on the tips: red to the left, green to the right
                let light = on_panel(&p, 1.0, 0.45);
                let is_left = light.x > 0.0;
                b.paint([1.0, 0.1, 0.08], if is_left { mat::NAV_RED } else { mat::NAV_GREEN });
                b.blob(light + v(light.x.signum() * 0.04, 0.0, 0.0), v(0.09, 0.07, 0.16), 8);
            }
            Role::AllMoving => {
                let (h0, h1) = (on_panel(&p, 0.0, 0.35), on_panel(&p, 1.0, 0.35));
                b.moving(left_part, h0, hinge_axis(h1 - h0, up));
                b.foil(&p, up, thickness, 0.0, 1.0, 0.0, 1.0);
                b.still();
            }
            Role::Elevon => {
                let (h0, h1) = (on_panel(&p, 0.0, 0.0), on_panel(&p, 1.0, 0.0));
                b.moving(left_part, h0, hinge_axis(h1 - h0, up));
                b.foil(&p, up, thickness, 0.0, 1.0, 0.0, 1.0);
                b.still();
            }
            Role::Fin => {
                b.foil(&p, up, thickness, 0.0, 0.68, 0.0, 1.0);
                b.foil(&p, up, thickness, 0.68, 1.0, 0.0, 0.08);
                b.foil(&p, up, thickness, 0.68, 1.0, 0.88, 1.0);
                let (h0, h1) = (on_panel(&p, 0.08, 0.68), on_panel(&p, 0.88, 0.68));
                b.moving(left_part, h0, hinge_axis(h1 - h0, up));
                b.foil(&p, up, thickness, 0.68, 1.0, 0.08, 0.88);
                b.still();
                // a band of trim near the top, and the strobe on the very top
                b.paint(trim, mat::PAINT);
                let band = jet_sim::Panel {
                    root: p.root.lerp(p.tip, 0.72),
                    tip: p.root.lerp(p.tip, 0.86),
                    root_chord: p.root_chord + (p.tip_chord - p.root_chord) * 0.72,
                    tip_chord: p.root_chord + (p.tip_chord - p.root_chord) * 0.86,
                };
                b.foil(&band, up, thickness * 1.12, 0.0, 0.68, 0.0, 1.0);
                b.paint([1.0, 1.0, 1.0], mat::STROBE);
                b.blob(on_panel(&p, 1.0, 0.5) + p.tip.lerp(p.root, 0.0) * 0.0 + (p.tip - p.root).norm() * 0.06, v(0.07, 0.07, 0.1), 8);
            }
        }
        if mirror {
            b.mirror(start);
        }
    }
}

/// A missile or bomb on a pylon, or flying.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Store {
    pub pos: V,
    pub missile: bool,
}

/// The stores each jet carries where they can be seen, in the order they go.
pub fn stores(spec: &Spec) -> Vec<Store> {
    let wing = spec.shapes.iter().find(|s| matches!(s.role, Role::Wing { .. }));
    let Some(wing) = wing else {
        return Vec::new();
    };
    let p = &wing.panel;
    let under = |s: f64, c: f64, drop: f64| {
        let q = on_panel(p, s, c);
        v(q.x, q.y - drop, q.z)
    };
    let mirror = |q: V| v(-q.x, q.y, q.z);
    let mut out = Vec::new();
    let mut pairs = |q: V, missile: bool| {
        out.push(Store { pos: q, missile });
        out.push(Store { pos: mirror(q), missile });
    };
    match spec.name {
        "Falcon" => {
            pairs(under(1.0, 0.45, -0.02) + v(0.12, 0.0, 0.3), true);
            pairs(under(0.62, 0.35, 0.42), true);
            pairs(under(0.3, 0.3, 0.6), false);
        }
        "Viper" => {
            pairs(under(0.6, 0.35, 0.4), true);
            pairs(under(0.35, 0.3, 0.45), true);
            pairs(v(0.85, -0.85, 0.5), true);
        }
        "Talon" => {
            pairs(under(0.62, 0.3, 0.42), true);
            pairs(under(0.35, 0.25, 0.48), true);
        }
        _ => {}
    }
    out
}

fn missile(b: &mut Builder, at: V, scale: f64) {
    let len = 2.9 * scale;
    let r = 0.12 * scale;
    let (tail, nose) = (at - v(0.0, 0.0, len * 0.55), at + v(0.0, 0.0, len * 0.45));
    b.paint([0.88, 0.88, 0.86], mat::PAINT);
    b.tube(tail, nose - v(0.0, 0.0, len * 0.12), r, r, 10, false);
    b.paint([0.35, 0.36, 0.35], mat::METAL);
    b.tube(nose - v(0.0, 0.0, len * 0.12), nose, r, r * 0.12, 10, false);
    b.paint([0.95, 0.75, 0.15], mat::PAINT);
    b.tube(nose - v(0.0, 0.0, len * 0.2), nose - v(0.0, 0.0, len * 0.16), r * 1.02, r * 1.02, 10, false);
    b.paint([0.8, 0.8, 0.78], mat::PAINT);
    for k in 0..4 {
        let a = std::f64::consts::FRAC_PI_4 + k as f64 * std::f64::consts::FRAC_PI_2;
        let d = v(a.cos(), a.sin(), 0.0);
        let p0 = tail + d * r;
        let fin = jet_sim::Panel { root: p0 + v(0.0, 0.0, 0.35 * scale), tip: p0 + d * (0.22 * scale) + v(0.0, 0.0, 0.18 * scale), root_chord: 0.35 * scale, tip_chord: 0.15 * scale };
        b.foil(&fin, v(-d.y, d.x, 0.0), 0.08, 0.0, 1.0, 0.0, 1.0);
    }
    b.paint([0.2, 0.2, 0.2], mat::METAL);
    b.tube(tail, tail - v(0.0, 0.0, 0.05), r * 0.9, r * 0.7, 10, true);
}

fn bomb(b: &mut Builder, at: V, scale: f64) {
    let len = 2.2 * scale;
    let r = 0.25 * scale;
    b.paint([0.33, 0.36, 0.26], mat::PAINT);
    let rings = [
        ring(len * 0.5, 0.0, 0.02, 0.02, 0.02, 2.0),
        ring(len * 0.4, 0.0, r * 0.75, r * 0.75, r * 0.75, 2.0),
        ring(len * 0.2, 0.0, r, r, r, 2.0),
        ring(-len * 0.15, 0.0, r, r, r, 2.0),
        ring(-len * 0.45, 0.0, r * 0.45, r * 0.45, r * 0.45, 2.0),
    ];
    let moved: Vec<Ring> = rings.iter().map(|q| Ring { z: q.z + at.z, x: at.x, y: at.y, ..*q }).collect();
    b.loft(&moved, 12, false, true);
    b.paint([0.95, 0.8, 0.1], mat::PAINT);
    b.tube(at + v(0.0, 0.0, len * 0.28), at + v(0.0, 0.0, len * 0.24), r * 0.96, r * 0.98, 12, false);
    b.paint([0.3, 0.32, 0.24], mat::PAINT);
    for k in 0..4 {
        let a = std::f64::consts::FRAC_PI_4 + k as f64 * std::f64::consts::FRAC_PI_2;
        let d = v(a.cos(), a.sin(), 0.0);
        let p0 = at + v(0.0, 0.0, -len * 0.3) + d * (r * 0.4);
        let fin = jet_sim::Panel { root: p0, tip: p0 + d * (r * 0.9), root_chord: 0.5 * scale, tip_chord: 0.4 * scale };
        b.foil(&fin, v(-d.y, d.x, 0.0), 0.06, 0.0, 1.0, 0.0, 1.0);
    }
}

/// A flying missile or bomb, nose along +z.
pub fn ordnance(missile_shape: bool) -> Mesh {
    let mut b = Builder::new();
    if missile_shape {
        missile(&mut b, V::ZERO, 1.0);
    } else {
        bomb(&mut b, V::ZERO, 1.0);
    }
    b.mesh
}

/// Landing gear: a leg up to the belly and a wheel (two for the big jet), folding back.
fn gear(b: &mut Builder, bottom: V, belly: f64, wheel: f64, twin: bool) {
    let center = bottom + v(0.0, wheel, 0.0);
    let top = v(bottom.x, belly, bottom.z + 0.15);
    b.moving(part::GEAR, top, v(1.0, 0.0, 0.0));
    b.paint([0.75, 0.76, 0.78], mat::METAL);
    b.tube(top, center + v(0.0, wheel * 0.3, 0.0), 0.075, 0.07, 8, true);
    b.tube(center + v(0.0, wheel * 0.3, 0.0), center, 0.1, 0.1, 8, true);
    let offsets: &[f64] = if twin { &[-0.28, 0.28] } else { &[0.0] };
    for &o in offsets {
        let c = center + v(o, 0.0, 0.0);
        let w = 0.11 * if twin { 1.2 } else { 1.0 };
        b.paint([0.08, 0.08, 0.08], mat::RUBBER);
        b.tube(c - v(w, 0.0, 0.0), c + v(w, 0.0, 0.0), wheel, wheel, 14, true);
        b.paint([0.6, 0.6, 0.62], mat::METAL);
        b.tube(c - v(w + 0.01, 0.0, 0.0), c + v(w + 0.01, 0.0, 0.0), wheel * 0.55, wheel * 0.55, 10, true);
    }
    b.still();
}

/// A nozzle at `at` (its exit), `r` across: a petalled cone, dark outside, glowing inside.
fn nozzle(b: &mut Builder, at: V, r: f64, len: f64) {
    b.paint([0.3, 0.29, 0.28], mat::METAL);
    b.tube(at + v(0.0, 0.0, len), at, r * 1.08, r, 16, false);
    b.paint([0.16, 0.15, 0.15], mat::METAL);
    b.tube(at, at + v(0.0, 0.0, len * 0.6), r * 0.95, r * 0.8, 16, false);
    b.paint([1.0, 0.5, 0.2], mat::HOT);
    b.tube(at + v(0.0, 0.0, len * 0.6), at + v(0.0, 0.0, len * 0.62), r * 0.8, r * 0.05, 16, false);
}

/// An intake's dark mouth inside a hull section.
fn intake(b: &mut Builder, r: Ring) {
    b.paint([0.04, 0.04, 0.05], mat::INTAKE);
    let inner = Ring { w: r.w * 0.85, top: r.top * 0.8, bottom: r.bottom * 0.8, z: r.z - 0.05, ..r };
    let deep = Ring { z: r.z - 1.2, ..inner };
    b.loft(&[inner, deep], 16, false, false);
    b.cap(&(0..16).map(|j| deep.point(j as f64 / 16.0 * std::f64::consts::TAU)).collect::<Vec<_>>(), v(0.0, 0.0, 1.0));
}

/// The pilot: a helmet with a dark visor, and the seat's headrest behind.
fn pilot(b: &mut Builder, eye: V) {
    b.moving(part::PILOT, eye, v(1.0, 0.0, 0.0));
    b.paint([0.85, 0.85, 0.82], mat::PAINT);
    b.blob(eye + v(0.0, 0.04, -0.06), v(0.15, 0.16, 0.17), 12);
    b.paint([0.12, 0.1, 0.06], mat::GLASS);
    b.blob(eye + v(0.0, 0.0, 0.04), v(0.13, 0.08, 0.12), 12);
    b.paint([0.18, 0.18, 0.2], mat::INTERIOR);
    b.cuboid(eye + v(-0.22, -0.9, -0.42), eye + v(0.22, 0.25, -0.24));
    b.still();
}

/// The cockpit round the pilot's eyes (at `eye`): the tub, the instrument panel with three
/// screens, the glass plate of the heads-up display on top of it, consoles, the stick, the
/// throttle and the canopy's frame.
fn cockpit(b: &mut Builder, eye: V, width: f64) {
    let e = |x: f64, y: f64, z: f64| eye + v(x, y, z);
    let w = width;
    b.paint([0.16, 0.17, 0.19], mat::INTERIOR);
    // the tub's walls and floor
    b.quad([e(w, -0.95, -0.7), e(w, -0.95, 0.95), e(w, -0.2, 0.95), e(w, -0.2, -0.7)], v(-1.0, 0.0, 0.0));
    b.quad([e(-w, -0.95, 0.95), e(-w, -0.95, -0.7), e(-w, -0.2, -0.7), e(-w, -0.2, 0.95)], v(1.0, 0.0, 0.0));
    b.quad([e(-w, -0.95, -0.7), e(w, -0.95, -0.7), e(w, -0.95, 0.95), e(-w, -0.95, 0.95)], V::UP);
    // the instrument panel, leaning back, and the glare shield over it
    let (pb, pt) = (e(0.0, -0.78, 0.66), e(0.0, -0.3, 0.6));
    let panel_n = v(0.0, 0.12, -1.0).norm();
    b.quad([pb + v(-w * 0.92, 0.0, 0.0), pb + v(w * 0.92, 0.0, 0.0), pt + v(w * 0.92, 0.0, 0.0), pt + v(-w * 0.92, 0.0, 0.0)], panel_n);
    b.paint([0.07, 0.07, 0.08], mat::INTERIOR);
    b.quad([pt + v(-w * 0.95, 0.0, -0.04), pt + v(w * 0.95, 0.0, -0.04), e(w * 0.8, -0.24, 1.05), e(-w * 0.8, -0.24, 1.05)], V::UP);
    // three screens: the left one (on +x) engines and weapons, the middle attitude, the right radar
    let screen_w = w * 0.38;
    for (k, id) in [(1.0, 2.0), (0.0, 0.0), (-1.0, 1.0)] {
        let cx = k * w * 0.6;
        let lift = 0.006;
        // (panel_n faces the pilot: the screens stand just proud of the panel)
        let c0 = pb.lerp(pt, 0.12) + v(cx, 0.0, 0.0) + panel_n * lift;
        let c1 = pb.lerp(pt, 0.82) + v(cx, 0.0, 0.0) + panel_n * lift;
        b.paint([0.05, 0.05, 0.06], mat::BUTTONS);
        let bezel = 0.035;
        b.quad(
            [c0 + v(screen_w * 0.5 + bezel, -bezel, 0.0), c0 + v(-screen_w * 0.5 - bezel, -bezel, 0.0), c1 + v(-screen_w * 0.5 - bezel, bezel, 0.0), c1 + v(screen_w * 0.5 + bezel, bezel, 0.0)],
            panel_n,
        );
        b.material = mat::SCREEN;
        let s = panel_n * 0.004;
        b.screen([c0 + v(screen_w * 0.5, 0.0, 0.0) + s, c0 + v(-screen_w * 0.5, 0.0, 0.0) + s, c1 + v(-screen_w * 0.5, 0.0, 0.0) + s, c1 + v(screen_w * 0.5, 0.0, 0.0) + s], panel_n, id);
    }
    // the heads-up display: a projector on the glare shield, two posts and the tilted glass
    b.paint([0.1, 0.1, 0.11], mat::METAL);
    b.cuboid(e(-0.12, -0.3, 0.62), e(0.12, -0.22, 0.8));
    let (g0, g1) = (e(0.0, -0.22, 0.7), e(0.0, 0.1, 0.64));
    b.tube(g0 + v(0.17, 0.0, 0.0), g1 + v(0.17, 0.0, 0.0), 0.012, 0.012, 6, true);
    b.tube(g0 + v(-0.17, 0.0, 0.0), g1 + v(-0.17, 0.0, 0.0), 0.012, 0.012, 6, true);
    b.paint([0.3, 0.9, 0.5], mat::HUD);
    let n = (g1 - g0).cross(v(1.0, 0.0, 0.0)).norm();
    b.quad([g0 + v(0.16, 0.0, 0.0), g0 + v(-0.16, 0.0, 0.0), g1 + v(-0.16, 0.0, 0.0), g1 + v(0.16, 0.0, 0.0)], n * -1.0);
    // side consoles with lit switches
    for side in [1.0, -1.0] {
        let x0 = side * (w - 0.02);
        let x1 = side * (w - 0.24);
        b.paint([0.14, 0.15, 0.17], mat::INTERIOR);
        b.cuboid(e(x0.min(x1), -0.95, -0.5), e(x0.max(x1), -0.55, 0.45));
        b.paint([0.1, 0.1, 0.1], mat::BUTTONS);
        b.quad([e(x0, -0.545, -0.45), e(x1, -0.545, -0.45), e(x1, -0.545, 0.4), e(x0, -0.545, 0.4)], V::UP);
    }
    // the throttle on the left console, the stick between the knees
    let throttle_base = e(w - 0.13, -0.55, 0.0);
    b.moving(part::THROTTLE, throttle_base, v(1.0, 0.0, 0.0));
    b.paint([0.08, 0.08, 0.08], mat::METAL);
    b.tube(throttle_base, throttle_base + v(0.0, 0.2, 0.06), 0.022, 0.022, 6, true);
    b.cuboid(throttle_base + v(-0.04, 0.17, 0.02), throttle_base + v(0.05, 0.27, 0.13));
    let stick_base = e(0.0, -0.98, 0.32);
    b.moving(part::STICK_PITCH, stick_base, v(1.0, 0.0, 0.0));
    b.tube(stick_base, stick_base + v(0.0, 0.32, 0.0), 0.025, 0.022, 8, true);
    b.paint([0.1, 0.1, 0.1], mat::METAL);
    b.blob(stick_base + v(0.0, 0.38, 0.0), v(0.04, 0.08, 0.045), 8);
    b.paint([0.85, 0.15, 0.1], mat::BUTTONS);
    b.blob(stick_base + v(0.0, 0.46, 0.01), v(0.015, 0.012, 0.015), 6);
    b.still();
}

/// The canopy's frame: rails along the sides of the glass (the glass itself is frameless in
/// front, so nothing crosses the pilot's view).
fn canopy_frame(b: &mut Builder, bow: Ring, rail_from: f64, rail_to: f64) {
    b.paint([0.12, 0.12, 0.13], mat::METAL);
    for side in [1.0, -1.0] {
        // along the cockpit's sill, stopping short of the instrument panel
        let x = side * (bow.w.max(0.5) + 0.08);
        b.tube(v(x, bow.y, rail_from), v(x, bow.y, rail_to - 0.7), 0.03, 0.03, 6, true);
    }
}

/// The opening in the hull the cockpit sits in, which the shader cuts away: half its width, the
/// height of the canopy's sill, and its back and front, in the body frame.
pub fn cockpit_cut(kind: usize) -> [f32; 4] {
    match kind {
        jet_sim::FALCON => [0.5, 0.62, 1.8, 4.9],
        jet_sim::SPECTER => [0.5, 0.5, 2.6, 6.6],
        jet_sim::VIPER => [0.5, 0.62, 1.8, 5.2],
        jet_sim::TALON => [0.5, 0.62, 3.2, 6.8],
        _ => [0.7, 1.0, 5.3, 8.4],
    }
}

/// The model of jet `kind`.
pub fn jet(kind: usize) -> Mesh {
    let spec = &jet_sim::specs()[kind];
    let mut b = Builder::new();
    let paint = spec.paint;
    let dark = [paint[0] * 0.55, paint[1] * 0.55, paint[2] * 0.55];
    let glass = if kind == jet_sim::SPECTER { [0.9, 0.7, 0.3] } else { [0.35, 0.45, 0.55] };
    surfaces(&mut b, spec);
    match kind {
        jet_sim::FALCON => {
            b.paint([0.25, 0.26, 0.28], mat::PAINT);
            b.loft(&[ring(7.5, 0.05, 0.02, 0.02, 0.02, 2.0), ring(7.0, 0.05, 0.22, 0.22, 0.22, 2.0), ring(6.0, 0.08, 0.45, 0.42, 0.4, 2.2)], 20, false, false);
            b.paint(paint, mat::PAINT);
            b.loft(
                &[
                    ring(6.0, 0.08, 0.45, 0.42, 0.4, 2.2),
                    ring(5.0, 0.12, 0.62, 0.56, 0.55, 2.4),
                    ring(4.0, 0.12, 0.78, 0.66, 0.7, 2.6),
                    ring(2.5, 0.1, 0.92, 0.72, 0.8, 2.8),
                    ring(1.0, 0.05, 1.0, 0.8, 0.85, 3.0),
                    ring(-1.0, 0.05, 1.02, 0.84, 0.82, 3.0),
                    ring(-3.5, 0.05, 0.95, 0.82, 0.75, 2.8),
                    ring(-5.5, 0.05, 0.82, 0.75, 0.7, 2.5),
                    ring(-6.8, 0.05, 0.68, 0.66, 0.66, 2.2),
                    ring(-7.2, 0.05, 0.62, 0.62, 0.62, 2.0),
                ],
                24,
                false,
                false,
            );
            // the chin intake under the cockpit
            let mouth = ring(4.2, -0.78, 0.52, 0.3, 0.32, 4.0);
            b.paint(paint, mat::PAINT);
            b.loft(&[mouth, ring(3.0, -0.8, 0.56, 0.3, 0.33, 4.0), ring(0.0, -0.72, 0.55, 0.25, 0.28, 4.0), ring(-2.0, -0.55, 0.42, 0.18, 0.18, 4.0)], 16, false, false);
            intake(&mut b, mouth);
            b.paint(glass, mat::GLASS);
            b.bubble(&[ring(5.3, 0.6, 0.05, 0.03, 0.0, 2.0), ring(4.7, 0.6, 0.42, 0.45, 0.0, 2.0), ring(3.8, 0.66, 0.56, 0.7, 0.0, 2.0), ring(2.9, 0.68, 0.55, 0.62, 0.0, 2.0), ring(2.0, 0.66, 0.42, 0.38, 0.0, 2.0), ring(1.4, 0.62, 0.1, 0.08, 0.0, 2.0)], 16);
            canopy_frame(&mut b, ring(4.75, 0.6, 0.43, 0.46, 0.0, 2.0), 1.6, 4.7);
            // a spine behind the canopy
            b.paint(paint, mat::PAINT);
            b.bubble(&[ring(1.6, 0.6, 0.3, 0.25, 0.0, 2.0), ring(-1.0, 0.75, 0.4, 0.28, 0.0, 2.2), ring(-4.5, 0.75, 0.3, 0.15, 0.0, 2.2), ring(-5.5, 0.72, 0.05, 0.02, 0.0, 2.0)], 12);
            // the airbrake behind the spine
            b.moving(part::AIRBRAKE, v(0.0, 0.84, -4.6), v(1.0, 0.0, 0.0));
            b.paint(dark, mat::PAINT);
            b.cuboid(v(-0.35, 0.8, -5.7), v(0.35, 0.86, -4.6));
            b.still();
            nozzle(&mut b, v(0.0, 0.05, -8.0), 0.6, 0.8);
            for g in spec.gear {
                gear(&mut b, g, -0.75, 0.33, false);
            }
        }
        jet_sim::SPECTER => {
            // a chined, faceted hull
            b.paint([0.12, 0.13, 0.14], mat::PAINT);
            b.loft(&[ring(9.5, 0.0, 0.02, 0.02, 0.02, 1.2), ring(8.0, 0.05, 0.55, 0.25, 0.25, 1.2)], 10, true, false);
            b.paint(paint, mat::PAINT);
            b.loft(
                &[
                    ring(8.0, 0.05, 0.55, 0.25, 0.25, 1.2),
                    ring(6.5, 0.08, 1.0, 0.45, 0.42, 1.3),
                    ring(5.0, 0.1, 1.35, 0.6, 0.55, 1.4),
                    ring(3.0, 0.1, 1.75, 0.72, 0.65, 1.4),
                    ring(0.0, 0.05, 2.0, 0.78, 0.7, 1.5),
                    ring(-3.0, 0.05, 2.0, 0.74, 0.65, 1.5),
                    ring(-6.0, 0.05, 1.85, 0.6, 0.55, 1.4),
                    ring(-8.4, 0.05, 1.6, 0.45, 0.42, 1.3),
                    ring(-8.9, 0.05, 1.5, 0.36, 0.35, 1.3),
                ],
                10,
                true,
                true,
            );
            // caret intakes on the flanks
            for side in [1.0, -1.0] {
                let start = b.mark();
                let mouth = ring(3.6, -0.05, 0.38, 0.42, 0.42, 1.4).at(side * 1.55);
                b.paint(paint, mat::PAINT);
                b.loft(&[mouth, ring(1.0, 0.0, 0.42, 0.45, 0.45, 1.4).at(side * 1.6), ring(-2.0, 0.05, 0.3, 0.3, 0.3, 1.4).at(side * 1.5)], 8, true, false);
                intake(&mut b, mouth);
                let _ = start;
            }
            b.paint(glass, mat::GLASS);
            b.bubble(&[ring(7.0, 0.42, 0.05, 0.02, 0.0, 2.0), ring(6.0, 0.48, 0.45, 0.45, 0.0, 2.0), ring(5.0, 0.6, 0.55, 0.72, 0.0, 2.0), ring(3.8, 0.62, 0.52, 0.62, 0.0, 2.0), ring(2.8, 0.58, 0.32, 0.3, 0.0, 2.0), ring(2.2, 0.55, 0.05, 0.04, 0.0, 2.0)], 12);
            canopy_frame(&mut b, ring(5.95, 0.48, 0.46, 0.46, 0.0, 2.0), 2.3, 5.9);
            // flat, shielded exhausts
            for x in [1.0, -1.0] {
                b.paint([0.15, 0.15, 0.16], mat::METAL);
                b.cuboid(v(x - 0.62, -0.25, -9.7), v(x + 0.62, 0.42, -8.8));
                b.paint([1.0, 0.45, 0.2], mat::HOT);
                b.quad([v(x - 0.5, -0.15, -9.72), v(x + 0.5, -0.15, -9.72), v(x + 0.5, 0.3, -9.72), v(x - 0.5, 0.3, -9.72)], v(0.0, 0.0, -1.0));
            }
            // the weapons bay doors under the belly
            for side in [1.0, -1.0] {
                b.moving(part::BAY, v(side * 0.95, -0.72, 0.0), v(0.0, 0.0, side));
                b.paint(dark, mat::PAINT);
                b.cuboid(v(if side > 0.0 { 0.02 } else { -0.95 }, -0.76, -2.0), v(if side > 0.0 { 0.95 } else { -0.02 }, -0.7, 3.0));
                b.still();
            }
            b.paint([0.05, 0.05, 0.05], mat::INTAKE);
            b.cuboid(v(-0.9, -0.69, -1.9), v(0.9, -0.66, 2.9));
            for g in spec.gear {
                gear(&mut b, g, -0.7, 0.36, false);
            }
        }
        jet_sim::VIPER => {
            b.paint([0.3, 0.31, 0.33], mat::PAINT);
            b.loft(&[ring(8.0, 0.0, 0.02, 0.02, 0.02, 2.0), ring(7.2, 0.0, 0.2, 0.2, 0.2, 2.0), ring(6.2, 0.04, 0.38, 0.36, 0.34, 2.2)], 20, false, false);
            b.paint(paint, mat::PAINT);
            b.loft(
                &[
                    ring(6.2, 0.04, 0.38, 0.36, 0.34, 2.2),
                    ring(4.8, 0.1, 0.6, 0.56, 0.55, 2.4),
                    ring(3.5, 0.1, 0.74, 0.65, 0.7, 2.6),
                    ring(1.5, 0.05, 0.85, 0.7, 0.75, 2.8),
                    ring(-1.5, 0.05, 0.9, 0.74, 0.72, 2.8),
                    ring(-4.5, 0.05, 0.85, 0.72, 0.68, 2.6),
                    ring(-6.8, 0.05, 0.7, 0.64, 0.62, 2.2),
                    ring(-7.6, 0.05, 0.62, 0.6, 0.6, 2.0),
                ],
                24,
                false,
                false,
            );
            for side in [1.0, -1.0] {
                let mouth = ring(3.0, -0.05, 0.32, 0.42, 0.42, 2.6).at(side * 0.98);
                b.paint(paint, mat::PAINT);
                b.loft(&[mouth, ring(1.0, 0.0, 0.36, 0.45, 0.45, 2.6).at(side * 0.98), ring(-1.5, 0.05, 0.2, 0.3, 0.3, 2.6).at(side * 0.85)], 14, false, false);
                intake(&mut b, mouth);
            }
            b.paint(glass, mat::GLASS);
            b.bubble(&[ring(5.6, 0.58, 0.05, 0.03, 0.0, 2.0), ring(4.9, 0.6, 0.42, 0.46, 0.0, 2.0), ring(4.0, 0.65, 0.52, 0.66, 0.0, 2.0), ring(3.0, 0.66, 0.48, 0.55, 0.0, 2.0), ring(2.0, 0.62, 0.3, 0.26, 0.0, 2.0), ring(1.4, 0.58, 0.06, 0.04, 0.0, 2.0)], 16);
            canopy_frame(&mut b, ring(4.85, 0.6, 0.43, 0.47, 0.0, 2.0), 1.5, 4.8);
            b.paint(paint, mat::PAINT);
            b.bubble(&[ring(1.5, 0.6, 0.25, 0.2, 0.0, 2.0), ring(-2.0, 0.75, 0.35, 0.22, 0.0, 2.2), ring(-4.0, 0.72, 0.05, 0.02, 0.0, 2.0)], 12);
            b.paint(spec.trim, mat::PAINT);
            b.tube(v(0.0, 0.05, 6.3), v(0.0, 0.05, 6.1), 0.385, 0.4, 24, false);
            nozzle(&mut b, v(0.0, 0.05, -8.4), 0.6, 0.85);
            for g in spec.gear {
                gear(&mut b, g, -0.7, 0.34, false);
            }
        }
        jet_sim::TALON => {
            b.paint([0.22, 0.25, 0.3], mat::PAINT);
            b.loft(&[ring(9.5, 0.0, 0.02, 0.02, 0.02, 2.0), ring(8.6, 0.05, 0.26, 0.26, 0.26, 2.0), ring(7.4, 0.08, 0.52, 0.46, 0.44, 2.2)], 20, false, false);
            b.paint(paint, mat::PAINT);
            b.loft(
                &[
                    ring(7.4, 0.08, 0.52, 0.46, 0.44, 2.2),
                    ring(6.2, 0.12, 0.74, 0.62, 0.6, 2.4),
                    ring(4.8, 0.12, 0.9, 0.7, 0.7, 2.6),
                    ring(3.0, 0.1, 1.25, 0.72, 0.7, 3.0),
                    ring(1.0, 0.05, 1.9, 0.68, 0.62, 3.5),
                    ring(-2.0, 0.05, 2.1, 0.6, 0.55, 3.5),
                    ring(-5.0, 0.05, 2.0, 0.52, 0.5, 3.5),
                    ring(-7.5, 0.05, 1.7, 0.45, 0.45, 3.0),
                ],
                24,
                false,
                true,
            );
            // the engines, slung under the body and spread apart
            for side in [1.0, -1.0] {
                let x = side * 1.25;
                let mouth = ring(2.6, -0.45, 0.55, 0.45, 0.45, 3.2).at(x);
                b.paint(paint, mat::PAINT);
                b.loft(&[mouth, ring(0.0, -0.3, 0.62, 0.55, 0.55, 2.8).at(x), ring(-5.0, -0.1, 0.66, 0.6, 0.6, 2.4).at(x), ring(-8.6, 0.0, 0.64, 0.62, 0.62, 2.0).at(x)], 18, false, false);
                intake(&mut b, mouth);
                let exit = v(x, 0.0, -9.4);
                b.moving(part::NOZZLE, v(x, 0.0, -8.6), v(-1.0, 0.0, 0.0));
                nozzle(&mut b, exit, 0.6, 0.8);
                b.still();
            }
            b.paint(glass, mat::GLASS);
            b.bubble(&[ring(7.2, 0.6, 0.05, 0.03, 0.0, 2.0), ring(6.5, 0.62, 0.44, 0.46, 0.0, 2.0), ring(5.5, 0.66, 0.56, 0.72, 0.0, 2.0), ring(4.5, 0.66, 0.52, 0.6, 0.0, 2.0), ring(3.5, 0.62, 0.32, 0.28, 0.0, 2.0), ring(2.8, 0.58, 0.06, 0.04, 0.0, 2.0)], 16);
            canopy_frame(&mut b, ring(6.45, 0.62, 0.45, 0.47, 0.0, 2.0), 2.9, 6.4);
            b.moving(part::AIRBRAKE, v(0.0, 0.62, -1.0), v(1.0, 0.0, 0.0));
            b.paint(dark, mat::PAINT);
            b.cuboid(v(-0.5, 0.58, -2.4), v(0.5, 0.64, -1.0));
            b.still();
            for g in spec.gear {
                gear(&mut b, g, -0.75, 0.36, false);
            }
        }
        _ => {
            // the Leviathan: a thick centre body blending into the wing
            b.paint(paint, mat::PAINT);
            b.loft(
                &[
                    ring(10.4, 0.0, 0.05, 0.05, 0.05, 2.0),
                    ring(9.4, 0.1, 1.3, 0.6, 0.45, 2.2),
                    ring(6.5, 0.2, 2.5, 1.25, 0.85, 2.4),
                    ring(2.0, 0.2, 2.9, 1.4, 0.95, 2.6),
                    ring(-3.0, 0.1, 2.9, 1.1, 0.85, 2.6),
                    ring(-6.5, 0.05, 2.7, 0.7, 0.55, 2.4),
                    ring(-8.4, 0.0, 2.4, 0.25, 0.25, 2.0),
                ],
                28,
                false,
                true,
            );
            b.paint(glass, mat::GLASS);
            b.bubble(&[ring(8.8, 0.9, 0.08, 0.03, 0.0, 2.0), ring(7.8, 1.05, 0.95, 0.45, 0.0, 2.4), ring(6.6, 1.25, 1.1, 0.62, 0.0, 2.4), ring(5.6, 1.3, 0.9, 0.42, 0.0, 2.4), ring(5.0, 1.3, 0.1, 0.05, 0.0, 2.0)], 16);
            canopy_frame(&mut b, ring(7.75, 1.05, 0.96, 0.46, 0.0, 2.4), 5.0, 7.7);
            // engine humps on top, intakes forward and slot exhausts aft
            for e in &spec.engines {
                let mouth = ring(1.5, e.y - 0.1, 0.75, 0.42, 0.25, 3.0).at(e.x);
                b.paint(paint, mat::PAINT);
                b.loft(&[mouth, ring(-1.5, e.y, 0.82, 0.5, 0.35, 2.8).at(e.x), ring(e.z, e.y - 0.15, 0.85, 0.28, 0.25, 3.2).at(e.x)], 16, false, false);
                intake(&mut b, mouth);
                b.paint([1.0, 0.45, 0.2], mat::HOT);
                b.quad([v(e.x + 0.75, e.y - 0.3, e.z - 0.01), v(e.x - 0.75, e.y - 0.3, e.z - 0.01), v(e.x - 0.75, e.y - 0.05, e.z - 0.01), v(e.x + 0.75, e.y - 0.05, e.z - 0.01)], v(0.0, 0.0, -1.0));
            }
            for side in [1.0, -1.0] {
                b.moving(part::BAY, v(side * 1.6, -0.86, 0.0), v(0.0, 0.0, side));
                b.paint(dark, mat::PAINT);
                b.cuboid(v(if side > 0.0 { 0.02 } else { -1.6 }, -0.92, -3.5), v(if side > 0.0 { 1.6 } else { -0.02 }, -0.86, 3.0));
                b.still();
            }
            b.paint([0.05, 0.05, 0.05], mat::INTAKE);
            b.cuboid(v(-1.55, -0.85, -3.4), v(1.55, -0.82, 2.9));
            for (i, g) in spec.gear.iter().enumerate() {
                gear(&mut b, *g, -0.85, if i == 0 { 0.45 } else { 0.55 }, i > 0);
            }
        }
    }
    // the stores on their pylons, each its own part so it goes when fired
    for (i, s) in stores(spec).iter().enumerate() {
        let p = part::STORES + i as u8;
        b.moving(p, s.pos, v(1.0, 0.0, 0.0));
        if s.missile {
            missile(&mut b, s.pos, 1.0);
        } else {
            bomb(&mut b, s.pos, 1.0);
        }
        b.paint(dark, mat::PAINT);
        b.cuboid(s.pos + v(-0.04, 0.12, -0.5), s.pos + v(0.04, 0.42, 0.5));
        b.still();
    }
    pilot(&mut b, spec.eye);
    cockpit(&mut b, spec.eye, if kind == jet_sim::LEVIATHAN { 0.75 } else { 0.55 });
    b.mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_jet_has_a_sound_model() {
        for kind in 0..jet_sim::COUNT {
            let m = jet(kind);
            assert!(m.vertices.len() > 2000, "{kind}: {} vertices", m.vertices.len());
            assert_eq!(m.indices.len() % 3, 0);
            assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
            assert!(m.vertices.iter().all(|vx| vx.pos.iter().chain(&vx.normal).all(|f| f.is_finite())));
            let spec = &jet_sim::specs()[kind];
            let reach = m.vertices.iter().map(|vx| vx.pos[0].abs()).fold(0.0f32, f32::max);
            assert!((reach - spec.span as f32 / 2.0).abs() < 1.5, "{}: half span {reach}", spec.name);
            println!("{}: {} triangles, {} stores", spec.name, m.indices.len() / 3, stores(spec).len());
        }
    }

    #[test]
    fn a_hinge_turns_the_trailing_edge_down() {
        let axis = hinge_axis(v(1.0, 0.0, 0.0), V::UP);
        // a point behind the hinge, turned a little about the axis, goes down
        let q = jet_sim::Q::axis_angle(axis, 0.2);
        assert!(q.rotate(v(0.0, 0.0, -1.0)).y < 0.0);
        let axis = hinge_axis(v(-1.0, 0.0, 0.0), V::UP);
        assert!(jet_sim::Q::axis_angle(axis, 0.2).rotate(v(0.0, 0.0, -1.0)).y < 0.0);
    }
}
