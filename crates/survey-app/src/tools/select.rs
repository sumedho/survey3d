use egui_cad::snap::Pick;
use survey_core::Conventions;

use super::{Tool, ToolCtx, ToolEvent, ToolOutcome};

pub const SELECT_TOOL_ID: &str = "select";

/// Click to select the object under the cursor; Shift toggles; clicking empty space clears.
pub struct SelectTool;

impl Tool for SelectTool {
    fn id(&self) -> &'static str {
        SELECT_TOOL_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        "Command:".into()
    }

    fn wants_point(&self) -> bool {
        false
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(Pick { .. }) => {
                match (ctx.hovered, ctx.shift) {
                    (Some(id), true) => ctx.selection.toggle(id),
                    (Some(id), false) => ctx.selection.set([id]),
                    (None, false) => ctx.selection.clear(),
                    (None, true) => {}
                }
                ToolOutcome::Continue
            }
            ToolEvent::Cancel => {
                ctx.selection.clear();
                ToolOutcome::Continue
            }
            ToolEvent::Text(_) | ToolEvent::Finish => ToolOutcome::Continue,
        }
    }
}
