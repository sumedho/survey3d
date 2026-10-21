//! Text listings of objects for the Reports window.

use std::fmt::Write as _;

use survey_core::Conventions;
use survey_core::arc::{Arc, polyline_metrics};
use survey_core::cogo::inverse;
use survey_model::Polyline;

/// Legs of a polyline: bearing and distance of each straight segment; radius,
/// arc length, chord bearing and chord distance of each arc; then totals.
pub fn polyline_listing(l: &Polyline, conv: &Conventions) -> String {
    let mut s = String::new();
    let name = if l.name.is_empty() { "(unnamed)" } else { &l.name };
    let _ = writeln!(
        s,
        "POLYLINE {name}  code {}  {}",
        if l.code.is_empty() { "—" } else { &l.code },
        if l.closed { "closed" } else { "open" }
    );
    // Polylines do not record whether heights are known; show them only when present.
    let has_heights = l.vertices.iter().any(|v| v.z != 0.0);
    let _ = writeln!(s, "  start  {}", conv.format_coord(l.vertices[0], has_heights));
    for (i, (a, b, bulge)) in l.segments().enumerate() {
        let chord = inverse(a, b);
        let brg = chord
            .bearing
            .map_or_else(|| "—".to_string(), |b| conv.format_bearing(b));
        match Arc::from_bulge(a, b, bulge) {
            Some(arc) => {
                let turn = if arc.sweep > 0.0 { "left" } else { "right" };
                let _ = writeln!(
                    s,
                    "  {:>3}  arc R {}  L {}  chord {} {}  ({turn})",
                    i + 1,
                    conv.format_length(arc.radius),
                    conv.format_length(arc.length_2d()),
                    brg,
                    conv.format_length(chord.horizontal)
                );
            }
            None => {
                let _ = writeln!(
                    s,
                    "  {:>3}  {}  {}",
                    i + 1,
                    brg,
                    conv.format_length(chord.horizontal)
                );
            }
        }
    }
    let (len, _, area) = polyline_metrics(&l.vertices, &l.bulges, l.closed);
    let _ = writeln!(s, "  length {}", conv.format_length_with_unit(len));
    if l.closed {
        let unit = conv.linear_unit.metres_per_unit();
        let _ = writeln!(
            s,
            "  area   {:.*} {}²",
            usize::from(conv.linear_decimals),
            area.abs() / (unit * unit),
            conv.linear_unit.suffix()
        );
    }
    s.push('\n');
    s
}
