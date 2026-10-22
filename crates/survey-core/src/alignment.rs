//! Horizontal alignments from polylines: the element table (straights and
//! arcs with their chainages) and set-out points at intervals and offsets.

use glam::{DVec2, DVec3};

use crate::angle::Bearing;
use crate::arc::{Arc, bulge_segments};
use crate::cogo::point_at_station;

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
}

impl Element {
    pub fn end_chainage(&self) -> f64 {
        self.start_chainage + self.length
    }
}

/// The elements of a polyline alignment starting at `start_chainage`.
pub fn elements(vertices: &[DVec3], bulges: &[f64], closed: bool, start_chainage: f64) -> Vec<Element> {
    let mut out = Vec::new();
    let mut ch = start_chainage;
    for (a, b, bulge) in bulge_segments(vertices, bulges, closed) {
        let (length, kind) = match Arc::from_bulge(a, b, bulge) {
            Some(arc) => {
                let t = arc.tangent_at(0.0);
                (
                    arc.length_2d(),
                    ElementKind::Arc {
                        radius: arc.radius,
                        right: arc.sweep < 0.0,
                        centre: arc.center,
                        start_bearing: Bearing::from_delta(t.x, t.y).unwrap_or_default(),
                        delta: arc.sweep.abs(),
                    },
                )
            }
            None => {
                let d = (b - a).truncate();
                let Some(bearing) = Bearing::from_delta(d.x, d.y) else {
                    continue;
                };
                (d.length(), ElementKind::Straight { bearing })
            }
        };
        out.push(Element {
            start_chainage: ch,
            length,
            start: a,
            end: b,
            kind,
        });
        ch += length;
    }
    out
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
}

/// Set-out points every `interval` along the alignment (chainages that are
/// multiples of the interval) at each offset, plus the element ends when
/// `key_points` is set. Heights are interpolated along the polyline.
pub fn setout(
    vertices: &[DVec3],
    bulges: &[f64],
    closed: bool,
    start_chainage: f64,
    interval: f64,
    offsets: &[f64],
    key_points: bool,
) -> Vec<SetoutPoint> {
    let els = elements(vertices, bulges, closed, start_chainage);
    let Some(last) = els.last() else {
        return Vec::new();
    };
    let end = last.end_chainage();
    let mut chainages: Vec<(f64, bool)> = Vec::new();
    if interval > 0.0 {
        let mut k = (start_chainage / interval).ceil() as i64;
        loop {
            let ch = k as f64 * interval;
            if ch > end + 1e-9 {
                break;
            }
            chainages.push((ch, false));
            k += 1;
        }
    }
    if key_points {
        for e in &els {
            chainages.push((e.start_chainage, true));
        }
        chainages.push((end, true));
    }
    chainages.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Merge coincident chainages, keeping the key-point flag.
    let mut merged: Vec<(f64, bool)> = Vec::new();
    for (ch, key) in chainages {
        match merged.last_mut() {
            Some(last) if (last.0 - ch).abs() < 1e-6 => last.1 |= key,
            _ => merged.push((ch, key)),
        }
    }
    let mut out = Vec::new();
    for (ch, key) in merged {
        for off in offsets {
            if let Some(p) = point_at_station(vertices, bulges, closed, ch - start_chainage, *off) {
                out.push(SetoutPoint {
                    chainage: ch,
                    offset: *off,
                    position: p,
                    key_point: key,
                });
            }
        }
    }
    out
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
    }
}
