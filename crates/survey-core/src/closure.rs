//! Figure (lot) closure and missing-value solutions.
//!
//! A figure is a run of courses from a start point, each a straight line or a
//! circular arc given by its chord bearing and chord length. The closure is the
//! gap between where the last course ends and the start. When exactly two
//! values are missing (a bearing and a distance of one course, two distances,
//! two bearings, or a bearing of one and the distance of another) they are
//! solved so the figure closes; some combinations have two solutions.
//!
//! Everything is computed relative to the start, so grid coordinates keep full
//! precision.

use std::fmt::Write as _;

use glam::{DVec2, DVec3};
use thiserror::Error;

use crate::accuracy::Tolerances;
use crate::angle::Bearing;
use crate::arc::{Arc, bulge_segments, polyline_metrics};
use crate::conventions::Conventions;
use crate::offset::circle_circle;

/// The circular arc of an arc course.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CourseArc {
    pub radius: f64,
    /// Turns right (clockwise) in the direction of travel.
    pub right: bool,
    /// Sweeps more than a semicircle.
    pub large: bool,
}

/// One course of a figure: a line, or an arc given by its chord.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Course {
    /// Bearing of the line or of the arc's chord; `None` when unknown.
    pub bearing: Option<Bearing>,
    /// Length of the line or of the arc's chord; `None` when unknown.
    pub distance: Option<f64>,
    pub arc: Option<CourseArc>,
}

impl Course {
    pub fn line(bearing: Bearing, distance: f64) -> Self {
        Self {
            bearing: Some(bearing),
            distance: Some(distance),
            arc: None,
        }
    }

    /// Chord vector when bearing and distance are known.
    pub fn vector(&self) -> Option<DVec2> {
        Some(self.bearing?.unit_vector() * self.distance?)
    }

    /// Central angle of the arc (radians, positive); `None` for lines, unknown
    /// chords or chords longer than the diameter.
    pub fn arc_angle(&self) -> Option<f64> {
        let arc = self.arc?;
        let half = self.distance? / (2.0 * arc.radius);
        if !(0.0..=1.0 + 1e-9).contains(&half) {
            return None;
        }
        let minor = 2.0 * half.min(1.0).asin();
        Some(if arc.large {
            std::f64::consts::TAU - minor
        } else {
            minor
        })
    }

    /// DXF-style bulge of the course (positive anticlockwise), 0 for lines.
    pub fn bulge(&self) -> f64 {
        match (self.arc, self.arc_angle()) {
            (Some(arc), Some(theta)) => {
                let b = (theta / 4.0).tan();
                if arc.right { -b } else { b }
            }
            _ => 0.0,
        }
    }

    /// Length along the course (arc length for arcs).
    pub fn length(&self) -> Option<f64> {
        match self.arc {
            Some(arc) => Some(arc.radius * self.arc_angle()?),
            None => self.distance,
        }
    }

    /// Arc length → chord length for a given radius (`None` if non-positive).
    pub fn chord_from_arc_length(radius: f64, arc_length: f64) -> Option<f64> {
        (radius > 0.0 && arc_length > 0.0 && arc_length < std::f64::consts::TAU * radius)
            .then(|| 2.0 * radius * (arc_length / (2.0 * radius)).sin().abs())
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum ClosureError {
    #[error("a figure needs at least two courses")]
    TooFew,
    #[error("{0} values are blank: leave none blank to check a closure, or exactly two to solve them")]
    Unknowns(usize),
    #[error("course {0}: the distance must be positive")]
    BadDistance(usize),
    #[error("course {0}: the radius must be positive")]
    BadRadius(usize),
    #[error("course {0}: the chord is longer than the arc's diameter")]
    ChordTooLong(usize),
    #[error("no solution: the known courses cannot be closed with these unknowns")]
    NoSolution,
}

/// A missing value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unknown {
    Bearing(usize),
    Distance(usize),
}

impl Unknown {
    pub fn course(self) -> usize {
        match self {
            Unknown::Bearing(i) | Unknown::Distance(i) => i,
        }
    }
}

/// One way of completing a figure.
#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    pub courses: Vec<Course>,
    /// Values that were solved (empty when checking a complete figure).
    pub solved: Vec<Unknown>,
    /// Courses whose solved distance came out negative, so their bearing was reversed.
    pub reversed: Vec<usize>,
}

/// Geometry of a complete figure.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    /// Course end points relative to the start; `vertices[0]` is the start.
    pub vertices: Vec<DVec2>,
    pub bulges: Vec<f64>,
    /// End of the last course minus the start.
    pub misclosure: DVec2,
    /// Total length along the courses.
    pub perimeter: f64,
    /// Enclosed plan area (the gap closed with a straight line).
    pub area: f64,
    pub clockwise: bool,
}

impl Figure {
    /// `N` of `1 : N`; `None` when the figure closes exactly.
    pub fn precision(&self) -> Option<f64> {
        let m = self.misclosure.length();
        (m > 1e-12).then(|| self.perimeter / m)
    }
}

/// The missing values of `courses`, checking the known ones.
pub fn unknowns(courses: &[Course]) -> Result<Vec<Unknown>, ClosureError> {
    let mut out = Vec::new();
    for (i, c) in courses.iter().enumerate() {
        if c.bearing.is_none() {
            out.push(Unknown::Bearing(i));
        }
        match c.distance {
            None => out.push(Unknown::Distance(i)),
            Some(d) if d <= 0.0 => return Err(ClosureError::BadDistance(i + 1)),
            Some(_) => {}
        }
        if let Some(arc) = c.arc {
            if arc.radius <= 0.0 {
                return Err(ClosureError::BadRadius(i + 1));
            }
            if c.distance.is_some_and(|d| d > 2.0 * arc.radius * (1.0 + 1e-9)) {
                return Err(ClosureError::ChordTooLong(i + 1));
            }
        }
    }
    Ok(out)
}

/// Completes the figure: the courses as given when nothing is missing, or every
/// solution for exactly two missing values.
pub fn solve(courses: &[Course]) -> Result<Vec<Solution>, ClosureError> {
    if courses.len() < 2 {
        return Err(ClosureError::TooFew);
    }
    let unknowns = unknowns(courses)?;
    if unknowns.is_empty() {
        return Ok(vec![Solution {
            courses: courses.to_vec(),
            solved: Vec::new(),
            reversed: Vec::new(),
        }]);
    }
    if unknowns.len() != 2 {
        return Err(ClosureError::Unknowns(unknowns.len()));
    }
    // What the unknown courses must add up to for the figure to close.
    let r: DVec2 = -courses.iter().filter_map(Course::vector).sum::<DVec2>();
    let (u0, u1) = (unknowns[0], unknowns[1]);
    let mut out = Vec::new();
    let mut push = |values: &[(usize, DVec2)]| {
        let mut cs = courses.to_vec();
        let mut reversed = Vec::new();
        for &(i, v) in values {
            let c = &mut cs[i];
            // A vector against a known bearing means a negative distance.
            if let (Some(b), None) = (c.bearing, c.distance)
                && b.unit_vector().dot(v) < 0.0
            {
                reversed.push(i);
            }
            c.bearing = Bearing::from_delta(v.x, v.y);
            c.distance = Some(v.length());
        }
        let ok = cs.iter().all(|c| {
            c.bearing.is_some()
                && c.distance.is_some_and(|d| d > 1e-12)
                && c.arc
                    .is_none_or(|a| c.distance.is_some_and(|d| d <= 2.0 * a.radius * (1.0 + 1e-9)))
        });
        // Reversing an arc's chord would also reverse its curve, so that is no solution.
        if ok && reversed.iter().all(|i| courses[*i].arc.is_none()) {
            out.push(Solution {
                courses: cs,
                solved: unknowns.clone(),
                reversed,
            });
        }
    };
    match (u0, u1) {
        // Bearing and distance of the same course.
        (Unknown::Bearing(i), Unknown::Distance(j)) | (Unknown::Distance(j), Unknown::Bearing(i))
            if i == j =>
        {
            push(&[(i, r)]);
        }
        // Two distances on known bearings.
        (Unknown::Distance(i), Unknown::Distance(j)) => {
            let (ui, uj) = (dir(courses, i), dir(courses, j));
            let det = ui.perp_dot(uj);
            if det.abs() > 1e-12 {
                let di = r.perp_dot(uj) / det;
                let dj = ui.perp_dot(r) / det;
                push(&[(i, ui * di), (j, uj * dj)]);
            }
        }
        // Two bearings with known distances: v_i on a circle about the start, v_j = r - v_i.
        (Unknown::Bearing(i), Unknown::Bearing(j)) => {
            let (di, dj) = (dist(courses, i), dist(courses, j));
            for vi in circle_circle(DVec2::ZERO, di, r, dj) {
                push(&[(i, vi), (j, r - vi)]);
            }
        }
        // The bearing of one course and the distance of another: v_i = r - t·u_j, |v_i| = d_i.
        (Unknown::Bearing(i), Unknown::Distance(j)) | (Unknown::Distance(j), Unknown::Bearing(i)) => {
            let (di, uj) = (dist(courses, i), dir(courses, j));
            let b = r.dot(uj);
            let disc = b * b - (r.length_squared() - di * di);
            // Tangency (one solution) within rounding of the squared lengths.
            let eps = 1e-12 * (r.length_squared() + di * di).max(1.0);
            if disc >= -eps {
                let s = if disc > eps { disc.sqrt() } else { 0.0 };
                let mut ts = vec![b + s];
                if s > 0.0 {
                    ts.push(b - s);
                }
                // Positive distances first.
                ts.sort_by(|a, b| b.total_cmp(a));
                for t in ts {
                    push(&[(i, r - uj * t), (j, uj * t)]);
                }
            }
        }
    }
    if out.is_empty() {
        Err(ClosureError::NoSolution)
    } else {
        Ok(out)
    }
}

fn dir(courses: &[Course], i: usize) -> DVec2 {
    courses[i].bearing.map(Bearing::unit_vector).unwrap_or_default()
}

fn dist(courses: &[Course], i: usize) -> f64 {
    courses[i].distance.unwrap_or_default()
}

/// Geometry of a complete figure (unknowns count as zero).
pub fn figure(courses: &[Course]) -> Figure {
    let mut vertices = vec![DVec2::ZERO];
    let mut p = DVec2::ZERO;
    let mut perimeter = 0.0;
    for c in courses {
        p += c.vector().unwrap_or_default();
        perimeter += c.length().unwrap_or_default();
        vertices.push(p);
    }
    let bulges: Vec<f64> = courses.iter().map(Course::bulge).collect();
    let mut closed: Vec<DVec3> = vertices.iter().map(|v| v.extend(0.0)).collect();
    let misclosure = p;
    // When the figure closes, the last vertex duplicates the start.
    if misclosure.length() < 1e-9 {
        closed.pop();
    }
    let (_, _, area) = polyline_metrics(&closed, &bulges, true);
    Figure {
        vertices,
        bulges,
        misclosure,
        perimeter,
        area: area.abs(),
        clockwise: area < 0.0,
    }
}

/// Courses that trace a polyline (closed polylines include the closing course).
pub fn courses_from_polyline(vertices: &[DVec3], bulges: &[f64], closed: bool) -> Vec<Course> {
    bulge_segments(vertices, bulges, closed)
        .filter_map(|(a, b, bulge)| {
            let d = (b - a).truncate();
            let bearing = Bearing::from_delta(d.x, d.y)?;
            let arc = Arc::from_bulge(a, b, bulge).map(|arc| CourseArc {
                radius: arc.radius,
                right: arc.sweep < 0.0,
                large: arc.sweep.abs() > std::f64::consts::PI,
            });
            Some(Course {
                bearing: Some(bearing),
                distance: Some(d.length()),
                arc,
            })
        })
        .collect()
}

/// A plain-text closure report.
pub fn report(
    title: &str,
    solution: &Solution,
    fig: &Figure,
    tolerances: &Tolerances,
    conv: &Conventions,
) -> String {
    let mut s = String::new();
    let len = |m: f64| conv.format_length(m);
    let _ = writeln!(s, "CLOSURE  {title}");
    let _ = writeln!(s);
    let _ = writeln!(s, "{:>3}  {:>16} {:>12}   Arc", "#", "Bearing", "Distance");
    for (i, c) in solution.courses.iter().enumerate() {
        let mark = |u: Unknown| if solution.solved.contains(&u) { "*" } else { " " };
        let bearing = c.bearing.map(|b| conv.format_bearing(b)).unwrap_or_default();
        let arc = match (c.arc, c.length()) {
            (Some(a), Some(l)) => format!(
                "R {}  arc {}  {}",
                len(a.radius),
                len(l),
                if a.right { "right" } else { "left" }
            ),
            _ => String::new(),
        };
        let _ = writeln!(
            s,
            "{:>3}  {:>15}{} {:>11}{}   {arc}",
            i + 1,
            bearing,
            mark(Unknown::Bearing(i)),
            c.distance.map(len).unwrap_or_default(),
            mark(Unknown::Distance(i)),
        );
    }
    if !solution.solved.is_empty() {
        let _ = writeln!(s, "     * solved");
    }
    for i in &solution.reversed {
        let _ = writeln!(
            s,
            "     course {} was solved against its given bearing; the bearing is reversed",
            i + 1
        );
    }
    let _ = writeln!(s);
    let m = fig.misclosure;
    let _ = writeln!(
        s,
        "Perimeter           {}",
        conv.format_length_with_unit(fig.perimeter)
    );
    match Bearing::from_delta(m.x, m.y).filter(|_| fig.precision().is_some()) {
        Some(b) => {
            let _ = writeln!(
                s,
                "Misclosure          {}",
                conv.format_length_with_unit(m.length())
            );
            let _ = writeln!(s, "Misclosure bearing  {}", conv.format_bearing(b));
        }
        None => {
            let _ = writeln!(s, "Misclosure          closes exactly");
        }
    }
    if let Some(n) = fig.precision() {
        let _ = writeln!(s, "Precision           1 : {n:.0}");
    }
    if let Some(c) = tolerances.check_linear(m.length(), fig.perimeter) {
        let _ = writeln!(
            s,
            "Allowed             {}  {}",
            conv.format_length_with_unit(c.allowed),
            if c.pass { "PASS" } else { "FAIL" }
        );
    }
    let _ = writeln!(s, "Area                {}", conv.format_area(fig.area));
    let _ = writeln!(
        s,
        "Direction           {}",
        if fig.clockwise {
            "clockwise"
        } else {
            "anticlockwise"
        }
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(deg: f64, d: f64) -> Course {
        Course::line(Bearing::from_degrees(deg), d)
    }

    fn close(a: Bearing, b: Bearing) -> bool {
        (a.angle() - b.angle()).normalized_signed().radians().abs() < 1e-9
    }

    /// A 30 × 40 rectangle run clockwise: N 30, E 40, S 30, W 40.
    fn rectangle() -> Vec<Course> {
        vec![
            line(0.0, 30.0),
            line(90.0, 40.0),
            line(180.0, 30.0),
            line(270.0, 40.0),
        ]
    }

    #[test]
    fn closed_figure() {
        let sol = solve(&rectangle()).expect("complete");
        assert_eq!(sol.len(), 1);
        let f = figure(&sol[0].courses);
        assert!(f.misclosure.length() < 1e-9 && f.precision().is_none());
        assert!((f.area - 1200.0).abs() < 1e-9 && f.clockwise);
        assert!((f.perimeter - 140.0).abs() < 1e-12);
    }

    #[test]
    fn misclosure_and_precision() {
        let mut c = rectangle();
        c[1].distance = Some(40.014); // 14 mm long
        let f = figure(&c);
        assert!((f.misclosure - DVec2::new(0.014, 0.0)).length() < 1e-9);
        assert!((f.precision().expect("open") - 140.014 / 0.014).abs() < 1e-6);
    }

    #[test]
    fn missing_bearing_and_distance() {
        let mut c = rectangle();
        c[3] = Course::default();
        let sol = solve(&c).expect("solvable");
        assert_eq!(sol.len(), 1);
        let last = sol[0].courses[3];
        assert!(close(last.bearing.expect("solved"), Bearing::from_degrees(270.0)));
        assert!((last.distance.expect("solved") - 40.0).abs() < 1e-9);
    }

    #[test]
    fn missing_two_distances() {
        let mut c = rectangle();
        c[0].distance = None;
        c[1].distance = None;
        let sol = solve(&c).expect("solvable");
        assert_eq!(sol.len(), 1);
        assert!((sol[0].courses[0].distance.expect("d") - 30.0).abs() < 1e-9);
        assert!((sol[0].courses[1].distance.expect("d") - 40.0).abs() < 1e-9);
        assert!(sol[0].reversed.is_empty());
        // Parallel unknown distances cannot be solved.
        let mut c = rectangle();
        c[0].distance = None;
        c[2].distance = None;
        assert_eq!(solve(&c), Err(ClosureError::NoSolution));
    }

    #[test]
    fn missing_two_bearings_has_two_solutions() {
        let mut c = rectangle();
        c[1].bearing = None;
        c[2].bearing = None;
        let sol = solve(&c).expect("solvable");
        assert_eq!(sol.len(), 2);
        // One solution is the original rectangle; both close.
        assert!(
            sol.iter()
                .any(|s| close(s.courses[1].bearing.expect("b"), Bearing::from_degrees(90.0)))
        );
        for s in &sol {
            assert!(figure(&s.courses).misclosure.length() < 1e-9);
        }
    }

    #[test]
    fn missing_bearing_and_other_distance() {
        let mut c = rectangle();
        c[1].bearing = None;
        c[2].distance = None;
        let sol = solve(&c).expect("solvable");
        assert!(!sol.is_empty());
        assert!(
            sol.iter()
                .any(|s| (s.courses[2].distance.expect("d") - 30.0).abs() < 1e-9)
        );
        for s in &sol {
            assert!(figure(&s.courses).misclosure.length() < 1e-9);
        }
        // Bearing of course 2 and distance of course 4: the line can run either way,
        // the second solution reversing course 4.
        let mut c = rectangle();
        c[1].bearing = None;
        c[3].distance = None;
        let sol = solve(&c).expect("solvable");
        assert_eq!(sol.len(), 2);
        assert!(sol[0].reversed.is_empty() && sol[1].reversed == vec![3]);
        assert!(close(
            sol[1].courses[3].bearing.expect("b"),
            Bearing::from_degrees(90.0)
        ));
        assert_eq!(
            solve(&[line(0.0, 1.0), Course::default(), Course::default()]),
            Err(ClosureError::Unknowns(4))
        );
    }

    #[test]
    fn arc_courses() {
        // A D shape: north 20, then a right-turning semicircle (chord due south) back to the start.
        let c = vec![
            line(0.0, 20.0),
            Course {
                bearing: Some(Bearing::from_degrees(180.0)),
                distance: Some(20.0),
                arc: Some(CourseArc {
                    radius: 10.0,
                    right: true,
                    large: false,
                }),
            },
        ];
        let f = figure(&c);
        let pi = std::f64::consts::PI;
        assert!(f.misclosure.length() < 1e-9);
        assert!((f.perimeter - (20.0 + 10.0 * pi)).abs() < 1e-9);
        assert!((f.area - 50.0 * pi).abs() < 1e-9, "{}", f.area);
        assert!(f.clockwise);
        // Round trip through a polyline.
        let verts: Vec<DVec3> = f.vertices[..2].iter().map(|v| v.extend(0.0)).collect();
        let back = courses_from_polyline(&verts, &f.bulges, true);
        assert_eq!(back.len(), 2);
        let arc = back[1].arc.expect("arc");
        assert!((arc.radius - 10.0).abs() < 1e-9 && arc.right);
        let chord = Course::chord_from_arc_length(10.0, 10.0 * pi).expect("chord");
        assert!((chord - 20.0).abs() < 1e-9);
    }

    #[test]
    fn report_mentions_solutions() {
        let mut c = rectangle();
        c[3] = Course::default();
        let sol = solve(&c).expect("solvable").remove(0);
        let f = figure(&sol.courses);
        let tol = Tolerances {
            min_precision: Some(10_000.0),
            ..Default::default()
        };
        let text = report("Lot 1", &sol, &f, &tol, &Conventions::default());
        assert!(
            text.contains("* solved") && text.contains("closes exactly"),
            "{text}"
        );
        assert!(text.contains("1200.000 m²"), "{text}");
    }
}
