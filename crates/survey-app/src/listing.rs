//! Text listings of objects for the Reports window.

use std::fmt::Write as _;

use survey_core::arc::{Arc, polyline_metrics};
use survey_core::cogo::inverse;
use survey_model::{Document, Polyline};

/// Legs of a polyline: bearing and distance of each straight segment; radius,
/// arc length, chord bearing and chord distance of each arc; then totals.
/// Lengths and the area are grid or ground as the project shows them.
pub fn polyline_listing(l: &Polyline, doc: &Document) -> String {
    let conv = doc.conventions();
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
    let mut total = 0.0;
    for (i, (a, b, bulge)) in l.segments().enumerate() {
        let chord = inverse(a, b);
        // Grid to shown (1 on the grid).
        let k = doc.shown_length(1.0, a, b);
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
                    conv.format_length(arc.radius * k),
                    conv.format_length(arc.length_2d() * k),
                    brg,
                    conv.format_length(chord.horizontal * k)
                );
                total += arc.length_2d() * k;
            }
            None => {
                let _ = writeln!(
                    s,
                    "  {:>3}  {}  {}",
                    i + 1,
                    brg,
                    conv.format_length(chord.horizontal * k)
                );
                total += chord.horizontal * k;
            }
        }
    }
    let (_, _, area) = polyline_metrics(&l.vertices, &l.bulges, l.closed);
    let kind = if doc.ground_distances() { " (ground)" } else { "" };
    let _ = writeln!(s, "  length {}{kind}", conv.format_length_with_unit(total));
    if l.closed {
        let centre = l.vertices.iter().sum::<glam::DVec3>() / l.vertices.len() as f64;
        let area = doc.shown_area(area.abs(), centre);
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
