//! Project settings: feature codes, layers, name and coordinate system.

use serde_json::{Value, json};
use survey_core::Conventions;
use survey_core::field_to_finish::{FeatureCode, FeatureLibrary};
use survey_core::projection::{Ellipsoid, TransverseMercator};
use survey_core::style::LineStyle;
use survey_model::{ChangeSet, Color, CoordinateSystem, Layer, LayerId};

use super::{Outcome, Server, ToolResult, flag, merge, required_str, str_arg};

impl Server {
    pub(super) fn set_feature_codes(&mut self, args: &Value) -> Outcome {
        let current = &self.doc().meta().feature_codes;
        let mut library = match str_arg(args, "preset") {
            Some("example") => FeatureLibrary::example(),
            Some(other) => return Err(format!("preset: unknown preset '{other}' (example)")),
            None if flag(args, "replace") => FeatureLibrary {
                codes: Vec::new(),
                controls: current.controls.clone(),
            },
            None => current.clone(),
        };
        if let Some(remove) = args.get("remove").and_then(Value::as_array) {
            for code in remove.iter().filter_map(Value::as_str) {
                let before = library.codes.len();
                library.codes.retain(|c| !c.code.eq_ignore_ascii_case(code));
                if library.codes.len() == before {
                    return Err(format!("remove: no code '{code}'"));
                }
            }
        }
        for (i, patch) in args
            .get("codes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let code = patch
                .get("code")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .ok_or_else(|| format!("codes[{i}]: code is required"))?;
            let at = library
                .codes
                .iter()
                .position(|c| c.code.eq_ignore_ascii_case(code));
            // An existing code keeps what is not given; a new one starts from defaults.
            let base = match at {
                Some(i) => library.codes[i].clone(),
                None => {
                    let color = Layer::palette_color(library.codes.len()).0;
                    FeatureCode::new(
                        code,
                        "",
                        &code.to_ascii_uppercase(),
                        [color[0], color[1], color[2]],
                        false,
                    )
                }
            };
            let mut value = serde_json::to_value(&base).map_err(|e| e.to_string())?;
            let mut patch = patch.clone();
            if let Some(color) = patch.get("color").and_then(Value::as_str) {
                let [r, g, b, _] = parse_color(color).map_err(|e| format!("codes[{i}].{e}"))?.0;
                patch["color"] = json!([r, g, b]);
            }
            merge(&mut value, patch);
            let feature: FeatureCode =
                serde_json::from_value(value).map_err(|e| format!("codes[{i}] ({code}): {e}"))?;
            match at {
                Some(i) => library.codes[i] = feature,
                None => library.codes.push(feature),
            }
        }
        if let Some(controls) = args.get("controls") {
            let mut value = serde_json::to_value(&library.controls).map_err(|e| e.to_string())?;
            merge(&mut value, controls.clone());
            library.controls = serde_json::from_value(value).map_err(|e| format!("controls: {e}"))?;
        }
        let n = library.codes.len();
        self.session.apply_feature_codes(library);
        Ok(ToolResult::text(format!(
            "{n} feature codes. Run 'linework' (run_commands) to string coded points into lines."
        )))
    }

    pub(super) fn set_layer(&mut self, args: &Value) -> Outcome {
        let name = required_str(args, "layer")?.trim().to_string();
        if name.is_empty() {
            return Err("layer: a name".into());
        }
        let doc = self.doc();
        let existing = doc.layer_by_name(&name).cloned();
        let mut layer = existing.clone().unwrap_or_else(|| {
            Layer::new(LayerId(0), name.clone(), Layer::palette_color(doc.layers().len()))
        });
        if let Some(new_name) = args.get("rename").and_then(Value::as_str) {
            layer.name = new_name.trim().to_string();
        }
        if let Some(color) = str_arg(args, "color") {
            layer.color = parse_color(color)?;
        }
        if let Some(style) = str_arg(args, "line_style") {
            layer.line_style = LineStyle::from_key(style).ok_or_else(|| {
                format!(
                    "line_style: one of {}",
                    keys(LineStyle::ALL.iter().map(|s| s.key()))
                )
            })?;
        }
        if let Some(w) = args.get("line_weight").and_then(Value::as_f64) {
            if !(0.1..=20.0).contains(&w) {
                return Err("line_weight: between 0.1 and 20 (screen points; 1.5 is normal)".into());
            }
            layer.line_weight = w as f32;
        }
        // Name, colour and style are one undoable step (a rename carries into
        // feature codes and surfaces); visibility and locking are display
        // state, as in the layer panel.
        let visible = args.get("visible").and_then(Value::as_bool);
        let locked = args.get("locked").and_then(Value::as_bool);
        let id = match &existing {
            Some(old) => {
                if *old != layer {
                    let cs = survey_app::session::layer_change(self.doc(), layer.clone())?;
                    self.session.execute(Box::new(cs));
                }
                old.id
            }
            None => {
                let cs = survey_app::session::layer_change(self.doc(), layer.clone())?;
                self.session.execute(Box::new(cs));
                self.doc()
                    .layer_by_name(layer.name.trim())
                    .map(|l| l.id)
                    .ok_or("The layer was not added")?
            }
        };
        let doc = &mut self.session.project.doc;
        if let Some(mut l) = doc.layer(id).cloned()
            && (visible.is_some() || locked.is_some())
        {
            l.visible = visible.unwrap_or(l.visible);
            l.locked = locked.unwrap_or(l.locked);
            doc.set_layer(id, Some(l));
            let doc = &self.session.project.doc;
            self.session
                .project
                .selection
                .retain(|e| doc.is_entity_editable(e));
        }
        if flag(args, "current") {
            // Where new objects go: a view setting, not an edit.
            self.session.project.doc.set_current_layer(id);
        }
        let doc = self.doc();
        let layer = doc.layer(id).ok_or("The layer has gone")?;
        let mut row = super::layer_row(layer);
        row["current"] = json!(doc.current_layer() == id);
        Ok(ToolResult::json(&row))
    }

    pub(super) fn set_project(&mut self, args: &Value) -> Outcome {
        let mut meta = self.doc().meta().clone();
        if let Some(name) = str_arg(args, "name") {
            meta.name = name.trim().to_string();
        }
        if let Some(d) = args.get("description").and_then(Value::as_str) {
            meta.description = d.to_string();
        }
        if let Some(preset) = str_arg(args, "conventions") {
            meta.conventions = Conventions::presets()
                .into_iter()
                .find(|c| c.name.eq_ignore_ascii_case(preset))
                .ok_or_else(|| {
                    format!(
                        "conventions: one of {}",
                        keys(Conventions::presets().iter().map(|c| c.name.as_str()))
                    )
                })?;
        }
        if let Some(cs) = args.get("coordinate_system") {
            meta.coord_system = coordinate_system(cs)?;
        }
        if &meta != self.doc().meta() {
            self.session
                .execute(Box::new(ChangeSet::new("Project settings").set_meta(meta)));
        }
        let meta = self.doc().meta();
        Ok(ToolResult::json(&json!({
            "name": meta.name,
            "description": meta.description,
            "conventions": meta.conventions.name,
            "coordinate_system": meta.coord_system.display_name(),
            "coordinate_system_detail": meta.coord_system,
        })))
    }
}

/// `#rrggbb` (or `rrggbb`).
pub(super) fn parse_color(text: &str) -> Result<Color, String> {
    let hex = text.trim().trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or(""), 16);
    match (hex.len(), channel(0), channel(2), channel(4)) {
        (6, Ok(r), Ok(g), Ok(b)) => Ok(Color::rgb(r, g, b)),
        _ => Err(format!("color: '{text}' is not #rrggbb")),
    }
}

fn keys<'a>(names: impl Iterator<Item = &'a str>) -> String {
    names.collect::<Vec<_>>().join(", ")
}

/// A coordinate system from `{type, name, zone, hemisphere, ellipsoid, epsg,
/// geoid_separation, description}`. A projected system named like
/// "MGA2020 zone 50" or "UTM zone 33N" fills in what it implies.
fn coordinate_system(v: &Value) -> Result<CoordinateSystem, String> {
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
    };
    let name = text("name").to_string();
    match text("type") {
        "unspecified" | "none" => Ok(CoordinateSystem::Unspecified),
        "local" => {
            if name.is_empty() {
                return Err("coordinate_system.name: required for a local grid".into());
            }
            Ok(CoordinateSystem::LocalGrid {
                name,
                description: text("description").to_string(),
            })
        }
        "projected" | "" if !name.is_empty() || v.get("zone").is_some() => {
            let lower = name.to_ascii_lowercase();
            let mga = lower.starts_with("mga") || lower.contains("gda");
            let (named_zone, letter) = zone_in(&lower).unzip();
            let zone = v
                .get("zone")
                .and_then(Value::as_u64)
                .map(|z| z as u32)
                .or(named_zone);
            let south = match text("hemisphere") {
                "south" | "s" => true,
                "north" | "n" => false,
                "" => mga || letter.flatten() == Some('s'),
                other => {
                    return Err(format!(
                        "coordinate_system.hemisphere: '{other}' (north or south)"
                    ));
                }
            };
            let ellipsoid = match text("ellipsoid").to_ascii_uppercase().as_str() {
                "GRS80" => Ellipsoid::GRS80,
                "WGS84" => Ellipsoid::WGS84,
                "" if mga || lower.contains("etrs") || lower.contains("nad83") => Ellipsoid::GRS80,
                "" => Ellipsoid::WGS84,
                other => return Err(format!("coordinate_system.ellipsoid: '{other}' (GRS80 or WGS84)")),
            };
            if let Some(z) = zone
                && !(1..=60).contains(&z)
            {
                return Err(format!("coordinate_system.zone: {z} is not a UTM zone (1–60)"));
            }
            let epsg = v
                .get("epsg")
                .and_then(Value::as_u64)
                .map(|e| e as u32)
                .or_else(|| {
                    let z = zone?;
                    if lower.starts_with("mga2020") || lower.starts_with("mga 2020") {
                        Some(7800 + z)
                    } else if lower.starts_with("mga94") || lower.starts_with("mga 94") {
                        Some(28300 + z)
                    } else {
                        None
                    }
                });
            let name = if name.is_empty() {
                format!("UTM zone {}{}", zone.unwrap_or(0), if south { "S" } else { "N" })
            } else {
                name
            };
            Ok(CoordinateSystem::Projected {
                name,
                epsg,
                zone,
                projection: zone.map(|z| TransverseMercator::utm(z, south, ellipsoid)),
                geoid_separation: v.get("geoid_separation").and_then(Value::as_f64).unwrap_or(0.0),
            })
        }
        "projected" | "" => Err("coordinate_system: give a name (e.g. \"MGA2020 zone 50\") or a zone".into()),
        other => Err(format!(
            "coordinate_system.type: '{other}' (projected, local or unspecified)"
        )),
    }
}

/// The zone number in a name such as "mga2020 zone 50" or "utm 33n", and the
/// hemisphere letter after it, if any.
fn zone_in(name: &str) -> Option<(u32, Option<char>)> {
    let after = name
        .split("zone")
        .nth(1)
        .or_else(|| name.split("utm").nth(1))?
        .trim_start();
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    let letter = after[digits.len()..]
        .chars()
        .next()
        .filter(|c| matches!(c, 'n' | 's'));
    Some((digits.parse().ok()?, letter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinate_systems_from_names() {
        let cs = coordinate_system(&json!({ "name": "MGA2020 Zone 50" })).expect("mga");
        match cs {
            CoordinateSystem::Projected {
                epsg,
                zone,
                projection: Some(tm),
                ..
            } => {
                assert_eq!((epsg, zone), (Some(7850), Some(50)));
                assert_eq!(tm, TransverseMercator::utm(50, true, Ellipsoid::GRS80));
            }
            other => panic!("{other:?}"),
        }
        let cs = coordinate_system(&json!({ "name": "UTM zone 33N" })).expect("utm");
        assert!(matches!(
            cs,
            CoordinateSystem::Projected { zone: Some(33), projection: Some(tm), epsg: None, .. }
                if tm == TransverseMercator::utm(33, false, Ellipsoid::WGS84)
        ));
        assert!(coordinate_system(&json!({ "type": "local" })).is_err());
        assert!(coordinate_system(&json!({ "zone": 61 })).is_err());
        assert_eq!(
            coordinate_system(&json!({ "type": "unspecified" })).expect("none"),
            CoordinateSystem::Unspecified
        );
        assert_eq!(parse_color("#7dd3fc").expect("hex"), Color::rgb(0x7d, 0xd3, 0xfc));
        assert!(parse_color("blue").is_err());
    }
}
