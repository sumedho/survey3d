//! Triangulated irregular networks (TINs): surface models built from points
//! and breaklines by constrained Delaunay triangulation.
//!
//! Breaklines become constraint edges, so no triangle crosses them; where
//! breaklines cross, a vertex is added with the height of the breakline being
//! inserted. An optional boundary (and holes) trims the surface, and a maximum
//! edge length drops long, thin triangles around the outside.
//!
//! Triangulation runs on coordinates relative to the first point so grid
//! coordinates keep full precision; the result stores absolute coordinates.

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Point2, Triangulation};
use thiserror::Error;

use crate::geom::Aabb3;

/// What a surface is built from (absolute coordinates).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TinInput {
    pub points: Vec<DVec3>,
    /// 3D polylines whose segments the triangles must follow.
    pub breaklines: Vec<Vec<DVec3>>,
    /// Outer boundary (plan); triangles outside are dropped.
    pub boundary: Option<Vec<DVec2>>,
    /// Holes (plan); triangles inside are dropped.
    pub holes: Vec<Vec<DVec2>>,
    /// Triangles with a longer plan edge are dropped.
    pub max_edge: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum TinError {
    #[error("A surface needs at least three points with heights")]
    TooFewPoints,
    #[error("The points are all in a line; no triangles can be formed")]
    Degenerate,
    #[error("Every triangle was removed by the boundary or the maximum edge length")]
    Empty,
}

/// A triangulated surface.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tin {
    pub vertices: Vec<DVec3>,
    /// Anticlockwise (plan) vertex indices.
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Clone, Copy, Debug)]
struct Vertex {
    pos: Point2<f64>,
    z: f64,
}

impl HasPosition for Vertex {
    type Scalar = f64;
    fn position(&self) -> Point2<f64> {
        self.pos
    }
}

type Cdt = ConstrainedDelaunayTriangulation<Vertex>;

/// Whether `p` is inside the polygon (even-odd rule).
pub fn point_in_polygon(p: DVec2, poly: &[DVec2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Height on the triangle `(a, b, c)` at `p` (plan barycentric); `None` outside it.
pub fn triangle_height(a: DVec3, b: DVec3, c: DVec3, p: DVec2) -> Option<f64> {
    let (a2, b2, c2) = (a.truncate(), b.truncate(), c.truncate());
    let det = (b2 - a2).perp_dot(c2 - a2);
    if det.abs() < 1e-18 {
        return None;
    }
    let l1 = (c2 - b2).perp_dot(p - b2) / det;
    let l2 = (a2 - c2).perp_dot(p - c2) / det;
    let l3 = 1.0 - l1 - l2;
    let eps = -1e-9;
    (l1 >= eps && l2 >= eps && l3 >= eps).then_some(l1 * a.z + l2 * b.z + l3 * c.z)
}

/// Builds a surface. Points without a usable height should not be passed in.
pub fn build(input: &TinInput) -> Result<Tin, TinError> {
    let origin = input
        .points
        .first()
        .or_else(|| input.breaklines.iter().flatten().next())
        .map(|p| p.truncate())
        .ok_or(TinError::TooFewPoints)?;
    let rel = |p: DVec3| Vertex {
        pos: Point2::new(p.x - origin.x, p.y - origin.y),
        z: p.z,
    };
    let vertices: Vec<Vertex> = input
        .points
        .iter()
        .filter(|p| p.is_finite())
        .map(|p| rel(*p))
        .collect();
    let mut cdt = Cdt::bulk_load(vertices).map_err(|_| TinError::Degenerate)?;

    // Breaklines: their vertices (and heights) win over coincident points.
    for line in &input.breaklines {
        let mut prev: Option<(spade::handles::FixedVertexHandle, Vertex)> = None;
        for p in line.iter().filter(|p| p.is_finite()) {
            let v = rel(*p);
            let Ok(h) = cdt.insert(v) else {
                prev = None;
                continue;
            };
            if let Some((ph, pv)) = prev
                && ph != h
            {
                let (a, b) = (pv, v);
                cdt.add_constraint_and_split(ph, h, |pos| {
                    let d = DVec2::new(b.pos.x - a.pos.x, b.pos.y - a.pos.y);
                    let t = (DVec2::new(pos.x - a.pos.x, pos.y - a.pos.y).dot(d) / d.length_squared())
                        .clamp(0.0, 1.0);
                    Vertex {
                        pos,
                        z: a.z + (b.z - a.z) * t,
                    }
                });
            }
            prev = Some((h, v));
        }
    }
    if cdt.num_inner_faces() == 0 {
        return Err(if cdt.num_vertices() < 3 {
            TinError::TooFewPoints
        } else {
            TinError::Degenerate
        });
    }

    // Boundary and holes become constraints (so the trimmed edge is clean) where
    // they lie on the surface; their heights come from the surface.
    let plan = |poly: &Vec<DVec2>| -> Vec<DVec2> { poly.iter().map(|p| *p - origin).collect() };
    let boundary = input.boundary.as_ref().map(plan);
    let holes: Vec<Vec<DVec2>> = input.holes.iter().map(plan).collect();
    for ring in boundary.iter().chain(&holes) {
        let interim = extract(&cdt, DVec2::ZERO);
        let index = interim.index();
        let height = |p: DVec2| index.height_at(&interim, p);
        let mut handles = Vec::new();
        for p in ring {
            let h = height(*p).and_then(|z| {
                cdt.insert(Vertex {
                    pos: Point2::new(p.x, p.y),
                    z,
                })
                .ok()
            });
            handles.push(h);
        }
        let n = handles.len();
        for i in 0..n {
            if let (Some(a), Some(b)) = (handles[i], handles[(i + 1) % n])
                && a != b
            {
                cdt.add_constraint_and_split(a, b, |pos| Vertex {
                    pos,
                    z: height(DVec2::new(pos.x, pos.y)).unwrap_or(f64::NAN),
                });
            }
        }
    }

    // Keep triangles inside the boundary, outside holes and within the edge limit.
    let max_edge = input.max_edge.filter(|m| *m > 0.0);
    let mut used: HashMap<usize, u32> = HashMap::new();
    let mut out = Tin::default();
    for face in cdt.inner_faces() {
        let vs = face.vertices();
        let pts = vs.map(|v| {
            let d = v.data();
            DVec3::new(d.pos.x, d.pos.y, d.z)
        });
        if pts.iter().any(|p| !p.z.is_finite()) {
            continue;
        }
        let centroid = (pts[0] + pts[1] + pts[2]).truncate() / 3.0;
        if boundary.as_ref().is_some_and(|b| !point_in_polygon(centroid, b)) {
            continue;
        }
        if holes.iter().any(|h| point_in_polygon(centroid, h)) {
            continue;
        }
        if let Some(m) = max_edge
            && (0..3).any(|i| pts[i].truncate().distance(pts[(i + 1) % 3].truncate()) > m)
        {
            continue;
        }
        let mut tri = [0u32; 3];
        for (k, v) in vs.iter().enumerate() {
            let next = out.vertices.len() as u32;
            let idx = *used.entry(v.fix().index()).or_insert_with(|| {
                out.vertices.push(pts[k] + origin.extend(0.0));
                next
            });
            tri[k] = idx;
        }
        out.triangles.push(tri);
    }
    if out.triangles.is_empty() {
        return Err(TinError::Empty);
    }
    Ok(out)
}

/// All faces of a triangulation as a TIN, offset by `origin`.
fn extract(cdt: &Cdt, origin: DVec2) -> Tin {
    let mut tin = Tin {
        vertices: cdt
            .vertices()
            .map(|v| DVec3::new(v.data().pos.x + origin.x, v.data().pos.y + origin.y, v.data().z))
            .collect(),
        triangles: Vec::new(),
    };
    for face in cdt.inner_faces() {
        tin.triangles
            .push(face.vertices().map(|v| v.fix().index() as u32));
    }
    tin
}

impl Tin {
    pub fn triangle(&self, i: usize) -> [DVec3; 3] {
        self.triangles[i].map(|k| self.vertices[k as usize])
    }

    pub fn bounds(&self) -> Aabb3 {
        Aabb3::from_points(self.vertices.iter().copied())
    }

    /// Plan area of all triangles.
    pub fn plan_area(&self) -> f64 {
        (0..self.triangles.len())
            .map(|i| {
                let [a, b, c] = self.triangle(i);
                0.5 * (b - a).truncate().perp_dot((c - a).truncate()).abs()
            })
            .sum()
    }

    /// Sloped (3D) area of all triangles.
    pub fn surface_area(&self) -> f64 {
        (0..self.triangles.len())
            .map(|i| {
                let [a, b, c] = self.triangle(i);
                0.5 * (b - a).cross(c - a).length()
            })
            .sum()
    }

    /// Unique edges as vertex index pairs.
    pub fn edges(&self) -> Vec<[u32; 2]> {
        let mut edges: Vec<[u32; 2]> = self
            .triangles
            .iter()
            .flat_map(|t| [[t[0], t[1]], [t[1], t[2]], [t[2], t[0]]])
            .map(|[a, b]| if a < b { [a, b] } else { [b, a] })
            .collect();
        edges.sort_unstable();
        edges.dedup();
        edges
    }

    /// A spatial index for height lookups and profiles.
    pub fn index(&self) -> TinIndex {
        TinIndex::new(self)
    }
}

/// A uniform grid over the TIN's triangles (plan), for point location.
#[derive(Clone, Debug, Default)]
pub struct TinIndex {
    min: DVec2,
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

impl TinIndex {
    pub fn new(tin: &Tin) -> Self {
        if tin.triangles.is_empty() {
            return Self::default();
        }
        let b = tin.bounds();
        let size = (b.max - b.min).truncate().max(DVec2::splat(1e-9));
        // About two triangles per cell.
        let cell = (size.x * size.y / (tin.triangles.len() as f64 / 2.0).max(1.0))
            .sqrt()
            .max(1e-9);
        let cols = ((size.x / cell).ceil() as usize).clamp(1, 4096);
        let rows = ((size.y / cell).ceil() as usize).clamp(1, 4096);
        let cell = (size.x / cols as f64).max(size.y / rows as f64);
        let mut index = Self {
            min: b.min.truncate(),
            cell,
            cols,
            rows,
            cells: vec![Vec::new(); cols * rows],
        };
        for i in 0..tin.triangles.len() {
            let [a, b, c] = tin.triangle(i);
            let lo = a.truncate().min(b.truncate()).min(c.truncate());
            let hi = a.truncate().max(b.truncate()).max(c.truncate());
            let (c0, r0) = index.cell_of(lo);
            let (c1, r1) = index.cell_of(hi);
            for r in r0..=r1 {
                for col in c0..=c1 {
                    index.cells[r * cols + col].push(i as u32);
                }
            }
        }
        index
    }

    fn cell_of(&self, p: DVec2) -> (usize, usize) {
        let d = (p - self.min) / self.cell;
        (
            (d.x.floor().max(0.0) as usize).min(self.cols - 1),
            (d.y.floor().max(0.0) as usize).min(self.rows - 1),
        )
    }

    /// Triangles whose bounding boxes may contain `p`.
    pub fn candidates(&self, p: DVec2) -> &[u32] {
        if self.cells.is_empty() {
            return &[];
        }
        let d = (p - self.min) / self.cell;
        if d.x < -1e-9 || d.y < -1e-9 || d.x > self.cols as f64 + 1e-9 || d.y > self.rows as f64 + 1e-9 {
            return &[];
        }
        let (c, r) = self.cell_of(p);
        &self.cells[r * self.cols + c]
    }

    /// Triangles whose cells overlap the plan box `lo..hi`.
    pub fn candidates_in(&self, lo: DVec2, hi: DVec2) -> Vec<u32> {
        if self.cells.is_empty() {
            return Vec::new();
        }
        let (c0, r0) = self.cell_of(lo);
        let (c1, r1) = self.cell_of(hi);
        let mut out = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                out.extend_from_slice(&self.cells[r * self.cols + c]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Surface height at `p`, or `None` off the surface.
    pub fn height_at(&self, tin: &Tin, p: DVec2) -> Option<f64> {
        self.candidates(p).iter().find_map(|i| {
            let [a, b, c] = tin.triangle(*i as usize);
            triangle_height(a, b, c, p)
        })
    }
}

/// A profile along a plan polyline: runs of `(distance along, height)`
/// separated where the line leaves the surface. Breaks are exact (every
/// triangle edge crossing is included).
pub fn profile(tin: &Tin, index: &TinIndex, line: &[DVec2]) -> Vec<Vec<(f64, f64)>> {
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut start = 0.0;
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = a.distance(b);
        if len <= 0.0 {
            continue;
        }
        let mut ts = vec![0.0, 1.0];
        for i in index.candidates_in(a.min(b), a.max(b)) {
            let t = tin.triangles[i as usize];
            for k in 0..3 {
                let p = tin.vertices[t[k] as usize].truncate();
                let q = tin.vertices[t[(k + 1) % 3] as usize].truncate();
                if let Some((s, u)) = crate::geom::line_intersection_2d(a, b, p, q)
                    && (-1e-12..=1.0 + 1e-12).contains(&s)
                    && (-1e-12..=1.0 + 1e-12).contains(&u)
                {
                    ts.push(s.clamp(0.0, 1.0));
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() * len < 1e-9);
        let at = |t: f64| index.height_at(tin, a + (b - a) * t);
        for (k, t) in ts.iter().enumerate() {
            // A gap between two points on the surface (e.g. across a hole).
            if k > 0 {
                let mid = 0.5 * (ts[k - 1] + t);
                if at(mid).is_none() && !current.is_empty() {
                    runs.push(std::mem::take(&mut current));
                }
            }
            match at(*t) {
                Some(z) => {
                    let d = start + t * len;
                    if current.last().is_none_or(|(x, _)| (d - x).abs() > 1e-9) {
                        current.push((d, z));
                    }
                }
                None => {
                    if !current.is_empty() {
                        runs.push(std::mem::take(&mut current));
                    }
                }
            }
        }
        start += len;
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs.retain(|r| r.len() > 1);
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: DVec3 = DVec3::new(512_000.0, 6_912_000.0, 0.0);

    /// A 10 × 10 grid of points on the plane z = 100 + 0.1 x + 0.2 y.
    fn plane() -> Vec<DVec3> {
        let mut pts = Vec::new();
        for i in 0..10 {
            for j in 0..10 {
                let (x, y) = (i as f64 * 10.0, j as f64 * 10.0);
                pts.push(O + DVec3::new(x, y, 100.0 + 0.1 * x + 0.2 * y));
            }
        }
        pts
    }

    #[test]
    fn plane_surface() {
        let tin = build(&TinInput {
            points: plane(),
            ..Default::default()
        })
        .expect("tin");
        assert_eq!(tin.vertices.len(), 100);
        assert_eq!(tin.triangles.len(), 2 * 9 * 9);
        assert!((tin.plan_area() - 8100.0).abs() < 1e-6);
        let index = tin.index();
        let h = index
            .height_at(&tin, (O + DVec3::new(33.3, 47.1, 0.0)).truncate())
            .expect("on");
        assert!((h - (100.0 + 3.33 + 9.42)).abs() < 1e-9, "{h}");
        assert!(
            index
                .height_at(&tin, (O + DVec3::new(-1.0, 5.0, 0.0)).truncate())
                .is_none()
        );
    }

    #[test]
    fn breaklines_are_respected() {
        // A ridge along x = 45 at height 120, between two flat rows of points.
        let mut points = Vec::new();
        for j in 0..=10 {
            let y = j as f64 * 10.0;
            points.push(O + DVec3::new(0.0, y, 100.0));
            points.push(O + DVec3::new(90.0, y, 100.0));
        }
        let ridge = vec![
            O + DVec3::new(45.0, 0.0, 120.0),
            O + DVec3::new(45.0, 100.0, 120.0),
        ];
        let tin = build(&TinInput {
            points,
            breaklines: vec![ridge],
            ..Default::default()
        })
        .expect("tin");
        // Every triangle lies wholly on one side of the ridge.
        for i in 0..tin.triangles.len() {
            let [a, b, c] = tin.triangle(i);
            let xs = [a.x, b.x, c.x].map(|x| x - O.x);
            let left = xs.iter().all(|x| *x <= 45.0 + 1e-9);
            let right = xs.iter().all(|x| *x >= 45.0 - 1e-9);
            assert!(left || right, "triangle crosses the ridge: {xs:?}");
        }
        let h = tin
            .index()
            .height_at(&tin, (O + DVec3::new(45.0, 50.0, 0.0)).truncate());
        assert!((h.expect("on") - 120.0).abs() < 1e-9);
    }

    #[test]
    fn boundary_holes_and_edge_limit() {
        let b = |pts: &[(f64, f64)]| {
            pts.iter()
                .map(|(x, y)| O.truncate() + DVec2::new(*x, *y))
                .collect::<Vec<_>>()
        };
        let tin = build(&TinInput {
            points: plane(),
            boundary: Some(b(&[(5.0, 5.0), (85.0, 5.0), (85.0, 85.0), (5.0, 85.0)])),
            holes: vec![b(&[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)])],
            ..Default::default()
        })
        .expect("tin");
        // 80 × 80 minus a 20 × 20 hole, clipped exactly at the constraints.
        assert!(
            (tin.plan_area() - (6400.0 - 400.0)).abs() < 1e-6,
            "{}",
            tin.plan_area()
        );
        // The boundary corner takes its height from the surface.
        let h = tin
            .index()
            .height_at(&tin, (O + DVec3::new(5.0, 5.0, 0.0)).truncate())
            .expect("on");
        assert!((h - (100.0 + 0.5 + 1.0)).abs() < 1e-9);

        // A sparse outlier is cut off by the maximum edge length.
        let mut pts = plane();
        pts.push(O + DVec3::new(300.0, 300.0, 100.0));
        let tin = build(&TinInput {
            points: pts,
            max_edge: Some(20.0),
            ..Default::default()
        })
        .expect("tin");
        assert!((tin.plan_area() - 8100.0).abs() < 1e-6);
        assert_eq!(
            build(&TinInput {
                points: plane()[..2].to_vec(),
                ..Default::default()
            }),
            Err(TinError::TooFewPoints)
        );
    }

    #[test]
    fn profiles_are_exact_and_gapped() {
        // A plane cut along a diagonal line, extending beyond the surface.
        let tin = build(&TinInput {
            points: plane(),
            ..Default::default()
        })
        .expect("tin");
        let index = tin.index();
        let o = O.truncate();
        let line = [o + DVec2::new(-10.0, 45.0), o + DVec2::new(100.0, 45.0)];
        let runs = profile(&tin, &index, &line);
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        // Starts where the line enters the surface (x = 0, 10 m along) and ends at x = 90.
        assert!((run[0].0 - 10.0).abs() < 1e-9 && (run.last().expect("end").0 - 100.0).abs() < 1e-9);
        for (d, z) in run {
            let x = d - 10.0;
            assert!((z - (100.0 + 0.1 * x + 0.2 * 45.0)).abs() < 1e-9);
        }
        // Every vertical grid line and diagonal crossed is a break (more than 10 points).
        assert!(run.len() > 10);

        // A hole splits the profile into two runs.
        let b = |pts: &[(f64, f64)]| {
            pts.iter()
                .map(|(x, y)| o + DVec2::new(*x, *y))
                .collect::<Vec<_>>()
        };
        let holed = build(&TinInput {
            points: plane(),
            holes: vec![b(&[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)])],
            ..Default::default()
        })
        .expect("tin");
        let runs = profile(&holed, &holed.index(), &line);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!((runs[0].last().expect("end").0 - 50.0).abs() < 1e-6);
        assert!((runs[1][0].0 - 70.0).abs() < 1e-6);
    }
}
