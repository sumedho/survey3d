//! Georeferenced images (orthophotos): add, inspect, set, place by control
//! points with residuals, and world files.

use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use survey_app::imagery::ImageStatus;
use survey_core::georef::{FitKind, fit};
use survey_model::{ImageRecord, Placement, RecordId};

use super::drawing::position;
use super::{Outcome, Server, ToolResult, str_arg};

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

impl Server {
    /// The image named or numbered by `image`, else the only one.
    fn image_id(&self, args: &Value) -> Result<RecordId, String> {
        let list = self.doc().images();
        let found = match args.get("image") {
            Some(Value::Number(n)) => list.iter().find(|r| Some(r.id.0) == n.as_i64()),
            Some(Value::String(name)) => list.iter().find(|r| r.name == *name),
            None | Some(Value::Null) if list.len() == 1 => list.first(),
            None | Some(Value::Null) => {
                return Err(format!(
                    "image: its name or id ({} images; add one with action add)",
                    list.len()
                ));
            }
            Some(_) => return Err("image: its name or id".into()),
        };
        found
            .map(|r| r.id)
            .ok_or_else(|| "image: no such image".to_string())
    }

    fn image_json(&self, id: RecordId) -> Value {
        let Ok(rec) = self.session.image(id) else {
            return Value::Null;
        };
        let conv = self.doc().conventions();
        let fitted = fit(&rec.control_points, rec.fit).ok();
        let status = match self.session.imagery.status(&rec.path) {
            ImageStatus::Ready => json!("ready"),
            ImageStatus::Building(f) => json!(format!("building {:.0}%", f * 100.0)),
            ImageStatus::Failed(e) => json!(format!("could not read: {e}")),
            ImageStatus::Missing => json!("file not found (relink it)"),
        };
        // From east, with its direction.
        let r = rec.transform.rotation();
        let text = conv.format_angle(survey_core::Angle::from_radians(r.abs()));
        let rotation = if r.abs() < 1e-12 {
            text
        } else {
            format!("{text} {}", if r > 0.0 { "anticlockwise" } else { "clockwise" })
        };
        json!({
            "id": id.0,
            "name": rec.name,
            "path": rec.path.display().to_string(),
            "size": [rec.width, rec.height],
            "pixel_size": round(rec.transform.pixel_size()),
            "rotation": rotation,
            "placement": match rec.placement {
                Placement::File => "file",
                Placement::ControlPoints => "control_points",
                Placement::Rough => "not_placed",
            },
            "epsg": rec.epsg,
            "corners": rec.corners().iter().map(|c| [round(c.x), round(c.y)]).collect::<Vec<_>>(),
            "visible": rec.visible,
            "opacity": rec.opacity,
            "elevation": rec.elevation,
            "fit": rec.fit,
            "rms": fitted.as_ref().map(|f| round(f.rms)),
            "control_points": rec.control_points.iter().enumerate().map(|(i, p)| json!({
                "number": i + 1,
                "pixel": [round(p.pixel[0]), round(p.pixel[1])],
                "world": [round(p.world[0]), round(p.world[1])],
                "enabled": p.enabled,
                "residual": fitted.as_ref().and_then(|f| f.residuals.get(i)).map(|r| round(r.length())),
            })).collect::<Vec<_>>(),
            "status": status,
        })
    }

    pub(super) fn image(&mut self, args: &Value) -> Outcome {
        match str_arg(args, "action").unwrap_or("show") {
            "add" => {
                let path = PathBuf::from(
                    str_arg(args, "path").ok_or("path: the image file (GeoTIFF, JPEG or PNG)")?,
                );
                let (id, summary) = self.session.import_image(&path)?;
                // Agents render straight after: build the tiles now.
                let messages = self.session.imagery.wait(&self.session.project.doc);
                let mut out = self.image_json(id);
                out["summary"] = json!(summary);
                out["messages"] = json!(messages);
                Ok(ToolResult::json(&out))
            }
            "show" => {
                let id = self.image_id(args)?;
                Ok(ToolResult::json(&self.image_json(id)))
            }
            "set" => {
                let id = self.image_id(args)?;
                let mut rec = self.session.image(id)?.clone();
                if let Some(v) = args.get("visible").and_then(Value::as_bool) {
                    rec.visible = v;
                }
                if let Some(v) = args.get("opacity").and_then(Value::as_f64) {
                    rec.opacity = v.clamp(0.0, 1.0) as f32;
                }
                if let Some(v) = args.get("elevation").and_then(Value::as_f64) {
                    rec.elevation = v;
                }
                if let Some(v) = str_arg(args, "name") {
                    rec.name = v.trim().to_string();
                }
                self.session.update_record(id, rec);
                Ok(ToolResult::json(&self.image_json(id)))
            }
            "control_points" => {
                let id = self.image_id(args)?;
                let mut rec: ImageRecord = self.session.image(id)?.clone();
                match str_arg(args, "fit") {
                    None => {}
                    Some("helmert") => rec.fit = FitKind::Helmert,
                    Some("affine") => rec.fit = FitKind::Affine,
                    Some(other) => return Err(format!("fit: '{other}' (helmert or affine)")),
                }
                // Point numbers all refer to the list as it was: switch first, then remove.
                for (key, on) in [("enable", true), ("disable", false)] {
                    for n in args
                        .get(key)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_u64)
                    {
                        let p = rec
                            .control_points
                            .get_mut((n as usize).wrapping_sub(1))
                            .ok_or_else(|| format!("{key}: there is no control point {n}"))?;
                        p.enabled = on;
                    }
                }
                // Removals highest number first, so the others keep theirs.
                let mut remove: Vec<usize> = args
                    .get("remove")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_u64)
                    .map(|n| n as usize)
                    .collect();
                remove.sort_unstable();
                for n in remove.into_iter().rev() {
                    if n == 0 || n > rec.control_points.len() {
                        return Err(format!("remove: there is no control point {n}"));
                    }
                    rec.control_points.remove(n - 1);
                }
                let doc = self.doc();
                for (i, p) in args
                    .get("add")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let pixel = match (p.get("pixel"), p.get("at")) {
                        (Some(px), None) => {
                            let a = px
                                .as_array()
                                .filter(|a| a.len() == 2)
                                .ok_or(format!("add[{i}].pixel: [column, row]"))?;
                            DVec2::new(
                                a[0].as_f64().ok_or(format!("add[{i}].pixel: numbers"))?,
                                a[1].as_f64().ok_or(format!("add[{i}].pixel: numbers"))?,
                            )
                        }
                        // A position on the image as it is drawn now.
                        (None, Some(at)) => {
                            let w = position(doc, Some(at), &format!("add[{i}].at"))?.truncate();
                            rec.transform
                                .inverse(w)
                                .ok_or("The image's placement cannot be inverted")?
                        }
                        _ => {
                            return Err(format!(
                                "add[{i}]: pixel [column, row] or at [e, n] (where it is on the image now)"
                            ));
                        }
                    };
                    // Within half a pixel of the edge counts as on it (corners given rounded).
                    let (w, h) = (f64::from(rec.width), f64::from(rec.height));
                    if pixel.x < -0.5 || pixel.y < -0.5 || pixel.x > w + 0.5 || pixel.y > h + 0.5 {
                        return Err(format!(
                            "add[{i}]: pixel {:.1}, {:.1} is off the image",
                            pixel.x, pixel.y
                        ));
                    }
                    let pixel = pixel.clamp(DVec2::ZERO, DVec2::new(w, h));
                    let world = position(doc, p.get("world"), &format!("add[{i}].world"))?.truncate();
                    rec.control_points.push(survey_core::georef::ControlPoint {
                        pixel: pixel.to_array(),
                        world: world.to_array(),
                        enabled: true,
                    });
                }
                let summary = self.session.refit_image(id, rec, "Control points".into())?;
                let mut out = self.image_json(id);
                out["summary"] = json!(summary);
                Ok(ToolResult::json(&out))
            }
            "save_world_file" => {
                let id = self.image_id(args)?;
                let path = self.session.save_world_file(id)?;
                Ok(ToolResult::text(format!("Wrote {}", path.display())))
            }
            "relink" => {
                let id = self.image_id(args)?;
                let path = PathBuf::from(str_arg(args, "path").ok_or("path: where the image file is now")?);
                let summary = self.session.relink_image(id, &path)?;
                self.session.imagery.wait(&self.session.project.doc);
                Ok(ToolResult::text(summary))
            }
            other => Err(format!(
                "action: '{other}' (add, show, set, control_points, save_world_file or relink)"
            )),
        }
    }
}
