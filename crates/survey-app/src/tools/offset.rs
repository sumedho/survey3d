use egui_cad::snap::Pick;
use survey_core::Conventions;
use survey_core::cogo::station_offset;
use survey_core::offset::offset_polyline;
use survey_model::{ChangeSet, Entity, Polyline};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome};

pub const ID: &str = "offset";

/// Parallel copies of polylines: type a distance (or `t` to offset through a
/// point), pick a polyline, then pick the side. Repeats until Escape.
#[derive(Default)]
pub struct OffsetTool {
    /// `None` = through a picked point.
    distance: Option<f64>,
    distance_set: bool,
    source: Option<Polyline>,
}

impl OffsetTool {
    /// Offset of the source towards (or through) `p`, as a signed right offset.
    fn result(&self, p: &Pick) -> Option<Result<Polyline, String>> {
        let line = self.source.as_ref()?;
        let so = station_offset(&line.vertices, &line.bulges, line.closed, p.point.truncate())?;
        let side = if so.offset >= 0.0 { 1.0 } else { -1.0 };
        let distance = self.distance.map_or(so.offset, |d| d * side);
        Some(
            offset_polyline(&line.vertices, &line.bulges, line.closed, distance)
                .map(|(vertices, bulges)| Polyline {
                    name: String::new(),
                    vertices,
                    bulges,
                    ..line.clone()
                })
                .map_err(|e| e.to_string()),
        )
    }
}

impl Tool for OffsetTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, conv: &Conventions) -> String {
        if !self.distance_set {
            return "Offset — distance [Through]:".into();
        }
        let how = self.distance.map_or_else(
            || "through point".to_string(),
            |d| conv.format_length_with_unit(d),
        );
        match self.source {
            None => format!("Offset ({how}) — pick a polyline:"),
            Some(_) if self.distance.is_none() => "Offset — pick the point to pass through:".into(),
            Some(_) => "Offset — pick a point on the side to offset:".into(),
        }
    }

    fn wants_point(&self) -> bool {
        self.source.is_some()
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        !self.distance_set && conv.parse_length(text).is_ok()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Text(text) if !self.distance_set => {
                let t = text.trim().to_ascii_lowercase();
                if t == "t" || t == "through" {
                    self.distance = None;
                    self.distance_set = true;
                } else {
                    match ctx.conventions().parse_length(&t) {
                        Ok(d) if d > 0.0 => {
                            self.distance = Some(d);
                            self.distance_set = true;
                        }
                        Ok(_) => ctx.log.error("The offset distance must be greater than zero"),
                        Err(e) => ctx.log.error(e.to_string()),
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(other) => {
                ctx.log.error(format!("Unknown option '{other}'"));
                ToolOutcome::Continue
            }
            ToolEvent::Point(pick) => {
                if !self.distance_set {
                    ctx.log
                        .warn("Type the offset distance first (or T to offset through a point)");
                } else if self.source.is_none() {
                    match ctx.hovered.and_then(|id| ctx.doc.entity(id)) {
                        Some(Entity::Polyline(l)) => self.source = Some(l.clone()),
                        Some(_) => ctx.log.warn("Only polylines can be offset"),
                        None => {}
                    }
                } else {
                    match self.result(&pick) {
                        Some(Ok(line)) => {
                            ctx.log
                                .result(format!("Offset: {} vertices", line.vertices.len()));
                            ctx.execute(
                                ChangeSet::new("Offset polyline").insert_entity(Entity::Polyline(line)),
                            );
                            self.source = None;
                        }
                        Some(Err(e)) => ctx.log.error(format!("Cannot offset: {e}")),
                        None => self.source = None,
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) | ToolEvent::Finish => ToolOutcome::Continue,
            ToolEvent::Cancel => {
                if self.source.take().is_some() {
                    ToolOutcome::Continue
                } else {
                    ToolOutcome::Done
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        if let Some(Some(Ok(line))) = cursor.map(|c| self.result(c)) {
            let size = line
                .vertices
                .iter()
                .fold(survey_core::Aabb3::EMPTY, |mut b, v| {
                    b.extend(*v);
                    b
                })
                .size()
                .truncate()
                .length();
            out.polylines.extend(line.tessellate((size * 1e-4).max(1e-4)));
        }
    }
}
