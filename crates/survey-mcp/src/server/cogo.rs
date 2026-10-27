//! COGO calculations that add nothing to the drawing.

use serde_json::{Value, json};
use survey_app::curves::{compound_text, reverse_text, segment_area, spiral_curve_text};
use survey_core::curve_sets::{
    CompoundInput, ReverseInput, SpiralCurveInput, solve_compound, solve_reverse, solve_spiral_curve,
};
use survey_core::solvers::{CurveInput, solve_curve};
use survey_core::{Angle, Conventions};

use super::{Outcome, Server, ToolResult, str_arg};

fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn angle_json(conv: &Conventions, radians: f64) -> Value {
    json!({ "text": conv.format_angle(Angle::from_radians(radians)), "degrees": round(radians.to_degrees()) })
}

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
        let angle = |key: &str| -> Result<Option<f64>, String> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Number(n)) => Ok(n.as_f64().map(|d| d.to_radians().abs())),
                Some(Value::String(s)) => Ok(Some(
                    conv.parse_angle(s)
                        .map_err(|e| format!("{key}: {e}"))?
                        .radians()
                        .abs(),
                )),
                Some(_) => Err(format!("{key}: degrees as a number, or an angle as text")),
            }
        };
        let units = conv.linear_unit.suffix();
        let err = |e: survey_core::solvers::SolveError| e.to_string();
        match str_arg(args, "type").unwrap_or("circular") {
            "circular" => {}
            "spiral" => {
                let c = solve_spiral_curve(SpiralCurveInput {
                    radius: length("radius")?,
                    spiral_length: length("spiral_length")?,
                    delta: angle("delta")?,
                    tangent: length("tangent")?,
                    total_length: length("total_length")?,
                })
                .map_err(err)?;
                let sp = &c.spiral;
                return Ok(ToolResult::json(&json!({
                    "units": units,
                    "radius": round(c.radius),
                    "delta": angle_json(&conv, c.delta),
                    "spiral_length": round(sp.length),
                    "a": round(sp.a),
                    "theta_s": angle_json(&conv, sp.theta),
                    "delta_c": angle_json(&conv, c.delta_c),
                    "arc": round(c.arc_length),
                    "total_length": round(c.total_length),
                    "tangent": round(c.tangent),
                    "external": round(c.external),
                    "shift_p": round(sp.p),
                    "k": round(sp.k),
                    "sc_from_ts": { "x": round(sp.x), "y": round(sp.y) },
                    "long_tangent": round(sp.long_tangent),
                    "short_tangent": round(sp.short_tangent),
                    "long_chord": round(sp.long_chord),
                    "deflection": angle_json(&conv, sp.deflection),
                    "report": spiral_curve_text(&conv, &c),
                })));
            }
            "compound" => {
                let solutions = solve_compound(CompoundInput {
                    r1: length("r1")?,
                    r2: length("r2")?,
                    delta1: angle("delta1")?,
                    delta2: angle("delta2")?,
                    delta: angle("delta")?,
                    t1: length("t1")?,
                    t2: length("t2")?,
                })
                .map_err(err)?;
                let rows: Vec<Value> = solutions
                    .iter()
                    .map(|c| {
                        json!({
                            "r1": round(c.r1), "r2": round(c.r2),
                            "delta1": angle_json(&conv, c.delta1), "delta2": angle_json(&conv, c.delta2),
                            "delta": angle_json(&conv, c.delta),
                            "t1": round(c.t1), "t2": round(c.t2),
                            "arc_tangents": [round(c.tangent1), round(c.tangent2)],
                            "common_tangent": round(c.common_tangent),
                            "arcs": [round(c.length1), round(c.length2)],
                            "long_chord": round(c.long_chord),
                            "report": compound_text(&conv, c),
                        })
                    })
                    .collect();
                return Ok(ToolResult::json(&json!({ "units": units, "solutions": rows })));
            }
            "reverse" => {
                let c = solve_reverse(ReverseInput {
                    radius: length("radius")?,
                    r1: length("r1")?,
                    r2: length("r2")?,
                    delta: angle("delta")?,
                    offset: length("offset")?,
                    along: length("along")?,
                })
                .map_err(err)?;
                return Ok(ToolResult::json(&json!({
                    "units": units,
                    "r1": round(c.r1), "r2": round(c.r2),
                    "delta": angle_json(&conv, c.delta),
                    "offset": round(c.offset), "along": round(c.along),
                    "chord": round(c.chord),
                    "arcs": [round(c.length1), round(c.length2)],
                    "report": reverse_text(&conv, &c),
                })));
            }
            other => return Err(format!("type: '{other}' (circular, spiral, compound or reverse)")),
        }
        let delta = angle("delta")?;
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
