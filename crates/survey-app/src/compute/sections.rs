//! Sections through surfaces: a long section along a polyline, or a cross
//! section square to it at a chainage.

use std::fmt::Write as _;

use glam::DVec2;
use survey_core::Conventions;
use survey_core::arc::polyline_metrics;
use survey_core::cogo::point_at_station;
use survey_core::tin::{TinIndex, profile};
use survey_model::{Document, Polyline, SurfaceId};

/// Chord tolerance for arcs in the alignment (metres).
const ARC_TOLERANCE: f64 = 0.01;

/// One surface's section: runs of (distance, height) where the line is over it.
#[derive(Clone, Debug)]
pub struct Series {
    pub surface: SurfaceId,
    pub name: String,
    pub runs: Vec<Vec<(f64, f64)>>,
}

/// The polyline's length (arcs included).
pub fn length(l: &Polyline) -> f64 {
    polyline_metrics(&l.vertices, &l.bulges, l.closed).0
}

/// Chainages every `interval` from the start, and the end.
pub fn stations(length: f64, interval: f64) -> Vec<f64> {
    let step = interval.max(0.01);
    let mut out: Vec<f64> = (0..=((length / step).floor() as usize))
        .map(|k| k as f64 * step)
        .collect();
    if out.last().is_some_and(|c| length - c > 1e-6) {
        out.push(length);
    }
    out
}

/// The plan line of a long section: the polyline, arcs as chords.
pub fn long_line(l: &Polyline) -> Vec<DVec2> {
    let mut pts: Vec<DVec2> = Vec::new();
    for part in l.tessellate(ARC_TOLERANCE) {
        for p in part {
            if pts.last().is_none_or(|q| q.distance(p.truncate()) > 1e-9) {
                pts.push(p.truncate());
            }
        }
    }
    pts
}

/// The plan line of a cross section at `chainage`, from `left` metres left
/// of the polyline to `right` metres right; empty off its ends.
pub fn cross_line(l: &Polyline, chainage: f64, left: f64, right: f64) -> Vec<DVec2> {
    let a = point_at_station(&l.vertices, &l.bulges, l.closed, chainage, -left);
    let b = point_at_station(&l.vertices, &l.bulges, l.closed, chainage, right);
    match (a, b) {
        (Some(a), Some(b)) => vec![a.truncate(), b.truncate()],
        _ => Vec::new(),
    }
}

/// Search indexes for every surface, for [`sections`].
pub fn indexes(doc: &Document) -> Vec<(SurfaceId, TinIndex)> {
    doc.surfaces().map(|(id, s)| (id, s.tin.index())).collect()
}

/// Each visible surface's section along `line`, distances shifted by `shift`
/// (a cross section's left offset, so the polyline is at 0).
pub fn sections(
    doc: &Document,
    indexes: &[(SurfaceId, TinIndex)],
    line: &[DVec2],
    shift: f64,
) -> Vec<Series> {
    indexes
        .iter()
        .filter_map(|(id, index)| {
            let s = doc.surface(*id).filter(|s| s.style.visible)?;
            let runs = profile(&s.tin, index, line)
                .into_iter()
                .map(|r| r.into_iter().map(|(d, z)| (d + shift, z)).collect())
                .collect();
            Some(Series {
                surface: *id,
                name: s.name.clone(),
                runs,
            })
        })
        .collect()
}

/// The height of a run at distance `x`, if the run covers it.
pub fn height_at(run: &[(f64, f64)], x: f64) -> Option<f64> {
    run.windows(2).find_map(|w| {
        let ((x0, z0), (x1, z1)) = (w[0], w[1]);
        (x >= x0 && x <= x1 && x1 > x0).then(|| z0 + (z1 - z0) * (x - x0) / (x1 - x0))
    })
}

/// The series as a table; `cross` labels distances as offsets, else chainages.
pub fn report(conv: &Conventions, title: &str, series: &[Series], cross: bool) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{}", title.to_uppercase());
    let x = if cross { "Offset" } else { "Chainage" };
    for ser in series {
        let _ = writeln!(s);
        let _ = writeln!(s, "{}", ser.name);
        let _ = writeln!(s, "{x:>12} {:>12}", "Height");
        for run in &ser.runs {
            for (d, z) in run {
                let d = if cross {
                    conv.format_length(*d)
                } else {
                    conv.format_chainage(*d)
                };
                let _ = writeln!(s, "{d:>12} {:>12}", conv.format_length(*z));
            }
            let _ = writeln!(s, "{:>12}", "—");
        }
    }
    s
}
