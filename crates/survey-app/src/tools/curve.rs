//! Curve solver on the command line (`curve`): type any two elements of a
//! circular curve as `key=value` (e.g. `r=20 a=15.71`) and the rest are
//! reported; `ls=` adds spirals each side, `compound r1=… r2=…` and
//! `reverse r=… p=…` solve curve pairs. Nothing is drawn.

use survey_core::Conventions;

use super::{Tool, ToolCtx, ToolEvent, ToolOutcome};
use crate::curves::{parse_request, solve_request};

pub const ID: &str = "curve";

#[derive(Default)]
pub struct CurveTool;

impl Tool for CurveTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        "Curve — two of r= d= a= c= t= e= m=; spirals r= ls= d=; compound r1= r2= d1= d2=; reverse r= p= (Enter to finish):"
            .into()
    }

    fn wants_point(&self) -> bool {
        false
    }

    fn claims_text(&self, _conv: &Conventions, text: &str) -> bool {
        text.contains('=')
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Text(text) => {
                let conv = ctx.conventions().clone();
                match parse_request(&conv, text).and_then(|r| solve_request(&conv, r)) {
                    Ok(solutions) => {
                        let n = solutions.len();
                        for (i, c) in solutions.iter().enumerate() {
                            let head = if n > 1 {
                                format!("Curve solution {} of {n}:\n", i + 1)
                            } else {
                                "Curve:\n".to_string()
                            };
                            ctx.log.result(format!("{head}{c}"));
                        }
                    }
                    Err(e) => ctx.log.error(e),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Point(_) | ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }
}
