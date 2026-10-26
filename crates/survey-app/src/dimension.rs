//! How a dimension is drawn: extension lines, a dimension line with arrows and
//! its length as text, laid out in a 2D drawing space (screen pixels or paper
//! millimetres) so arrows and text keep their size at any zoom or scale.

use glam::DVec2;
use survey_core::Conventions;
use survey_model::Dimension;

/// Sizes on paper (millimetres); the screen draws them at its pixels per mm.
const GAP: f64 = 1.0;
const OVERSHOOT: f64 = 1.5;
const ARROW: f64 = 2.2;
const ARROW_HALF_WIDTH: f64 = 0.55;
const TEXT_GAP: f64 = 0.8;

/// A dimension laid out in a drawing space.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionDrawing {
    /// Extension lines and the dimension line.
    pub lines: Vec<[DVec2; 2]>,
    /// Filled arrowheads (tip first).
    pub arrows: Vec<[DVec2; 3]>,
    pub text: String,
    /// The middle of the text.
    pub text_at: DVec2,
    /// Reading direction (radians in the drawing space), kept upright.
    pub angle: f64,
}

/// The text a dimension shows: its own, with `<>` standing for the measured
/// length, or the measured length.
pub fn dimension_text(d: &Dimension, conv: &Conventions) -> String {
    let measured = conv.format_length(d.measured());
    if d.text.trim().is_empty() {
        measured
    } else {
        d.text.replace("<>", &measured)
    }
}

/// Lays out `d` with `map` taking world plan points to the drawing space,
/// `unit` drawing units per paper millimetre, `text_height` the text's capital
/// height (drawing units) and `y_up` whether the space's y axis points up.
pub fn layout(
    d: &Dimension,
    conv: &Conventions,
    map: impl Fn(DVec2) -> DVec2,
    unit: f64,
    text_height: f64,
    y_up: bool,
) -> Option<DimensionDrawing> {
    let [p, q] = d.line().map(&map);
    let (a, b) = (map(d.from.truncate()), map(d.to.truncate()));
    let length = p.distance(q);
    let u = (q - p).try_normalize()?;
    let mut lines = Vec::new();
    for (from, foot) in [(a, p), (b, q)] {
        let reach = foot - from;
        if let Some(dir) = reach.try_normalize()
            && reach.length() > GAP * unit
        {
            lines.push([from + dir * GAP * unit, foot + dir * OVERSHOOT * unit]);
        }
    }
    lines.push([p, q]);
    // Arrows inside the line, or outside it when it is too short for them.
    let inside = length >= 3.0 * ARROW * unit;
    let w = u.perp() * ARROW_HALF_WIDTH * unit;
    let arrows = [(p, u), (q, -u)]
        .map(|(tip, inward)| {
            let back = tip + inward * if inside { ARROW } else { -ARROW } * unit;
            [tip, back + w, back - w]
        })
        .to_vec();
    if !inside {
        for [tip, ..] in &arrows {
            let out = (*tip - (p + q) * 0.5).normalize_or_zero();
            lines.push([*tip, *tip + out * ARROW * 1.6 * unit]);
        }
    }
    // Upright: reading left to right, or upwards on the page.
    let up_sign = if y_up { 1.0 } else { -1.0 };
    let reads = if u.x.abs() > 1e-9 {
        u.x > 0.0
    } else {
        u.y * up_sign > 0.0
    };
    let along = if reads { u } else { -u };
    // The text sits on the side of the line away from the measured points
    // (above it when the line runs through them).
    let mid = (p + q) * 0.5;
    let toward = mid - (a + b) * 0.5;
    let above = along.perp() * up_sign;
    let side = if toward.length() > 1e-9 * unit.max(1e-9) && toward.dot(above) < 0.0 {
        -above
    } else {
        above
    };
    let text_at = mid + side * (TEXT_GAP * unit + text_height / 2.0);
    Some(DimensionDrawing {
        lines,
        arrows,
        text: dimension_text(d, conv),
        text_at,
        angle: along.y.atan2(along.x),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use survey_model::{DimensionKind, LayerId};

    fn dim(kind: DimensionKind, line_at: DVec3) -> Dimension {
        Dimension {
            kind,
            from: DVec3::new(0.0, 0.0, 0.0),
            to: DVec3::new(20.0, 0.0, 0.0),
            line_at,
            text: String::new(),
            layer: LayerId(1),
        }
    }

    #[test]
    fn aligned_dimension_layout() {
        let conv = Conventions::default();
        // Paper at 1:100 (10 mm per metre), y up; the line 5 m below.
        let d = dim(DimensionKind::Aligned, DVec3::new(10.0, -5.0, 0.0));
        let drawing = layout(&d, &conv, |w| w * 10.0, 1.0, 2.0, true).expect("layout");
        assert_eq!(drawing.text, "20.000");
        assert_eq!(
            drawing.lines.len(),
            3,
            "two extension lines and the dimension line"
        );
        assert_eq!(
            drawing.lines[2],
            [DVec2::new(0.0, -50.0), DVec2::new(200.0, -50.0)]
        );
        // Extension lines leave a gap at the points and run past the line.
        assert_eq!(drawing.lines[0], [DVec2::new(0.0, -1.0), DVec2::new(0.0, -51.5)]);
        // Text below the line (away from the points), reading left to right.
        assert!(drawing.text_at.y < -50.0 && drawing.angle.abs() < 1e-12);
        assert_eq!(drawing.arrows[0][0], DVec2::new(0.0, -50.0));
        assert!(drawing.arrows[0][1].x > 0.0, "arrows inside a long line");
    }

    #[test]
    fn text_overrides_and_short_lines() {
        let conv = Conventions::default();
        let mut d = dim(DimensionKind::Horizontal, DVec3::new(0.0, 2.0, 0.0));
        d.to = DVec3::new(0.4, 3.0, 0.0);
        d.text = "<> (fence)".into();
        let drawing = layout(&d, &conv, |w| w * 10.0, 1.0, 2.0, true).expect("layout");
        assert_eq!(drawing.text, "0.400 (fence)");
        assert!(drawing.arrows[0][1].x < 0.0, "arrows outside a short line");
        // A vertical dimension reads upwards (screen space: y down).
        let v = dim(DimensionKind::Vertical, DVec3::new(-3.0, 0.0, 0.0));
        let mut v = v;
        v.to = DVec3::new(0.0, 20.0, 0.0);
        let drawing = layout(&v, &conv, |w| DVec2::new(w.x, -w.y) * 10.0, 4.0, 8.0, false).expect("layout");
        assert!(
            (drawing.angle + std::f64::consts::FRAC_PI_2).abs() < 1e-12,
            "{}",
            drawing.angle
        );
        assert!(
            drawing.text_at.x < -30.0,
            "left of the line, away from the points"
        );
    }
}
