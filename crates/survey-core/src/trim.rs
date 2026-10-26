//! Intersections of lines and arcs extended past their ends (the corner where
//! two building faces or kerb lines would meet), and trimming and extending
//! polylines to boundary edges. Plan geometry; heights follow the polyline.
//!
//! A position along a polyline is a parameter `s`: the segment index plus the
//! fraction along that segment (by angle on an arc).

use std::f64::consts::TAU;

use glam::{DVec2, DVec3};
use thiserror::Error;

use crate::arc::{Arc, bulge_segments, reverse_bulges};

/// A segment as stored on a polyline: start, end and bulge.
pub type Segment = (DVec3, DVec3, f64);

const EPS: f64 = 1e-9;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum TrimError {
    #[error("The line does not cross the boundary")]
    NoCrossing,
    #[error("A closed polyline must cross the boundary twice to be trimmed")]
    ClosedOnce,
    #[error("Closed polylines cannot be extended")]
    Closed,
    #[error("Extended, the line never meets the boundary")]
    NoMeeting,
    #[error("The polyline has no length")]
    Degenerate,
}

/// The circle or line a segment lies on, extended without end.
enum Carrier {
    Line { a: DVec2, d: DVec2 },
    Circle { c: DVec2, r: f64 },
}

fn carrier(s: &Segment) -> Option<Carrier> {
    match Arc::from_bulge(s.0, s.1, s.2) {
        Some(arc) => Some(Carrier::Circle {
            c: arc.center,
            r: arc.radius,
        }),
        None => {
            let (a, b) = (s.0.truncate(), s.1.truncate());
            (a.distance(b) > EPS).then(|| Carrier::Line { a, d: b - a })
        }
    }
}

/// Where the line `a + d·u` meets the circle: the line parameters.
fn line_circle(a: DVec2, d: DVec2, c: DVec2, r: f64) -> Vec<f64> {
    let f = a - c;
    let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - r * r);
    let disc = qb * qb - 4.0 * qa * qc;
    if qa == 0.0 || disc < -EPS * qa {
        return Vec::new();
    }
    let s = disc.max(0.0).sqrt();
    if s < EPS {
        vec![-qb / (2.0 * qa)]
    } else {
        vec![(-qb - s) / (2.0 * qa), (-qb + s) / (2.0 * qa)]
    }
}

fn circle_circle(c0: DVec2, r0: f64, c1: DVec2, r1: f64) -> Vec<DVec2> {
    let d = c1 - c0;
    let dist = d.length();
    if dist < EPS || dist > r0 + r1 + EPS || dist < (r0 - r1).abs() - EPS {
        return Vec::new();
    }
    let a = (r0 * r0 - r1 * r1 + dist * dist) / (2.0 * dist);
    let h = (r0 * r0 - a * a).max(0.0).sqrt();
    let base = c0 + d * (a / dist);
    if h < EPS {
        return vec![base];
    }
    let off = d.perp() * (h / dist);
    vec![base + off, base - off]
}

/// Where two segments' carriers meet: lines extended both ways, arcs as
/// whole circles. Parallel lines do not meet.
pub fn intersect_extended(s1: &Segment, s2: &Segment) -> Vec<DVec2> {
    let (Some(c1), Some(c2)) = (carrier(s1), carrier(s2)) else {
        return Vec::new();
    };
    match (c1, c2) {
        (Carrier::Line { a, d }, Carrier::Line { a: b, d: e }) => {
            let den = d.perp_dot(e);
            if den.abs() < EPS * d.length() * e.length() {
                return Vec::new();
            }
            let u = (b - a).perp_dot(e) / den;
            vec![a + d * u]
        }
        (Carrier::Line { a, d }, Carrier::Circle { c, r })
        | (Carrier::Circle { c, r }, Carrier::Line { a, d }) => {
            line_circle(a, d, c, r).into_iter().map(|u| a + d * u).collect()
        }
        (Carrier::Circle { c: c0, r: r0 }, Carrier::Circle { c: c1, r: r1 }) => circle_circle(c0, r0, c1, r1),
    }
}

/// The fraction along `s` of a plan point lying on its carrier, when it is
/// within the segment.
fn fraction_on(s: &Segment, p: DVec2) -> Option<f64> {
    match Arc::from_bulge(s.0, s.1, s.2) {
        Some(arc) => {
            let v = p - arc.center;
            // A point at either end may sit just outside by rounding.
            if p.distance(s.0.truncate()) < 1e-7 {
                return Some(0.0);
            }
            if p.distance(s.1.truncate()) < 1e-7 {
                return Some(1.0);
            }
            arc.param_of_angle(v.y.atan2(v.x))
        }
        None => {
            let (a, b) = (s.0.truncate(), s.1.truncate());
            let d = b - a;
            let len2 = d.length_squared();
            if len2 < EPS {
                return None;
            }
            let t = (p - a).dot(d) / len2;
            let tol = 1e-9 / len2.sqrt();
            (-tol..=1.0 + tol).contains(&t).then(|| t.clamp(0.0, 1.0))
        }
    }
}

/// Where two segments cross, within both.
pub fn intersect_segments(s1: &Segment, s2: &Segment) -> Vec<DVec2> {
    intersect_extended(s1, s2)
        .into_iter()
        .filter(|p| fraction_on(s1, *p).is_some() && fraction_on(s2, *p).is_some())
        .collect()
}

fn segments_of(vertices: &[DVec3], bulges: &[f64], closed: bool) -> Vec<Segment> {
    bulge_segments(vertices, bulges, closed).collect()
}

/// The point at parameter `s` (with its height).
pub fn point_at(vertices: &[DVec3], bulges: &[f64], closed: bool, s: f64) -> DVec3 {
    let segs = segments_of(vertices, bulges, closed);
    if segs.is_empty() {
        return vertices.first().copied().unwrap_or_default();
    }
    let i = (s.floor().max(0.0) as usize).min(segs.len() - 1);
    let t = (s - i as f64).clamp(0.0, 1.0);
    let (a, b, bulge) = segs[i];
    match Arc::from_bulge(a, b, bulge) {
        Some(arc) => arc.point_at(t),
        None => a + (b - a) * t,
    }
}

/// The parameter of the point on the polyline nearest `p`.
pub fn nearest_param(vertices: &[DVec3], bulges: &[f64], closed: bool, p: DVec2) -> Option<f64> {
    let mut best: Option<(f64, f64)> = None;
    for (i, s) in segments_of(vertices, bulges, closed).iter().enumerate() {
        let (t, q) = match Arc::from_bulge(s.0, s.1, s.2) {
            Some(arc) => {
                let q = arc.closest_point(p).truncate();
                (fraction_on(s, q).unwrap_or(0.0), q)
            }
            None => {
                let (a, b) = (s.0.truncate(), s.1.truncate());
                let d = b - a;
                let t = if d.length_squared() > 0.0 {
                    ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (t, a + d * t)
            }
        };
        let dist = q.distance(p);
        if best.is_none_or(|(d, _)| dist < d) {
            best = Some((dist, i as f64 + t));
        }
    }
    best.map(|(_, s)| s)
}

/// The parameters where the polyline crosses the boundary segments, sorted.
pub fn crossings(vertices: &[DVec3], bulges: &[f64], closed: bool, boundary: &[Segment]) -> Vec<f64> {
    let mut out = Vec::new();
    for (i, s) in segments_of(vertices, bulges, closed).iter().enumerate() {
        for b in boundary {
            for p in intersect_segments(s, b) {
                if let Some(t) = fraction_on(s, p) {
                    out.push(i as f64 + t);
                }
            }
        }
    }
    out.sort_by(f64::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    out
}

/// The part of the polyline from parameter `from` to `to` (on a closed one,
/// `to < from` runs on past the start).
pub fn sub_polyline(
    vertices: &[DVec3],
    bulges: &[f64],
    closed: bool,
    from: f64,
    to: f64,
) -> (Vec<DVec3>, Vec<f64>) {
    let segs = segments_of(vertices, bulges, closed);
    let n = segs.len();
    if n == 0 {
        return (vertices.to_vec(), Vec::new());
    }
    let nf = n as f64;
    let to = if closed && to < from { to + nf } else { to };
    // A parameter past the end of a closed polyline, brought back round.
    let wrap = |q: f64| if q > nf { q - nf } else { q };
    let mut out_v = vec![point_at(vertices, bulges, closed, wrap(from))];
    let mut out_b = Vec::new();
    let mut s = from;
    while s < to - 1e-12 {
        let i = s.floor() as usize;
        let next = ((i + 1) as f64).min(to);
        let seg = segs[i % n];
        let (t0, t1) = (s - i as f64, next - i as f64);
        out_b.push(if seg.2 == 0.0 {
            0.0
        } else {
            (seg.2.atan() * (t1 - t0)).tan()
        });
        out_v.push(point_at(vertices, bulges, closed, wrap(next)));
        s = next;
    }
    if out_b.iter().all(|b| *b == 0.0) {
        out_b.clear();
    }
    (out_v, out_b)
}

/// A polyline piece: vertices and bulges (empty when straight).
pub type Piece = (Vec<DVec3>, Vec<f64>);

/// Cuts away the part of the polyline around parameter `pick` between the
/// boundary crossings either side of it. An open polyline leaves up to two
/// pieces; a closed one becomes one open piece.
pub fn trim(
    vertices: &[DVec3],
    bulges: &[f64],
    closed: bool,
    cuts: &[f64],
    pick: f64,
) -> Result<Vec<Piece>, TrimError> {
    let n = segments_of(vertices, bulges, closed).len() as f64;
    if n == 0.0 {
        return Err(TrimError::Degenerate);
    }
    let before = cuts
        .iter()
        .copied()
        .filter(|c| *c < pick - 1e-12)
        .reduce(f64::max);
    let after = cuts
        .iter()
        .copied()
        .filter(|c| *c > pick + 1e-12)
        .reduce(f64::min);
    if closed {
        let mut distinct = cuts.to_vec();
        distinct.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        // Both ends and the start count as one place on a closed figure.
        if distinct.len() >= 2
            && (distinct[0] - 0.0).abs() < 1e-9
            && (distinct[distinct.len() - 1] - n).abs() < 1e-9
        {
            distinct.pop();
        }
        if distinct.len() < 2 {
            return Err(TrimError::ClosedOnce);
        }
        let a = before.or_else(|| distinct.last().copied()).expect("two cuts");
        let b = after.or_else(|| distinct.first().copied()).expect("two cuts");
        return Ok(vec![sub_polyline(vertices, bulges, true, b, a)]);
    }
    if before.is_none() && after.is_none() {
        return Err(TrimError::NoCrossing);
    }
    let mut pieces = Vec::new();
    if let Some(a) = before.filter(|a| *a > 1e-9) {
        pieces.push(sub_polyline(vertices, bulges, false, 0.0, a));
    }
    if let Some(b) = after.filter(|b| *b < n - 1e-9) {
        pieces.push(sub_polyline(vertices, bulges, false, b, n));
    }
    Ok(pieces)
}

/// Extends the end (or the start, with `at_start`) of an open polyline to the
/// nearest boundary segment it meets: a straight end along its line, an arc
/// on round its circle. Heights carry on at the end segment's grade.
pub fn extend(
    vertices: &[DVec3],
    bulges: &[f64],
    at_start: bool,
    boundary: &[Segment],
) -> Result<Piece, TrimError> {
    let mut v = vertices.to_vec();
    let mut b = bulges.to_vec();
    b.resize(v.len(), 0.0);
    if at_start {
        reverse_bulges(&mut v, &mut b, false);
        b.resize(v.len(), 0.0);
    }
    let n = v.len();
    if n < 2 {
        return Err(TrimError::Degenerate);
    }
    let last: Segment = (v[n - 2], v[n - 1], b[n - 2]);
    let mut best: Option<(f64, DVec2)> = None;
    match Arc::from_bulge(last.0, last.1, last.2) {
        Some(arc) => {
            // How much further round the circle (same direction) each meeting is.
            let end = arc.end_angle();
            let room = TAU - arc.sweep.abs();
            for edge in boundary {
                for p in intersect_extended(&last, edge) {
                    if fraction_on(edge, p).is_none() {
                        continue;
                    }
                    let d = p - arc.center;
                    let ang = d.y.atan2(d.x);
                    let extra = if arc.sweep >= 0.0 {
                        (ang - end).rem_euclid(TAU)
                    } else {
                        (end - ang).rem_euclid(TAU)
                    };
                    if extra > 1e-9 && extra < room && best.is_none_or(|(e, _)| extra < e) {
                        best = Some((extra, p));
                    }
                }
            }
            let (extra, p) = best.ok_or(TrimError::NoMeeting)?;
            let sweep = arc.sweep + extra * arc.sweep.signum();
            let grade = (arc.z_end - arc.z_start) / arc.sweep.abs().max(EPS);
            v[n - 1] = p.extend(arc.z_end + grade * extra);
            b[n - 2] = (sweep / 4.0).tan();
        }
        None => {
            let (a, e) = (last.0.truncate(), last.1.truncate());
            let d = e - a;
            if d.length_squared() < EPS {
                return Err(TrimError::Degenerate);
            }
            for edge in boundary {
                for p in intersect_extended(&last, edge) {
                    if fraction_on(edge, p).is_none() {
                        continue;
                    }
                    let u = (p - a).dot(d) / d.length_squared();
                    if u > 1.0 + 1e-9 && best.is_none_or(|(bu, _)| u < bu) {
                        best = Some((u, p));
                    }
                }
            }
            let (u, p) = best.ok_or(TrimError::NoMeeting)?;
            v[n - 1] = p.extend(last.0.z + (last.1.z - last.0.z) * u);
        }
    }
    if at_start {
        reverse_bulges(&mut v, &mut b, false);
    }
    b.truncate(v.len());
    if b.iter().all(|x| *x == 0.0) {
        b.clear();
    }
    Ok((v, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    #[test]
    fn apparent_intersections() {
        // Two building faces that stop short of their corner.
        let a: Segment = (p(0.0, 0.0), p(5.0, 0.0), 0.0);
        let b: Segment = (p(10.0, 2.0), p(10.0, 8.0), 0.0);
        assert_eq!(intersect_extended(&a, &b), vec![DVec2::new(10.0, 0.0)]);
        assert!(
            intersect_segments(&a, &b).is_empty(),
            "they do not actually cross"
        );
        // A line and an arc's circle (a quarter circle radius 10 about the origin).
        let arc: Segment = (
            p(10.0, 0.0),
            p(0.0, 10.0),
            (std::f64::consts::FRAC_PI_2 / 4.0).tan(),
        );
        let horiz: Segment = (p(-20.0, 6.0), p(-15.0, 6.0), 0.0);
        let mut pts = intersect_extended(&horiz, &arc);
        pts.sort_by(|a, b| a.x.total_cmp(&b.x));
        assert_eq!(pts.len(), 2);
        assert!((pts[1] - DVec2::new(8.0, 6.0)).length() < 1e-9);
        // Parallel lines never meet.
        assert!(intersect_extended(&a, &(p(0.0, 1.0), p(5.0, 1.0), 0.0)).is_empty());
    }

    #[test]
    fn trimming_open_and_closed() {
        let line = [p(0.0, 0.0), p(10.0, 0.0), p(20.0, 0.0)];
        let fence: Vec<Segment> = vec![
            (p(15.0, -5.0), p(15.0, 5.0), 0.0),
            (p(5.0, -5.0), p(5.0, 5.0), 0.0),
        ];
        let cuts = crossings(&line, &[], false, &fence);
        assert_eq!(cuts, vec![0.5, 1.5]);
        // Picking the middle leaves both ends.
        let pieces = trim(&line, &[], false, &cuts, 1.0).expect("trim");
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].0, vec![p(0.0, 0.0), p(5.0, 0.0)]);
        assert_eq!(pieces[1].0, vec![p(15.0, 0.0), p(20.0, 0.0)]);
        // Picking the end trims it back to the last crossing.
        let pieces = trim(&line, &[], false, &cuts, 1.8).expect("trim");
        assert_eq!(
            pieces,
            vec![(vec![p(0.0, 0.0), p(10.0, 0.0), p(15.0, 0.0)], vec![])]
        );
        assert_eq!(trim(&line, &[], false, &[], 1.0), Err(TrimError::NoCrossing));

        // A closed square cut by a line through it: the picked side goes.
        let square = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
        let cutter: Vec<Segment> = vec![(p(5.0, -1.0), p(5.0, 11.0), 0.0)];
        let cuts = crossings(&square, &[], true, &cutter);
        assert_eq!(cuts, vec![0.5, 2.5]);
        // Pick on the right side (segment 1, x = 10).
        let pieces = trim(&square, &[], true, &cuts, 1.5).expect("trim");
        assert_eq!(pieces.len(), 1);
        assert_eq!(
            pieces[0].0,
            vec![p(5.0, 10.0), p(0.0, 10.0), p(0.0, 0.0), p(5.0, 0.0)]
        );
    }

    #[test]
    fn trimming_an_arc_keeps_its_curve() {
        // A semicircle radius 10 from (10,0) round to (-10,0), cut at x = 0.
        let v = [p(10.0, 0.0), p(-10.0, 0.0)];
        let b = [1.0];
        let cutter: Vec<Segment> = vec![(p(0.0, -20.0), p(0.0, 20.0), 0.0)];
        let cuts = crossings(&v, &b, false, &cutter);
        assert_eq!(cuts.len(), 1);
        assert!((cuts[0] - 0.5).abs() < 1e-9);
        let pieces = trim(&v, &b, false, &cuts, 0.8).expect("trim");
        let (pv, pb) = &pieces[0];
        assert!((pv[1] - p(0.0, 10.0)).length() < 1e-9);
        // A quarter circle: bulge tan(90°/4).
        assert!((pb[0] - (std::f64::consts::FRAC_PI_2 / 4.0).tan()).abs() < 1e-9);
    }

    #[test]
    fn extending_lines_and_arcs() {
        let wall: Vec<Segment> = vec![(p(20.0, -5.0), p(20.0, 5.0), 0.0)];
        // Straight, with heights carrying on at the grade (1 in 10).
        let v = [DVec3::new(0.0, 0.0, 0.0), DVec3::new(10.0, 0.0, 1.0)];
        let (ext, _) = extend(&v, &[], false, &wall).expect("extend");
        assert!((ext[1] - DVec3::new(20.0, 0.0, 2.0)).length() < 1e-9);
        // From the start instead: the line runs the other way, away from the wall.
        assert_eq!(extend(&v, &[], true, &wall), Err(TrimError::NoMeeting));
        let back: Vec<Segment> = vec![(p(-3.0, -5.0), p(-3.0, 5.0), 0.0)];
        let (ext, _) = extend(&v, &[], true, &back).expect("extend start");
        assert!((ext[0].truncate() - DVec2::new(-3.0, 0.0)).length() < 1e-9);
        // A quarter arc continued round its circle to the y = 0 line on the left.
        let v = [p(10.0, 0.0), p(0.0, 10.0)];
        let b = [(std::f64::consts::FRAC_PI_2 / 4.0).tan()];
        let ground: Vec<Segment> = vec![(p(-20.0, 0.0), p(-5.0, 0.0), 0.0)];
        let (ext, eb) = extend(&v, &b, false, &ground).expect("extend arc");
        assert!((ext[1] - p(-10.0, 0.0)).length() < 1e-9);
        assert!((eb[0] - 1.0).abs() < 1e-9, "now a semicircle");
    }
}
