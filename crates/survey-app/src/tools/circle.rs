use egui_cad::snap::Pick;
use glam::DVec3;
use survey_core::Conventions;
use survey_core::arc::Arc;
use survey_core::point_input::ResolvedPoint;
use survey_model::{ChangeSet, Entity, Polyline};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "circle";

/// Circle from a centre and a radius (picked or typed). Stored as a closed
/// polyline of two semicircular arc segments, as DXF does.
#[derive(Default)]
pub struct CircleTool {
    centre: Option<ResolvedPoint>,
}

/// The closed two-arc polyline for a circle.
pub fn circle_polyline(centre: DVec3, radius: f64) -> (Vec<DVec3>, Vec<f64>) {
    let dx = DVec3::X * radius;
    (vec![centre + dx, centre - dx], vec![1.0, 1.0])
}

impl Tool for CircleTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        if self.centre.is_none() {
            "Circle — centre point:".into()
        } else {
            "Circle — radius (pick a point or type a distance):".into()
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.centre
    }

    fn wants_distance(&self) -> bool {
        self.centre.is_some()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                match self.centre {
                    None => self.centre = Some(resolved(&pick)),
                    Some(c) => {
                        let r = c.position.truncate().distance(pick.point.truncate());
                        self.commit(ctx, r);
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                match (self.centre, ctx.conventions().parse_length(text)) {
                    (Some(_), Ok(r)) => self.commit(ctx, r),
                    (None, _) => ctx.log.error("Pick or type the centre point first"),
                    (Some(_), Err(e)) => ctx.log.error(e.to_string()),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => {
                if self.centre.take().is_some() {
                    ToolOutcome::Continue
                } else {
                    ToolOutcome::Done
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        if let (Some(c), Some(p)) = (self.centre, cursor) {
            let r = c.position.truncate().distance(p.point.truncate());
            if r > 0.0 {
                let (v, b) = circle_polyline(c.position, r);
                for (a, e, bulge) in [(v[0], v[1], b[0]), (v[1], v[0], b[1])] {
                    if let Some(arc) = Arc::from_bulge(a, e, bulge) {
                        out.polylines.push(arc.tessellate(r * 1e-3));
                    }
                }
            }
            out.rubber.push([c.position, p.point]);
        }
    }
}

impl CircleTool {
    fn commit(&mut self, ctx: &mut ToolCtx<'_>, radius: f64) {
        let Some(c) = self.centre.take() else {
            return;
        };
        if radius <= 0.0 {
            ctx.log.error("Radius must be greater than zero");
            self.centre = Some(c);
            return;
        }
        let (vertices, bulges) = circle_polyline(c.position, radius);
        ctx.log.result(format!(
            "Circle: radius {}",
            ctx.conventions().format_length_with_unit(radius)
        ));
        let line = Polyline {
            name: String::new(),
            vertices,
            bulges,
            closed: true,
            code: String::new(),
            layer: ctx.doc.current_layer(),
        };
        ctx.execute(ChangeSet::new("Add circle").insert_entity(Entity::Polyline(line)));
    }
}
