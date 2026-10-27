//! Alignments: the geometry of an alignment record (its centreline with
//! transitions fitted), the element table and set-out tables as text.

use std::fmt::Write as _;

use survey_core::Conventions;
use survey_core::alignment::{ElementKind, Geometry, SetoutPoint};
use survey_model::{AlignmentRecord, Document, Entity, Polyline};

/// An alignment's centreline, if it still exists.
pub fn centreline<'a>(doc: &'a Document, a: &AlignmentRecord) -> Option<&'a Polyline> {
    match doc.entity(a.polyline) {
        Some(Entity::Polyline(l)) => Some(l),
        _ => None,
    }
}

/// The alignment's geometry (transitions fitted), or why it cannot be built;
/// `None` when the centreline has gone.
pub fn geometry(doc: &Document, a: &AlignmentRecord) -> Option<Result<Geometry, String>> {
    let l = centreline(doc, a)?;
    Some(Geometry::new(
        &l.vertices,
        &l.bulges,
        l.closed,
        a.start_chainage,
        &a.transitions,
    ))
}

/// The element table as text.
pub fn element_report(doc: &Document, a: &AlignmentRecord) -> Option<String> {
    let g = match geometry(doc, a)? {
        Ok(g) => g,
        Err(e) => return Some(format!("ALIGNMENT  {}\n\n{e}\n", a.name)),
    };
    let conv = doc.conventions();
    let len = |v: f64| conv.format_length(v);
    let angle = |v: f64| conv.format_angle(survey_core::Angle::from_radians(v));
    let mut s = String::new();
    let _ = writeln!(s, "ALIGNMENT  {}", a.name);
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{:<9} {:>12} {:>12} {:>10}  {:<46} Start",
        "Element", "Start ch", "End ch", "Length", "Bearing / radius"
    );
    for e in &g.elements {
        let (kind, detail) = match e.kind {
            ElementKind::Straight { bearing } => ("Straight", conv.format_bearing(bearing)),
            ElementKind::Arc {
                radius, right, delta, ..
            } => (
                "Arc",
                format!(
                    "R {} {} Δ {}",
                    len(radius),
                    if right { "right" } else { "left" },
                    angle(delta)
                ),
            ),
            ElementKind::Spiral {
                radius,
                right,
                entering,
                ..
            } => {
                let sp = survey_core::spiral::Spiral::new(radius, e.length);
                (
                    "Spiral",
                    format!(
                        "{} R {} {} A {} θs {}",
                        if entering { "in" } else { "out" },
                        len(radius),
                        if right { "right" } else { "left" },
                        len(sp.a),
                        angle(sp.theta)
                    ),
                )
            }
        };
        let _ = writeln!(
            s,
            "{:<9} {:>12} {:>12} {:>10}  {:<46} {}",
            kind,
            conv.format_chainage(e.start_chainage),
            conv.format_chainage(e.end_chainage()),
            len(e.length),
            detail,
            conv.format_coord(e.start, false)
        );
    }
    let names = g.key_point_names();
    if names.iter().any(|(_, n)| *n == "TS") {
        let _ = writeln!(s);
        let _ = writeln!(s, "{:<6} {:>12}  Coordinates", "Point", "Chainage");
        for (ch, name) in names {
            if let Some((p, _)) = g.point_at(ch, 0.0) {
                let _ = writeln!(
                    s,
                    "{name:<6} {:>12}  {}",
                    conv.format_chainage(ch),
                    conv.format_coord(p, false)
                );
            }
        }
    }
    Some(s)
}

/// The centreline's arcs: (segment, radius, turns right).
pub fn arcs(l: &Polyline) -> Vec<(usize, f64, bool)> {
    l.segments()
        .enumerate()
        .filter_map(|(i, (a, b, bulge))| {
            survey_core::arc::Arc::from_bulge(a, b, bulge).map(|arc| (i, arc.radius, arc.sweep < 0.0))
        })
        .collect()
}

/// A set-out table as text.
pub fn setout_report(conv: &Conventions, name: &str, pts: &[SetoutPoint], names: &[String]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "SET-OUT  {name}");
    let (ea, nb) = conv.horizontal_labels();
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{:<8} {:>12} {:>10}  {:>14} {:>14}  Note",
        "Point", "Chainage", "Offset", ea, nb
    );
    for (p, n) in pts.iter().zip(names) {
        let (a, b) = conv.horizontal_values(p.position);
        let _ = writeln!(
            s,
            "{:<8} {:>12} {:>10}  {:>14} {:>14}  {}",
            n,
            conv.format_chainage(p.chainage),
            conv.format_length(p.offset),
            conv.format_length(a),
            conv.format_length(b),
            p.key
        );
    }
    s
}

/// "L5.000" / "R5.000" / "CL" for an offset.
pub fn offset_label(conv: &Conventions, offset: f64) -> String {
    if offset.abs() < 1e-9 {
        "CL".into()
    } else if offset < 0.0 {
        format!("L{}", conv.format_length(-offset))
    } else {
        format!("R{}", conv.format_length(offset))
    }
}
