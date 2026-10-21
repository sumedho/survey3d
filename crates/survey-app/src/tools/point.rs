use egui_cad::snap::Pick;
use survey_core::Conventions;
use survey_core::point_input::ResolvedPoint;
use survey_model::{ChangeSet, Entity, SurveyPoint};

use super::{Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const ID: &str = "point";

/// Creates a point at each pick. Typing text that is not a point sets the code
/// for following points (`-` clears it).
#[derive(Default)]
pub struct PointTool {
    code: String,
    last: Option<ResolvedPoint>,
}

impl Tool for PointTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        if self.code.is_empty() {
            "Point — pick or enter coordinates [code]:".into()
        } else {
            format!("Point ({}) — pick or enter coordinates [code]:", self.code)
        }
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.last
    }

    fn takes_codes(&self) -> bool {
        true
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                self.add_point(ctx, &pick);
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                self.code = if text.trim() == "-" {
                    String::new()
                } else {
                    text.trim().to_string()
                };
                ctx.log.info(if self.code.is_empty() {
                    "Code cleared".to_string()
                } else {
                    format!("Code set to {}", self.code)
                });
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }
}

impl PointTool {
    fn add_point(&mut self, ctx: &mut ToolCtx<'_>, pick: &Pick) {
        let doc = ctx.doc;
        let name = doc.next_point_name();
        let point = SurveyPoint {
            name: name.clone(),
            position: pick.point,
            height_known: pick.height_known,
            code: self.code.clone(),
            description: String::new(),
            layer: doc.current_layer(),
        };
        let text = doc.conventions().format_coord(pick.point, pick.height_known);
        ctx.log.result(format!("Point {name}: {text}"));
        ctx.execute(ChangeSet::new(format!("Add point {name}")).insert_entity(Entity::Point(point)));
        self.last = Some(resolved(pick));
    }
}
