//! Contours from a TIN.
//!
//! Each triangle crossed by a contour level contributes one segment between
//! two of its edges; segments are chained through shared edges into open
//! (ending on the surface boundary) or closed polylines. Vertices exactly at a
//! level count as above it, so contours never pass through a vertex twice.

use std::collections::HashMap;

use glam::DVec3;

use crate::tin::Tin;

#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    pub z: f64,
    /// An index (major) contour.
    pub major: bool,
    pub points: Vec<DVec3>,
    pub closed: bool,
}

type EdgeKey = (u32, u32);

fn key(a: u32, b: u32) -> EdgeKey {
    if a < b { (a, b) } else { (b, a) }
}

/// Contours every `interval` (levels are multiples of it); every
/// `major_every`-th level is major. At most `max_levels` levels are traced.
pub fn contours(tin: &Tin, interval: f64, major_every: u32, max_levels: usize) -> Vec<Contour> {
    if !interval.is_finite() || interval <= 0.0 || tin.triangles.is_empty() {
        return Vec::new();
    }
    let b = tin.bounds();
    let first = (b.min.z / interval).ceil() as i64;
    let last = (b.max.z / interval).floor() as i64;
    if last < first || (last - first) as usize >= max_levels {
        return Vec::new();
    }
    // Segments per level as pairs of crossed edges.
    let mut segments: HashMap<i64, Vec<(EdgeKey, EdgeKey)>> = HashMap::new();
    for t in &tin.triangles {
        let z = t.map(|i| tin.vertices[i as usize].z);
        let lo = z[0].min(z[1]).min(z[2]);
        let hi = z[0].max(z[1]).max(z[2]);
        let k0 = ((lo / interval).floor() as i64).max(first);
        let k1 = ((hi / interval).ceil() as i64).min(last);
        for k in k0..=k1 {
            let level = k as f64 * interval;
            let above = z.map(|v| v >= level);
            let crossed: Vec<EdgeKey> = (0..3)
                .filter(|&i| above[i] != above[(i + 1) % 3])
                .map(|i| key(t[i], t[(i + 1) % 3]))
                .collect();
            if crossed.len() == 2 {
                segments.entry(k).or_default().push((crossed[0], crossed[1]));
            }
        }
    }
    let mut out = Vec::new();
    let mut levels: Vec<i64> = segments.keys().copied().collect();
    levels.sort_unstable();
    for k in levels {
        let level = k as f64 * interval;
        let major = major_every > 0 && k.rem_euclid(i64::from(major_every)) == 0;
        let point = |e: EdgeKey| {
            let (a, b) = (tin.vertices[e.0 as usize], tin.vertices[e.1 as usize]);
            let t = (level - a.z) / (b.z - a.z);
            (a + (b - a) * t).truncate().extend(level)
        };
        for chain in chain(&segments[&k]) {
            let closed = chain.len() > 2 && chain.first() == chain.last();
            let mut points: Vec<DVec3> = chain.into_iter().map(point).collect();
            if closed {
                points.pop();
            }
            // A level through a vertex can trace a chain of coincident points (e.g. a peak).
            points.dedup_by(|a, b| a.distance_squared(*b) < 1e-18);
            if points.len() < 2 {
                continue;
            }
            out.push(Contour {
                z: level,
                major,
                points,
                closed,
            });
        }
    }
    out
}

/// Joins segments sharing edges into chains of edges (a closed chain repeats its first edge).
fn chain(segments: &[(EdgeKey, EdgeKey)]) -> Vec<Vec<EdgeKey>> {
    let mut by_edge: HashMap<EdgeKey, Vec<usize>> = HashMap::new();
    for (i, (a, b)) in segments.iter().enumerate() {
        by_edge.entry(*a).or_default().push(i);
        by_edge.entry(*b).or_default().push(i);
    }
    let mut used = vec![false; segments.len()];
    let mut out = Vec::new();
    let walk = |start_seg: usize, start_edge: EdgeKey, used: &mut Vec<bool>| {
        let mut edges = vec![start_edge];
        let mut seg = start_seg;
        let mut edge = start_edge;
        loop {
            used[seg] = true;
            let (a, b) = segments[seg];
            edge = if a == edge { b } else { a };
            edges.push(edge);
            match by_edge[&edge].iter().find(|s| !used[**s]) {
                Some(next) => seg = *next,
                None => break,
            }
        }
        edges
    };
    // Open chains start at boundary edges (shared by only one segment).
    let mut starts: Vec<(EdgeKey, usize)> = by_edge
        .iter()
        .filter(|(_, s)| s.len() == 1)
        .map(|(e, s)| (*e, s[0]))
        .collect();
    starts.sort_unstable();
    for (edge, seg) in starts {
        if !used[seg] {
            out.push(walk(seg, edge, &mut used));
        }
    }
    for seg in 0..segments.len() {
        if !used[seg] {
            out.push(walk(seg, segments[seg].0, &mut used));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tin::{TinInput, build};

    const O: DVec3 = DVec3::new(512_000.0, 6_912_000.0, 0.0);

    fn cone() -> Tin {
        // A cone: apex 10 m high at the centre, radius 50, ring of 36 points at 0.
        let mut points = vec![O + DVec3::new(0.0, 0.0, 10.0)];
        for i in 0..36 {
            let a = i as f64 * std::f64::consts::TAU / 36.0;
            points.push(O + DVec3::new(50.0 * a.cos(), 50.0 * a.sin(), 0.0));
        }
        build(&TinInput {
            points,
            ..Default::default()
        })
        .expect("tin")
    }

    #[test]
    fn cone_gives_closed_rings() {
        let c = contours(&cone(), 2.0, 5, 1000);
        // Levels 0..10: 0 and 10 touch vertices (counted as above), so 2,4,6,8 form rings
        // plus level 0 (on the rim, all vertices "above") and 10 (apex only) give nothing.
        let levels: Vec<f64> = c.iter().map(|k| k.z).collect();
        assert_eq!(levels, vec![2.0, 4.0, 6.0, 8.0]);
        for k in &c {
            assert!(k.closed, "level {} not closed", k.z);
            // Ring radius = 50 · (1 − z/10) (on the chords).
            let r = 50.0 * (1.0 - k.z / 10.0);
            for p in &k.points {
                let d = (*p - O).truncate().length();
                assert!(d <= r + 1e-9 && d > r * 0.99, "{d} vs {r}");
                assert_eq!(p.z, k.z);
            }
        }
        assert!(!c[0].major && c.iter().all(|k| !k.major));
        let c = contours(&cone(), 1.0, 5, 1000);
        assert!(c.iter().any(|k| k.major && k.z == 5.0));
    }

    #[test]
    fn plane_gives_open_lines() {
        let mut points = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                let (x, y) = (i as f64 * 10.0, j as f64 * 10.0);
                points.push(O + DVec3::new(x, y, 100.0 + 0.1 * x));
            }
        }
        let tin = build(&TinInput {
            points,
            ..Default::default()
        })
        .expect("tin");
        let c = contours(&tin, 1.0, 5, 1000);
        // Heights 100..104: levels 101–103 cross the plane; 104 runs along its top edge
        // (vertices on a level count as above it), and 100 gives nothing.
        assert_eq!(c.len(), 4);
        for k in &c {
            assert!(!k.closed);
            let x = k.points[0].x - O.x;
            assert!(k.points.iter().all(|p| ((p.x - O.x) - x).abs() < 1e-9));
            let ys: Vec<f64> = k.points.iter().map(|p| p.y - O.y).collect();
            let span =
                ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
            assert!((span - 40.0).abs() < 1e-9);
        }
        assert!(contours(&tin, 0.0, 5, 1000).is_empty());
        assert!(contours(&tin, 0.001, 5, 100).is_empty(), "too many levels");
    }
}
