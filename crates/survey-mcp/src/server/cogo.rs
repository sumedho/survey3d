//! COGO calculations that add nothing to the drawing.

use serde_json::{Value, json};
use survey_app::curves::segment_area;
use survey_core::Angle;
use survey_core::solvers::{CurveInput, solve_curve};

use super::{Outcome, Server, ToolResult};

impl Server {
    /// Any two elements of a circular curve give the rest.
    pub(super) fn solve_curve(&mut self, args: &Value) -> Outcome {
        let conv = self.doc().conventions().clone();
        let length = |key: &str| -> Result<Option<f64>, String> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Number(n)) => Ok(n.as_f64().map(|v| v * conv.linear_unit.metres_per_unit())),
                Some(Value::String(s)) => conv.parse_length(s).map(Some).map_err(|e| format!("{key}: {e}")),
                Some(_) => Err(format!("{key}: a number or text")),
            }
        };
        let delta = match args.get("delta") {
            None | Some(Value::Null) => None,
            Some(Value::Number(n)) => n.as_f64().map(|d| d.to_radians()),
            Some(Value::String(s)) => Some(conv.parse_angle(s).map_err(|e| format!("delta: {e}"))?.radians()),
            Some(_) => return Err("delta: degrees as a number, or an angle as text".into()),
        };
        let input = CurveInput {
            radius: length("radius")?,
            delta: delta.map(f64::abs),
            length: length("arc")?,
            chord: length("chord")?,
            tangent: length("tangent")?,
            external: length("external")?,
            mid_ordinate: length("mid_ordinate")?,
        };
        let solutions = solve_curve(input).map_err(|e| e.to_string())?;
        let round = |v: f64| (v * 1e6).round() / 1e6;
        let rows: Vec<Value> = solutions
            .iter()
            .map(|c| {
                let half_circle = c.delta < std::f64::consts::PI;
                json!({
                    "radius": round(c.radius),
                    "delta": conv.format_angle(Angle::from_radians(c.delta)),
                    "delta_degrees": round(c.delta.to_degrees()),
                    "arc": round(c.length),
                    "chord": round(c.chord),
                    "tangent": half_circle.then(|| round(c.tangent)),
                    "external": half_circle.then(|| round(c.external)),
                    "mid_ordinate": round(c.mid_ordinate),
                    "segment_area": round(segment_area(c)),
                })
            })
            .collect();
        Ok(ToolResult::json(&json!({
            "units": conv.linear_unit.suffix(),
            "solutions": rows,
        })))
    }
}
