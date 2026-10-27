//! Horizontal alignments from polylines: the element table (straights, arcs
//! and, where transitions are given, clothoid spirals) with their chainages,
//! points at a chainage and offset, the chainage and offset of a point, and
//! set-out points at intervals and offsets.
//!
//! A transition belongs to an arc of the polyline with straights on both
//! sides: the arc's radius and the two straights (meeting at their
//! intersection point) define a spiral–arc–spiral that replaces the plain
//! arc, moving its tangent points back along the straights.

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::angle::Bearing;
use crate::arc::{Arc, bulge_segments};
use crate::spiral::{clothoid, fit};

/// Spirals into and out of one arc of an alignment's polyline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    /// The arc's segment of the polyline (from 0).
    pub segment: usize,
    /// Spiral lengths before and after the arc (0 for none).
    pub entry: f64,
    pub exit: f64,
}

/// One element of an alignment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Element {
    pub start_chainage: f64,
    pub length: f64,
    pub start: DVec3,
    pub end: DVec3,
    pub kind: ElementKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ElementKind {
    Straight {
        bearing: Bearing,
    },
    Arc {
        radius: f64,
        /// Turns right (clockwise).
        right: bool,
        centre: DVec2,
        /// Bearing of the tangent at the start.
        start_bearing: Bearing,
        /// Central angle (radians, positive).
        delta: f64,
    },
    /// A clothoid between a straight and an arc of `radius`.
    Spiral {
        radius: f64,
        right: bool,
        /// From the straight into the arc (TS to SC), else out of it (CS to ST).
        entering: bool,
        /// Bearing of the tangent at its straight end (the TS or the ST).
        bearing: Bearing,
    },
}

impl Element {
    pub fn end_chainage(&self) -> f64 {
        self.start_chainage + self.length
    }

    /// Plan position and unit direction of travel `s` along the element.
    fn at(&self, s: f64) -> (DVec2, DVec2) {
        let s = s.clamp(0.0, self.length);
        match self.kind {
            ElementKind::Straight { bearing } => {
                let u = bearing.unit_vector();
                (self.start.truncate() + u * s, u)
            }
            ElementKind::Arc {
                radius,
                right,
                centre,
                start_bearing,
                ..
            } => {
                let turn = if right { -s / radius } else { s / radius };
                let r0 = self.start.truncate() - centre;
                (
                    centre + DVec2::from_angle(turn).rotate(r0),
                    DVec2::from_angle(turn).rotate(start_bearing.unit_vector()),
                )
            }
            ElementKind::Spiral {
                radius,
                right,
                entering,
                bearing,
            } => {
                let side = if right { -1.0 } else { 1.0 };
                let u = bearing.unit_vector();
                let n = DVec2::new(-u.y, u.x) * side;
                let theta = |l: f64| l * l / (2.0 * radius * self.length);
                if entering {
                    let c = clothoid(s, radius, self.length);
                    (
                        self.start.truncate() + u * c.x + n * c.y,
                        DVec2::from_angle(side * theta(s)).rotate(u),
                    )
                } else {
                    let t = self.length - s;
                    let c = clothoid(t, radius, self.length);
                    (
                        self.end.truncate() - u * c.x + n * c.y,
                        DVec2::from_angle(-side * theta(t)).rotate(u),
                    )
                }
            }
        }
    }

    fn height(&self, s: f64) -> f64 {
        if self.length <= 0.0 {
            return self.start.z;
        }
        self.start.z + (self.end.z - self.start.z) * (s / self.length).clamp(0.0, 1.0)
    }

    /// The radius at the arc end of a spiral, or of an arc.
    pub fn radius(&self) -> Option<f64> {
        match self.kind {
            ElementKind::Arc { radius, .. } | ElementKind::Spiral { radius, .. } => Some(radius),
            ElementKind::Straight { .. } => None,
        }
    }
}

/// An alignment's elements, laid end to end from its start chainage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    pub elements: Vec<Element>,
    pub start_chainage: f64,
}

/// A straight or arc of the polyline, before transitions are fitted.
enum Raw {
    Straight(DVec3, DVec3),
    Arc(Arc, DVec3, DVec3),
}

fn line_intersection(p: DVec2, u: DVec2, q: DVec2, v: DVec2) -> Option<DVec2> {
    let cross = u.perp_dot(v);
    (cross.abs() > 1e-12).then(|| p + u * ((q - p).perp_dot(v) / cross))
}

fn bearing_of(v: DVec2) -> Bearing {
    Bearing::from_delta(v.x, v.y).unwrap_or_default()
}

impl Geometry {
    /// The alignment of a polyline from `start_chainage`, with `transitions`
    /// on its arcs.
    pub fn new(
        vertices: &[DVec3],
        bulges: &[f64],
        closed: bool,
        start_chainage: f64,
        transitions: &[Transition],
    ) -> Result<Self, String> {
        let raw: Vec<Raw> = bulge_segments(vertices, bulges, closed)
            .map(|(a, b, bulge)| match Arc::from_bulge(a, b, bulge) {
                Some(arc) => Raw::Arc(arc, a, b),
                None => Raw::Straight(a, b),
            })
            .collect();
        let n = raw.len();
        // Straights with ends moved to the transitions' TS and ST.
        let mut ends: Vec<(DVec3, DVec3)> = raw
            .iter()
            .map(|r| match r {
                Raw::Straight(a, b) | Raw::Arc(_, a, b) => (*a, *b),
            })
            .collect();
        let mut fitted: Vec<Option<(crate::spiral::Transitioned, f64, f64)>> = (0..n).map(|_| None).collect();
        for t in transitions.iter().filter(|t| t.entry > 0.0 || t.exit > 0.0) {
            let i = t.segment;
            let what = format!(
                "Transition on segment {i} (the polyline's segment {} from its start)",
                i + 1
            );
            let Some(Raw::Arc(arc, ..)) = raw.get(i) else {
                return Err(format!("{what}: that segment is not an arc"));
            };
            let (prev, next) = match (closed, i) {
                (true, _) => ((i + n - 1) % n, (i + 1) % n),
                (false, 0) => return Err(format!("{what}: the arc needs a straight before it")),
                (false, _) if i + 1 >= n => return Err(format!("{what}: the arc needs a straight after it")),
                _ => (i - 1, i + 1),
            };
            let (Raw::Straight(a0, b0), Raw::Straight(a1, b1)) = (&raw[prev], &raw[next]) else {
                return Err(format!("{what}: the arc needs straights on both sides"));
            };
            let (back, forward) = (
                (*b0 - *a0).truncate().normalize_or_zero(),
                (*b1 - *a1).truncate().normalize_or_zero(),
            );
            let pi = line_intersection(a0.truncate(), back, a1.truncate(), forward)
                .ok_or_else(|| format!("{what}: the straights either side are parallel"))?;
            let f =
                fit(pi, back, forward, arc.radius, t.entry, t.exit).map_err(|e| format!("{what}: {e}"))?;
            if f.left != (arc.sweep > 0.0) {
                return Err(format!("{what}: the arc turns the other way to its straights"));
            }
            // Heights along the straights at the new tangent points.
            let z_on = |a: DVec3, b: DVec3, p: DVec2| {
                let d = (b - a).truncate();
                let t = ((p - a.truncate()).dot(d) / d.length_squared().max(1e-18)).clamp(0.0, 1.0);
                a.z + (b.z - a.z) * t
            };
            ends[prev].1 = f.ts.extend(z_on(*a0, *b0, f.ts));
            ends[next].0 = f.st.extend(z_on(*a1, *b1, f.st));
            fitted[i] = Some((f, ends[prev].1.z, ends[next].0.z));
        }
        let mut elements = Vec::new();
        let mut ch = start_chainage;
        let mut push =
            |elements: &mut Vec<Element>, length: f64, start: DVec3, end: DVec3, kind: ElementKind| {
                if length > 1e-9 {
                    elements.push(Element {
                        start_chainage: ch,
                        length,
                        start,
                        end,
                        kind,
                    });
                    ch += length;
                }
            };
        for (i, r) in raw.iter().enumerate() {
            match (r, &fitted[i]) {
                (Raw::Straight(..), _) => {
                    let (a, b) = ends[i];
                    let d = (b - a).truncate();
                    let (orig_a, orig_b) = match r {
                        Raw::Straight(a, b) | Raw::Arc(_, a, b) => (*a, *b),
                    };
                    let dir = (orig_b - orig_a).truncate();
                    if d.dot(dir) < -1e-6 {
                        return Err(format!(
                            "The transitions overlap on the straight at segment {i} (the polyline's segment {}): shorten the spirals",
                            i + 1
                        ));
                    }
                    push(
                        &mut elements,
                        d.length(),
                        a,
                        b,
                        ElementKind::Straight {
                            bearing: bearing_of(dir),
                        },
                    );
                }
                (Raw::Arc(arc, a, b), None) => {
                    let t = arc.tangent_at(0.0);
                    push(
                        &mut elements,
                        arc.length_2d(),
                        *a,
                        *b,
                        ElementKind::Arc {
                            radius: arc.radius,
                            right: arc.sweep < 0.0,
                            centre: arc.center,
                            start_bearing: bearing_of(t),
                            delta: arc.sweep.abs(),
                        },
                    );
                }
                (Raw::Arc(..), Some((f, z0, z1))) => {
                    let right = !f.left;
                    let total = f.total_length().max(1e-12);
                    let z_at = |l: f64| z0 + (z1 - z0) * l / total;
                    let (ts, sc, cs, st) = (
                        f.ts.extend(*z0),
                        f.sc.extend(z_at(f.entry.length)),
                        f.cs.extend(z_at(f.entry.length + f.arc_length())),
                        f.st.extend(*z1),
                    );
                    let back = (f.pi - f.ts).normalize_or_zero();
                    let forward = (f.st - f.pi).normalize_or_zero();
                    push(
                        &mut elements,
                        f.entry.length,
                        ts,
                        sc,
                        ElementKind::Spiral {
                            radius: f.radius,
                            right,
                            entering: true,
                            bearing: bearing_of(back),
                        },
                    );
                    let side = if right { -1.0 } else { 1.0 };
                    let sc_dir = DVec2::from_angle(side * f.entry.theta).rotate(back);
                    push(
                        &mut elements,
                        f.arc_length(),
                        sc,
                        cs,
                        ElementKind::Arc {
                            radius: f.radius,
                            right,
                            centre: f.centre,
                            start_bearing: bearing_of(sc_dir),
                            delta: f.delta_c,
                        },
                    );
                    push(
                        &mut elements,
                        f.exit.length,
                        cs,
                        st,
                        ElementKind::Spiral {
                            radius: f.radius,
                            right,
                            entering: false,
                            bearing: bearing_of(forward),
                        },
                    );
                }
            }
        }
        Ok(Self {
            elements,
            start_chainage,
        })
    }

    pub fn end_chainage(&self) -> f64 {
        self.elements
            .last()
            .map_or(self.start_chainage, Element::end_chainage)
    }

    pub fn length(&self) -> f64 {
        self.end_chainage() - self.start_chainage
    }

    fn element_at(&self, chainage: f64) -> Option<&Element> {
        let eps = 1e-9 * (1.0 + chainage.abs());
        if chainage < self.start_chainage - eps || chainage > self.end_chainage() + eps {
            return None;
        }
        self.elements
            .iter()
            .find(|e| chainage <= e.end_chainage() + eps)
            .or(self.elements.last())
    }

    /// The point at `chainage`, `offset` to the right (negative left), and
    /// the bearing of travel there.
    pub fn point_at(&self, chainage: f64, offset: f64) -> Option<(DVec3, Bearing)> {
        let e = self.element_at(chainage)?;
        let s = chainage - e.start_chainage;
        let (p, u) = e.at(s);
        let right = DVec2::new(u.y, -u.x);
        Some(((p + right * offset).extend(e.height(s)), bearing_of(u)))
    }

    /// The chainage and offset (positive right) of a plan point, from the
    /// nearest element.
    pub fn station_offset(&self, p: DVec2) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64, f64)> = None;
        for e in &self.elements {
            // Nearest of a few samples, then Newton steps on (p − c(s))·u(s) = 0.
            const SAMPLES: usize = 32;
            let mut s = (0..=SAMPLES)
                .map(|k| e.length * k as f64 / SAMPLES as f64)
                .min_by(|a, b| e.at(*a).0.distance(p).total_cmp(&e.at(*b).0.distance(p)))
                .unwrap_or(0.0);
            for _ in 0..30 {
                let (c, u) = e.at(s);
                let step = (p - c).dot(u);
                s = (s + step).clamp(0.0, e.length);
                if step.abs() < 1e-10 {
                    break;
                }
            }
            let (c, u) = e.at(s);
            let d = c.distance(p);
            let offset = (p - c).dot(DVec2::new(u.y, -u.x));
            if best.is_none_or(|b| d < b.0 - 1e-12) {
                best = Some((d, e.start_chainage + s, offset));
            }
        }
        best.map(|(_, ch, off)| (ch, off))
    }

    /// The centreline as points no more than `tolerance` off it (plan).
    pub fn tessellate(&self, tolerance: f64) -> Vec<DVec3> {
        let mut out: Vec<DVec3> = Vec::new();
        for e in &self.elements {
            let steps = match e.kind {
                ElementKind::Straight { .. } => 1,
                // Chord sagitta s ≈ c²/8R: chord length √(8·R·tol).
                _ => {
                    let r = e.radius().unwrap_or(f64::INFINITY);
                    let chord = (8.0 * r * tolerance.max(1e-6)).sqrt();
                    ((e.length / chord).ceil() as usize).clamp(1, 10_000)
                }
            };
            for k in 0..=steps {
                if k == 0 && !out.is_empty() {
                    continue;
                }
                let s = e.length * k as f64 / steps as f64;
                out.push(e.at(s).0.extend(e.height(s)));
            }
        }
        out
    }

    /// Set-out points every `interval` along the alignment (chainages that
    /// are multiples of the interval) at each offset, plus the element ends
    /// (TS, SC, CS, ST and tangent points) when `key_points` is set.
    pub fn setout(&self, interval: f64, offsets: &[f64], key_points: bool) -> Vec<SetoutPoint> {
        let Some(last) = self.elements.last() else {
            return Vec::new();
        };
        let end = last.end_chainage();
        let mut chainages: Vec<(f64, Option<&'static str>)> = Vec::new();
        if interval > 0.0 {
            let mut k = (self.start_chainage / interval).ceil() as i64;
            loop {
                let ch = k as f64 * interval;
                if ch > end + 1e-9 {
                    break;
                }
                chainages.push((ch, None));
                k += 1;
            }
        }
        if key_points {
            chainages.extend(
                self.key_point_names()
                    .into_iter()
                    .map(|(ch, name)| (ch, Some(name))),
            );
        }
        chainages.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Merge coincident chainages, keeping the key point's name.
        let mut merged: Vec<(f64, Option<&'static str>)> = Vec::new();
        for (ch, key) in chainages {
            match merged.last_mut() {
                Some(last) if (last.0 - ch).abs() < 1e-6 => last.1 = last.1.or(key),
                _ => merged.push((ch, key)),
            }
        }
        let mut out = Vec::new();
        for (ch, key) in merged {
            for off in offsets {
                if let Some((p, _)) = self.point_at(ch, *off) {
                    out.push(SetoutPoint {
                        chainage: ch,
                        offset: *off,
                        position: p,
                        key_point: key.is_some(),
                        key: key.unwrap_or(""),
                    });
                }
            }
        }
        out
    }

    /// What each element's start is called: TS, SC, CS, ST, or TP (a tangent
    /// point between a straight and an arc, or the ends).
    pub fn key_point_names(&self) -> Vec<(f64, &'static str)> {
        let mut out: Vec<(f64, &'static str)> = Vec::new();
        for (i, e) in self.elements.iter().enumerate() {
            let name = match (i.checked_sub(1).map(|k| &self.elements[k].kind), &e.kind) {
                (_, ElementKind::Spiral { entering: true, .. }) => "TS",
                (Some(ElementKind::Spiral { entering: true, .. }), ElementKind::Arc { .. }) => "SC",
                (Some(ElementKind::Arc { .. }), ElementKind::Spiral { entering: false, .. }) => "CS",
                (Some(ElementKind::Spiral { entering: false, .. }), _) => "ST",
                (None, _) => "Start",
                _ => "TP",
            };
            out.push((e.start_chainage, name));
        }
        if let Some(last) = self.elements.last() {
            let name = if matches!(last.kind, ElementKind::Spiral { entering: false, .. }) {
                "ST"
            } else {
                "End"
            };
            out.push((last.end_chainage(), name));
        }
        out
    }
}

/// The elements of a polyline alignment (no transitions) starting at `start_chainage`.
pub fn elements(vertices: &[DVec3], bulges: &[f64], closed: bool, start_chainage: f64) -> Vec<Element> {
    Geometry::new(vertices, bulges, closed, start_chainage, &[])
        .map(|g| g.elements)
        .unwrap_or_default()
}

/// A computed set-out point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SetoutPoint {
    pub chainage: f64,
    /// Positive right of the direction of travel.
    pub offset: f64,
    pub position: DVec3,
    /// At a tangent point (start or end of an element) rather than an interval.
    pub key_point: bool,
    /// The key point's name (Start, TP, TS, SC, CS, ST, End), else empty.
    pub key: &'static str,
}

/// Set-out points along a polyline alignment without transitions (see
/// [`Geometry::setout`]). Heights are interpolated along the polyline.
pub fn setout(
    vertices: &[DVec3],
    bulges: &[f64],
    closed: bool,
    start_chainage: f64,
    interval: f64,
    offsets: &[f64],
    key_points: bool,
) -> Vec<SetoutPoint> {
    Geometry::new(vertices, bulges, closed, start_chainage, &[])
        .map(|g| g.setout(interval, offsets, key_points))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: DVec3 = DVec3::new(300_000.0, 5_000_000.0, 50.0);

    /// East 30, then a quarter circle (r = 20) turning left, then north 15.
    fn road() -> (Vec<DVec3>, Vec<f64>) {
        let v = vec![
            O,
            O + DVec3::new(30.0, 0.0, 0.0),
            O + DVec3::new(50.0, 20.0, 0.0),
            O + DVec3::new(50.0, 35.0, 0.0),
        ];
        let b = vec![0.0, (std::f64::consts::PI / 8.0).tan(), 0.0];
        (v, b)
    }

    #[test]
    fn element_table() {
        let (v, b) = road();
        let els = elements(&v, &b, false, 1000.0);
        assert_eq!(els.len(), 3);
        assert!((els[0].length - 30.0).abs() < 1e-9 && els[0].start_chainage == 1000.0);
        match els[1].kind {
            ElementKind::Arc {
                radius,
                right,
                centre,
                delta,
                ..
            } => {
                assert!((radius - 20.0).abs() < 1e-9 && !right);
                assert!((centre - (O.truncate() + DVec2::new(30.0, 20.0))).length() < 1e-9);
                assert!((delta - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
            }
            _ => panic!("arc expected"),
        }
        let arc_len = 20.0 * std::f64::consts::FRAC_PI_2;
        assert!((els[2].start_chainage - (1030.0 + arc_len)).abs() < 1e-9);
    }

    #[test]
    fn setout_points() {
        let (v, b) = road();
        let pts = setout(&v, &b, false, 1000.0, 10.0, &[-5.0, 0.0, 5.0], true);
        // Interval chainages 1000..1070 (8) plus key points 1030 (already), 1031.416, 1046.416 → 10 × 3.
        assert_eq!(pts.len(), 30);
        let tp = pts
            .iter()
            .find(|p| p.key_point && (p.chainage - 1030.0).abs() < 1e-9 && p.offset == 5.0)
            .expect("TP");
        // 5 m right of the line heading east is south.
        assert!((tp.position - (O + DVec3::new(30.0, -5.0, 0.0))).length() < 1e-9);
        assert!(
            pts.iter()
                .all(|p| p.chainage <= 1000.0 + 30.0 + 20.0 * std::f64::consts::FRAC_PI_2 + 15.0 + 1e-9)
        );
        // On the arc: 5 m right of the centreline is 5 m further from the centre.
        let g = Geometry::new(&v, &b, false, 1000.0, &[]).expect("geometry");
        let (p, _) = g.point_at(1040.0, 5.0).expect("on the arc");
        let centre = O.truncate() + DVec2::new(30.0, 20.0);
        assert!((p.truncate().distance(centre) - 25.0).abs() < 1e-9);
    }

    /// A long straight east, R 200 turning left 60°, then a straight on.
    fn highway() -> (Vec<DVec3>, Vec<f64>) {
        let (r, delta) = (200.0, 60f64.to_radians());
        let tc = O.truncate() + DVec2::new(300.0, 0.0);
        let centre = tc + DVec2::new(0.0, r);
        let ct = centre + DVec2::from_angle(-std::f64::consts::FRAC_PI_2 + delta) * r;
        let end = ct + DVec2::from_angle(delta) * 300.0;
        (
            vec![O, tc.extend(50.0), ct.extend(50.0), end.extend(56.0)],
            vec![0.0, (delta / 4.0).tan(), 0.0],
        )
    }

    #[test]
    fn transitions_replace_the_arc() {
        let (v, b) = highway();
        let plain = Geometry::new(&v, &b, false, 0.0, &[]).expect("plain");
        let t = [Transition {
            segment: 1,
            entry: 50.0,
            exit: 50.0,
        }];
        let g = Geometry::new(&v, &b, false, 0.0, &t).expect("spirals");
        let kinds: Vec<&str> = g.key_point_names().iter().map(|(_, n)| *n).collect();
        assert_eq!(kinds, ["Start", "TS", "SC", "CS", "ST", "End"]);
        assert_eq!(g.elements.len(), 5);
        // Direction is continuous at every element joint, and position too.
        for w in g.elements.windows(2) {
            let (p0, u0) = w[0].at(w[0].length);
            let (p1, u1) = w[1].at(0.0);
            assert!(p0.distance(p1) < 1e-6, "{p0} {p1}");
            assert!(u0.angle_to(u1).abs() < 1e-9, "{u0} {u1}");
        }
        // Mid-spiral, the curvature is half the arc's (radius 400): check by three points.
        let spiral = g.elements[1];
        let (a, _) = spiral.at(24.0);
        let (m, _) = spiral.at(25.0);
        let (c, _) = spiral.at(26.0);
        let k = 2.0 * (m - a).perp_dot(c - m) / (a.distance(m) * m.distance(c) * a.distance(c));
        assert!((1.0 / k - 400.0).abs() < 1.0, "{}", 1.0 / k);
        // The spirals add Ls to the curve and take p·tan(Δ/2) + k off each straight.
        let extra = g.length() - plain.length();
        let sp = crate::spiral::Spiral::new(200.0, 50.0);
        let want = 50.0 - 2.0 * (sp.p * 30f64.to_radians().tan() + sp.k);
        assert!((extra - want).abs() < 1e-9, "{extra} vs {want}");
        // Station and offset round-trip on a spiral.
        let (p, _) = g.point_at(spiral.start_chainage + 30.0, -4.0).expect("point");
        let (ch, off) = g.station_offset(p.truncate()).expect("station");
        assert!(
            (ch - (spiral.start_chainage + 30.0)).abs() < 1e-6 && (off + 4.0).abs() < 1e-6,
            "{ch} {off}"
        );
        // Heights follow the polyline's ends.
        assert!((g.point_at(g.end_chainage(), 0.0).expect("end").0.z - 56.0).abs() < 1e-9);
        // Set-out includes the TS, SC, CS and ST.
        let pts = g.setout(0.0, &[0.0], true);
        assert_eq!(pts.len(), 6);
    }

    #[test]
    fn bad_transitions_are_explained() {
        let (v, b) = highway();
        let on = |segment, entry| {
            Geometry::new(
                &v,
                &b,
                false,
                0.0,
                &[Transition {
                    segment,
                    entry,
                    exit: entry,
                }],
            )
        };
        assert!(on(0, 20.0).expect_err("straight").contains("not an arc"));
        assert!(on(1, 300.0).is_err(), "longer than the turn");
        // Spirals so long the TS runs past the start of the first straight.
        let short = [v[0] + DVec3::new(250.0, 0.0, 0.0), v[1], v[2], v[3]];
        let e = Geometry::new(
            &short,
            &b,
            false,
            0.0,
            &[Transition {
                segment: 1,
                entry: 150.0,
                exit: 150.0,
            }],
        )
        .expect_err("overlap");
        assert!(e.contains("overlap"), "{e}");
    }
}
