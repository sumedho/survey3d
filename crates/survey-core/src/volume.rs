//! Volumes between surfaces (or a surface and a level), optionally within a
//! boundary.
//!
//! The two surfaces and the boundary are merged into one constrained
//! triangulation containing every edge of each, so within each merged
//! triangle both surfaces are planar and their difference is linear. The
//! volume of each triangle is then integrated exactly, splitting it where the
//! difference changes sign. Only the area covered by both datums (and inside
//! the boundary) counts.
//!
//! Sign convention: **fill** is where the top datum is above the base, **cut**
//! where it is below.

use glam::DVec2;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use thiserror::Error;

use crate::tin::{Tin, TinIndex, point_in_polygon};

/// One side of a volume calculation.
#[derive(Clone, Copy, Debug)]
pub enum Datum<'a> {
    Surface(&'a Tin),
    Level(f64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeResult {
    /// Top above base.
    pub fill: f64,
    /// Top below base (positive number).
    pub cut: f64,
    pub fill_area: f64,
    pub cut_area: f64,
    /// Plan area compared (both datums present, inside the boundary).
    pub area: f64,
}

impl VolumeResult {
    /// Fill minus cut.
    pub fn net(&self) -> f64 {
        self.fill - self.cut
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum VolumeError {
    #[error("Choose at least one surface (two levels have no extent)")]
    NoSurface,
    #[error("The surfaces do not overlap inside the boundary")]
    NoOverlap,
}

/// Volume between `base` and `top`; also returns the difference surface
/// (`z` = top − base) over the compared area.
pub fn volume(
    base: Datum<'_>,
    top: Datum<'_>,
    boundary: Option<&[DVec2]>,
) -> Result<(VolumeResult, Tin), VolumeError> {
    let tins: Vec<&Tin> = [base, top]
        .into_iter()
        .filter_map(|d| match d {
            Datum::Surface(t) => Some(t),
            Datum::Level(_) => None,
        })
        .collect();
    let first = tins.first().ok_or(VolumeError::NoSurface)?;
    let origin = first.vertices.first().map(|v| v.truncate()).unwrap_or_default();
    let rel = |p: DVec2| Point2::new(p.x - origin.x, p.y - origin.y);

    let mut cdt: ConstrainedDelaunayTriangulation<Point2<f64>> = ConstrainedDelaunayTriangulation::new();
    let mut handles: Vec<Vec<Option<spade::handles::FixedVertexHandle>>> = Vec::new();
    for t in &tins {
        handles.push(
            t.vertices
                .iter()
                .map(|v| cdt.insert(rel(v.truncate())).ok())
                .collect(),
        );
    }
    let ring: Vec<spade::handles::FixedVertexHandle> = boundary
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| cdt.insert(rel(*p)).ok())
        .collect();
    // Every TIN edge and the boundary become constraints so the merged
    // triangles lie wholly within one triangle of each surface.
    for (t, hs) in tins.iter().zip(&handles) {
        for [a, b] in t.edges() {
            if let (Some(ha), Some(hb)) = (hs[a as usize], hs[b as usize])
                && ha != hb
            {
                cdt.add_constraint_and_split(ha, hb, |p| p);
            }
        }
    }
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        if a != b {
            cdt.add_constraint_and_split(a, b, |p| p);
        }
    }

    let indexes: Vec<TinIndex> = tins.iter().map(|t| t.index()).collect();
    let height = |d: Datum<'_>, p: DVec2| -> Option<f64> {
        match d {
            Datum::Level(z) => Some(z),
            Datum::Surface(t) => {
                let k = tins.iter().position(|x| std::ptr::eq(*x, t))?;
                indexes[k].height_at(t, p)
            }
        }
    };
    let boundary_rel: Option<Vec<DVec2>> = boundary.map(|b| b.iter().map(|p| *p - origin).collect());

    let mut result = VolumeResult::default();
    let mut diff = Tin::default();
    for face in cdt.inner_faces() {
        let corners = face.vertices().map(|v| {
            let p = v.position();
            DVec2::new(p.x, p.y)
        });
        let centroid = (corners[0] + corners[1] + corners[2]) / 3.0;
        if boundary_rel
            .as_ref()
            .is_some_and(|b| !point_in_polygon(centroid, b))
        {
            continue;
        }
        let abs = |p: DVec2| p + origin;
        // Both datums must cover the triangle (tested at its centroid).
        if height(base, abs(centroid)).is_none() || height(top, abs(centroid)).is_none() {
            continue;
        }
        let d: Option<Vec<f64>> = corners
            .iter()
            .map(|c| {
                // Corners lie on both surfaces; nudge towards the centroid if a
                // lookup lands just outside through rounding.
                let at = |p: DVec2| Some(height(top, abs(p))? - height(base, abs(p))?);
                at(*c).or_else(|| at(*c + (centroid - *c) * 1e-9))
            })
            .collect();
        let Some(d) = d else {
            continue;
        };
        let (fill, cut, fill_area, cut_area, area) = integrate(corners, [d[0], d[1], d[2]]);
        result.fill += fill;
        result.cut += cut;
        result.fill_area += fill_area;
        result.cut_area += cut_area;
        result.area += area;
        let base_index = diff.vertices.len() as u32;
        for (c, dz) in corners.iter().zip(&d) {
            diff.vertices.push(abs(*c).extend(*dz));
        }
        diff.triangles.push([base_index, base_index + 1, base_index + 2]);
    }
    if diff.triangles.is_empty() {
        return Err(VolumeError::NoOverlap);
    }
    Ok((result, weld(diff)))
}

/// Exact integral of a linear function (values `d` at the corners) over a
/// triangle: (positive volume, negative volume as a positive number,
/// positive area, negative area, total area).
fn integrate(p: [DVec2; 3], d: [f64; 3]) -> (f64, f64, f64, f64, f64) {
    let area = |poly: &[DVec2]| -> f64 {
        let n = poly.len();
        (0..n)
            .map(|i| poly[i].perp_dot(poly[(i + 1) % n]))
            .sum::<f64>()
            .abs()
            / 2.0
    };
    let total = area(&p);
    // Clip the triangle at d = 0 into its positive and negative parts, carrying d.
    let mut pos: Vec<(DVec2, f64)> = Vec::new();
    let mut neg: Vec<(DVec2, f64)> = Vec::new();
    for i in 0..3 {
        let (a, da) = (p[i], d[i]);
        let (b, db) = (p[(i + 1) % 3], d[(i + 1) % 3]);
        if da >= 0.0 {
            pos.push((a, da));
        }
        if da <= 0.0 {
            neg.push((a, da));
        }
        if (da > 0.0 && db < 0.0) || (da < 0.0 && db > 0.0) {
            let t = da / (da - db);
            let x = (a + (b - a) * t, 0.0);
            pos.push(x);
            neg.push(x);
        }
    }
    // The integral of a linear function over a polygon: fan triangles from the
    // first vertex, each contributing its area × the mean of its corner values.
    let part = |poly: &[(DVec2, f64)]| -> (f64, f64) {
        if poly.len() < 3 {
            return (0.0, 0.0);
        }
        let mut vol = 0.0;
        let mut a_sum = 0.0;
        for k in 1..poly.len() - 1 {
            let tri = [poly[0], poly[k], poly[k + 1]];
            let a = area(&[tri[0].0, tri[1].0, tri[2].0]);
            vol += a * (tri[0].1 + tri[1].1 + tri[2].1) / 3.0;
            a_sum += a;
        }
        (vol, a_sum)
    };
    let (fill, fill_area) = part(&pos);
    let (cut, cut_area) = part(&neg);
    (fill, -cut, fill_area, cut_area, total)
}

/// Merges coincident vertices of a triangle soup.
fn weld(t: Tin) -> Tin {
    let mut map: std::collections::HashMap<(i64, i64), u32> = std::collections::HashMap::new();
    let mut out = Tin::default();
    let mut remap = Vec::with_capacity(t.vertices.len());
    for v in &t.vertices {
        // Millimetre keys; the difference height of shared corners is the same.
        let k = ((v.x * 1000.0).round() as i64, (v.y * 1000.0).round() as i64);
        let i = *map.entry(k).or_insert_with(|| {
            out.vertices.push(*v);
            out.vertices.len() as u32 - 1
        });
        remap.push(i);
    }
    out.triangles = t
        .triangles
        .iter()
        .map(|tri| tri.map(|i| remap[i as usize]))
        .collect();
    out
}

/// Volume of a stockpile or depression against a level, as a convenience.
pub fn against_level(
    surface: &Tin,
    level: f64,
    boundary: Option<&[DVec2]>,
) -> Result<VolumeResult, VolumeError> {
    volume(Datum::Level(level), Datum::Surface(surface), boundary).map(|(r, _)| r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tin::{TinInput, build};
    use glam::DVec3;

    const O: DVec3 = DVec3::new(512_000.0, 6_912_000.0, 0.0);

    fn grid(f: impl Fn(f64, f64) -> f64, n: usize, step: f64) -> Tin {
        let mut points = Vec::new();
        for i in 0..=n {
            for j in 0..=n {
                let (x, y) = (i as f64 * step, j as f64 * step);
                points.push(O + DVec3::new(x, y, f(x, y)));
            }
        }
        build(&TinInput {
            points,
            ..Default::default()
        })
        .expect("tin")
    }

    #[test]
    fn flat_surfaces() {
        // 100 × 100 at 10 and 12: 20 000 m³ of fill.
        let base = grid(|_, _| 10.0, 10, 10.0);
        let top = grid(|_, _| 12.0, 10, 10.0);
        let (r, diff) = volume(Datum::Surface(&base), Datum::Surface(&top), None).expect("volume");
        assert!((r.fill - 20_000.0).abs() < 1e-6, "{r:?}");
        assert!(r.cut.abs() < 1e-9 && (r.area - 10_000.0).abs() < 1e-6);
        assert!(diff.vertices.iter().all(|v| (v.z - 2.0).abs() < 1e-9));
        // Swapped: all cut.
        let (r, _) = volume(Datum::Surface(&top), Datum::Surface(&base), None).expect("volume");
        assert!((r.cut - 20_000.0).abs() < 1e-6 && r.fill.abs() < 1e-9);
    }

    #[test]
    fn plane_through_a_level_splits_exactly() {
        // z = x/10 over 0..100: against level 5, fill (above) = cut = ½ · 50 · 5 · 100.
        let s = grid(|x, _| x / 10.0, 10, 10.0);
        let r = against_level(&s, 5.0, None).expect("volume");
        assert!((r.fill - 12_500.0).abs() < 1e-6, "{r:?}");
        assert!((r.cut - 12_500.0).abs() < 1e-6);
        assert!((r.fill_area - 5000.0).abs() < 1e-6 && (r.cut_area - 5000.0).abs() < 1e-6);
    }

    #[test]
    fn misaligned_surfaces_and_boundary() {
        // Two different triangulations of non-planar surfaces: the merged result is
        // exact for the TIN surfaces; compare against a fine numeric integral.
        let base = grid(|x, y| 0.01 * x * y / 10.0, 5, 20.0);
        let top = grid(|x, y| 5.0 + ((x - 50.0) / 25.0).sin() + y / 50.0, 7, 100.0 / 7.0);
        let (r, diff) = volume(Datum::Surface(&base), Datum::Surface(&top), None).expect("volume");
        let (bi, ti) = (base.index(), top.index());
        let mut numeric = 0.0;
        let n = 400;
        let h = 100.0 / n as f64;
        for i in 0..n {
            for j in 0..n {
                let p = O.truncate() + DVec2::new((i as f64 + 0.5) * h, (j as f64 + 0.5) * h);
                numeric += (ti.height_at(&top, p).expect("t") - bi.height_at(&base, p).expect("b")) * h * h;
            }
        }
        assert!((r.net() - numeric).abs() < 0.5, "{} vs {numeric}", r.net());
        assert!((r.area - 10_000.0).abs() < 1e-6);
        assert!(!diff.triangles.is_empty());

        // Inside a 50 × 50 boundary of the flat case: 5000 m³.
        let b = [(25.0, 25.0), (75.0, 25.0), (75.0, 75.0), (25.0, 75.0)]
            .map(|(x, y)| O.truncate() + DVec2::new(x, y));
        let flat0 = grid(|_, _| 10.0, 10, 10.0);
        let flat2 = grid(|_, _| 12.0, 10, 10.0);
        let (r, _) = volume(Datum::Surface(&flat0), Datum::Surface(&flat2), Some(&b)).expect("volume");
        assert!(
            (r.fill - 5000.0).abs() < 1e-6 && (r.area - 2500.0).abs() < 1e-6,
            "{r:?}"
        );
        assert_eq!(
            volume(Datum::Level(1.0), Datum::Level(2.0), None).map(|x| x.0),
            Err(VolumeError::NoSurface)
        );
    }
}
