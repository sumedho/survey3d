use survey_core::Conventions;
use survey_model::{ChangeSet, Entity, TextAlign, TextNote};

use super::{Tool, ToolCtx, ToolEvent, ToolOutcome};

pub const ID: &str = "text";

/// Places text: pick (or type) where it goes, then type it. Each line typed
/// adds a note there, centred, read left to right; change its size, angle and
/// alignment in the properties panel.
#[derive(Default)]
pub struct TextTool {
    at: Option<glam::DVec3>,
}

impl Tool for TextTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match self.at {
            None => "Text — position (centre of the text):".into(),
            Some(_) => "Text — type the text (Enter to finish):".into(),
        }
    }

    fn wants_point(&self) -> bool {
        self.at.is_none()
    }

    /// Once placed, anything typed is the text itself (even a command's name).
    fn claims_text(&self, _conv: &Conventions, text: &str) -> bool {
        self.at.is_some() && !text.trim().is_empty()
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Point(pick) => {
                self.at = Some(pick.point);
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                let Some(at) = self.at.take() else {
                    ctx.log.error(format!("Unknown option '{text}'"));
                    return ToolOutcome::Continue;
                };
                let note = TextNote {
                    text: text.to_string(),
                    position: at,
                    height: TextNote::DEFAULT_HEIGHT,
                    rotation: 0.0,
                    align: TextAlign::Centre,
                    layer: ctx.doc.current_layer(),
                };
                ctx.execute(ChangeSet::new(format!("Add text \"{text}\"")).insert_entity(Entity::Text(note)));
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }
}
