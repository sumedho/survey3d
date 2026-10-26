//! Dimensions (`dim`): pick the two points to measure between, then where the
//! dimension line goes (a point, or an offset typed as a distance: positive to
//! the left of first → second). `a`, `h` and `v` switch between aligned,
//! horizontal (eastings) and vertical (northings) dimensions.

use egui_cad::snap::Pick;
use glam::DVec3;
use survey_core::Conventions;
use survey_core::point_input::ResolvedPoint;
use survey_model::{ChangeSet, Dimension, DimensionKind, Entity};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "dim";

#[derive(Default)]
pub struct DimTool {
    kind: DimensionKind,
    from: Option<ResolvedPoint>,
    to: Option<ResolvedPoint>,
}

impl DimTool {
    fn dimension(&self, ctx: &ToolCtx<'_>, line_at: DVec3) -> Option<Dimension> {
        Some(Dimension {
            kind: self.kind,
            from: self.from?.position,
            to: self.to?.position,
            line_at,
            text: String::new(),
            layer: ctx.doc.current_layer(),
        })
    }

    fn place(&mut self, ctx: &mut ToolCtx<'_>, line_at: DVec3) {
        let Some(d) = self.dimension(ctx, line_at) else {
            return;
        };
        if d.measured() < 1e-9 {
            ctx.log
                .warn("Nothing to measure: the points are level in this direction");
            return;
        }
        let text = crate::dimension::dimension_text(&d, ctx.conventions());
        ctx.execute(ChangeSet::new(format!("Dimension {text}")).insert_entity(Entity::Dimension(d)));
        ctx.log.result(format!("Dimension ({}) {text}", self.kind.key()));
        self.from = None;
        self.to = None;
    }

    /// The point `offset` to the left of first → second, beside the first.
    fn offset_point(&self, offset: f64) -> Option<DVec3> {
        let (a, b) = (self.from?.position, self.to?.position);
        let d = Dimension {
            kind: self.kind,
            from: a,
            to: b,
            line_at: a,
            text: String::new(),
            layer: survey_model::LayerId(0),
        };
        let mut left = d.direction().perp();
        // Horizontal and vertical lines go to the side the second point is not on.
        if self.kind != DimensionKind::Aligned && (b - a).truncate().dot(left) > 0.0 {
            left = -left;
        }
        Some(a + (left * offset).extend(0.0))
    }
}

impl Tool for DimTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        let kind = self.kind.key();
        match (self.from, self.to) {
            (None, _) => format!("Dimension ({kind}) — first point, or a / h / v:"),
            (Some(_), None) => format!("Dimension ({kind}) — second point:"),
            _ => format!("Dimension ({kind}) — where the line goes (a point, or an offset distance):"),
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.to.or(self.from)
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        matches!(text.trim(), "a" | "h" | "v") || (self.to.is_some() && conv.parse_length(text).is_ok())
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                let p = resolved(&pick);
                if self.from.is_none() {
                    self.from = Some(p);
                } else if self.to.is_none() {
                    self.to = Some(p);
                } else {
                    self.place(ctx, p.position);
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                let conv = ctx.conventions().clone();
                match text.trim() {
                    "a" => self.kind = DimensionKind::Aligned,
                    "h" => self.kind = DimensionKind::Horizontal,
                    "v" => self.kind = DimensionKind::Vertical,
                    t => match conv.parse_length(t).ok().and_then(|o| self.offset_point(o)) {
                        Some(at) => self.place(ctx, at),
                        None => ctx.log.error(format!("Unknown option '{t}'")),
                    },
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        let Some(c) = cursor else { return };
        match (self.from, self.to) {
            (Some(a), None) => out.rubber.push([a.position, c.point]),
            (Some(a), Some(b)) => {
                let d = Dimension {
                    kind: self.kind,
                    from: a.position,
                    to: b.position,
                    line_at: c.point,
                    text: String::new(),
                    layer: survey_model::LayerId(0),
                };
                let [p, q] = d.line().map(|p| p.extend(c.point.z));
                out.polylines.push(vec![a.position, p, q, b.position]);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_go_left_of_the_line_or_away_from_it() {
        let mut t = DimTool {
            from: Some(ResolvedPoint {
                position: DVec3::ZERO,
                height_known: false,
            }),
            to: Some(ResolvedPoint {
                position: DVec3::new(10.0, 10.0, 0.0),
                height_known: false,
            }),
            ..Default::default()
        };
        let p = t.offset_point(2.0).expect("point");
        assert!((p - DVec3::new(-2.0f64.sqrt(), 2.0f64.sqrt(), 0.0)).length() < 1e-12);
        t.kind = DimensionKind::Horizontal;
        assert_eq!(
            t.offset_point(2.0),
            Some(DVec3::new(0.0, -2.0, 0.0)),
            "below, away from the second point"
        );
    }
}
