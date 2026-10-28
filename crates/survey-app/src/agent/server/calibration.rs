//! Site calibration: fit GNSS-measured points to local control, preview the
//! residuals, and apply it.

use crate::calibration::{SiteFit, Target, apply, compute, format_rotation, outside_control};
use serde_json::{Value, json};
use survey_core::calibration::{PairSetup, SiteCalibration, VerticalMethod};
use survey_model::{ChangeSet, Entity};

use super::{Outcome, Server, ToolResult, str_arg};

impl Server {
    pub(super) fn site_calibration(&mut self, args: &Value) -> Outcome {
        let mut setup = self.doc().meta().calibration.clone();
        if let Some(pairs) = args.get("pairs").and_then(Value::as_array) {
            setup.pairs = pairs
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let text = |key: &str| {
                        p.get(key)
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .ok_or_else(|| format!("pairs[{i}].{key} is required"))
                    };
                    let flag = |key: &str| p.get(key).and_then(Value::as_bool).unwrap_or(true);
                    Ok(PairSetup {
                        measured: text("measured")?,
                        control: text("control")?,
                        horizontal: flag("horizontal"),
                        vertical: flag("vertical"),
                    })
                })
                .collect::<Result<_, String>>()?;
        }
        let opts = &mut setup.options;
        if let Some(fixed) = args.get("fixed_scale").and_then(Value::as_bool) {
            opts.fixed_scale = fixed;
        }
        if let Some(v) = str_arg(args, "vertical") {
            opts.vertical = VerticalMethod::from_key(v)
                .ok_or_else(|| format!("vertical: '{v}' is not none, constant or plane"))?;
        }
        if let Some(t) = args.get("tolerance_h").and_then(Value::as_f64) {
            opts.tolerance_h = t;
        }
        if let Some(t) = args.get("tolerance_v").and_then(Value::as_f64) {
            opts.tolerance_v = t;
        }
        let site = compute(self.doc(), &setup)?;
        let mut out = fit_json(self, &site, &setup);

        let target = match str_arg(args, "apply").unwrap_or("none") {
            "none" => None,
            "all_except_control" => Some(Target::AllExceptControl),
            "selection" => Some(Target::Objects(
                self.session.project.selection.ids().iter().copied().collect(),
            )),
            other => {
                return Err(format!(
                    "apply: '{other}' is not none, all_except_control or selection"
                ));
            }
        };
        if let Some(target) = target {
            let (cs, summary) = apply(self.doc(), &setup, &target)?;
            self.session.apply(Box::new(cs), summary.clone());
            out["applied"] = json!(summary);
        } else if args.get("save").and_then(Value::as_bool).unwrap_or(false) {
            let mut meta = self.doc().meta().clone();
            meta.calibration = SiteCalibration {
                applied: meta.calibration.applied,
                ..setup
            };
            self.session.execute(Box::new(
                ChangeSet::new("Save site calibration pairs").set_meta(meta),
            ));
            out["saved"] = json!(true);
        } else {
            out["preview"] = json!(
                "Nothing changed. Check the residuals and warnings, then call again with apply: \
                 all_except_control (or selection)."
            );
        }
        Ok(ToolResult::json(&out))
    }
}

fn fit_json(server: &Server, site: &SiteFit, setup: &SiteCalibration) -> Value {
    let doc = server.doc();
    let conv = doc.conventions();
    let opts = &setup.options;
    let fit = &site.fit;
    let round = |v: f64| (v * 1e4).round() / 1e4;
    let pairs: Vec<Value> = site
        .pairs
        .iter()
        .zip(&fit.residuals)
        .enumerate()
        .map(|(i, (p, r))| {
            let over = r.horizontal.is_some_and(|v| v.length() > opts.tolerance_h)
                || r.vertical.is_some_and(|v| v.abs() > opts.tolerance_v);
            // Left-out pairs get their misfit too (how far they are from the fit).
            let (h, v) = site.check(i);
            json!({
                "measured": p.setup.measured,
                "control": p.setup.control,
                "horizontal": p.pair.horizontal,
                "vertical": p.pair.vertical,
                "has_heights": p.has_heights,
                "residual_e": h.map(|v| round(v.x)),
                "residual_n": h.map(|v| round(v.y)),
                "residual_h": v.map(round),
                "over_limit": over,
            })
        })
        .collect();
    let c = &fit.calibration;
    let horizontal = c.horizontal.map(|h| {
        json!({
            "pairs": fit.horizontal_points,
            "redundancy": fit.horizontal_redundancy,
            "scale": h.scale(),
            "scale_ppm": round((h.scale() - 1.0) * 1e6),
            "rotation": format_rotation(conv, h.rotation()),
            "rotation_degrees_clockwise": h.rotation().to_degrees(),
            "shift_e": round(h.shift().x),
            "shift_n": round(h.shift().y),
            "origin": [h.origin.x, h.origin.y],
            "fixed_scale": opts.fixed_scale,
        })
    });
    let vertical = c.vertical.map(|v| {
        json!({
            "points": fit.vertical_points,
            "redundancy": fit.vertical_redundancy,
            "method": fit.vertical_method.map(|m| m.key()),
            "shift": round(v.constant),
            "slope_north_ppm": round(v.slope_north * 1e6),
            "slope_east_ppm": round(v.slope_east * 1e6),
        })
    });
    // Measured points beyond the control (where the fit is extrapolated).
    let points: Vec<_> = doc
        .entities()
        .filter_map(|(_, e)| match e {
            Entity::Point(p) => Some(c.apply_plan(p.position).truncate()),
            _ => None,
        })
        .collect();
    json!({
        "pairs": pairs,
        "horizontal": horizontal,
        "vertical": vertical,
        "max_residual_h": fit.max_horizontal().map(round),
        "max_residual_v": fit.max_vertical().map(round),
        "limits": { "h": opts.tolerance_h, "v": opts.tolerance_v },
        "points_outside_control": outside_control(site, &points),
        "warnings": fit.warnings,
    })
}
