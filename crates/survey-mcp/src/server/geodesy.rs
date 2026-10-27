//! Coordinates and set-out: grid ↔ geographic conversion with the geoid,
//! heights between the ellipsoid and the geoid, and set-out from a station.

use serde_json::{Value, json};
use survey_app::compute::projection::{self, Converted, HeightKind};
use survey_app::compute::setout;
use survey_core::{Angle, Conventions};
use survey_model::{ChangeSet, Document, Entity, EntityId, SurveyPoint};

use super::{Outcome, Server, ToolResult, flag, next_entity_id, str_arg};

fn round(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (v * k).round() / k
}

/// Points named in `args[key]`: a list of names, "selection" or "all".
fn point_targets(server: &Server, args: &Value, key: &str) -> Result<Vec<EntityId>, String> {
    let doc = server.doc();
    match args.get(key) {
        Some(Value::String(s)) if s == "selection" => {
            Ok(server.session.project.selection.ids().iter().copied().collect())
        }
        Some(Value::String(s)) if s == "all" => Ok(doc
            .entities()
            .filter(|(_, e)| matches!(e, Entity::Point(_)))
            .map(|(id, _)| id)
            .collect()),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                let name = match v {
                    Value::String(s) => s.trim().to_string(),
                    Value::Number(n) => n.to_string(),
                    _ => return Err(format!("{key}: point names")),
                };
                doc.find_point_by_name(&name)
                    .map(|(id, _)| id)
                    .ok_or_else(|| format!("{key}: no point named '{name}'"))
            })
            .collect(),
        None | Some(Value::Null) => Err(format!("{key}: point names, \"selection\" or \"all\"")),
        Some(_) => Err(format!("{key}: point names, \"selection\" or \"all\"")),
    }
}

/// A latitude or longitude: decimal degrees, or text in the project's angle format.
fn degrees(conv: &Conventions, v: Option<&Value>, key: &str) -> Result<f64, String> {
    match v {
        Some(Value::Number(n)) => n.as_f64().ok_or_else(|| format!("{key}: a number")),
        Some(Value::String(s)) => {
            let t = s.trim();
            let (negative, body) = match t.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, t),
            };
            let a = conv.parse_angle(body).map_err(|e| format!("{key}: {e}"))?;
            Ok(if negative { -a.degrees() } else { a.degrees() })
        }
        _ => Err(format!("{key}: required (degrees, south and west negative)")),
    }
}

fn converted_json(conv: &Conventions, c: &Converted) -> Value {
    let angle = |d: f64| {
        let a = conv.format_angle(Angle::from_degrees(d.abs()));
        if d < 0.0 { format!("-{a}") } else { a }
    };
    json!({
        "name": c.name,
        "latitude": round(c.latitude, 10),
        "longitude": round(c.longitude, 10),
        "latitude_dms": projection::geodetic(c.latitude),
        "longitude_dms": projection::geodetic(c.longitude),
        "easting": round(c.easting, 4),
        "northing": round(c.northing, 4),
        "height": round(c.height, 4),
        "ellipsoidal_height": round(c.ellipsoidal, 4),
        "geoid_separation": round(c.separation.n, 4),
        "geoid_source": c.separation.model.clone().unwrap_or_else(|| "project constant".into()),
        "point_scale": round(c.scale, 10),
        "convergence": angle(c.convergence),
        "convergence_degrees": round(c.convergence, 9),
        "height_factor": round(c.height_factor, 10),
        "combined_factor": round(c.combined(), 10),
    })
}

/// A grid position `[e, n, h?]` or `{easting, northing, height?}`.
fn grid_input(v: &Value) -> Result<(f64, f64, f64, Option<String>), String> {
    let num = |v: Option<&Value>| v.and_then(Value::as_f64);
    match v {
        Value::Array(a) => match (num(a.first()), num(a.get(1))) {
            (Some(e), Some(n)) => Ok((e, n, num(a.get(2)).unwrap_or(0.0), None)),
            _ => Err("grid: [easting, northing, height?]".into()),
        },
        Value::Object(_) => match (num(v.get("easting")), num(v.get("northing"))) {
            (Some(e), Some(n)) => Ok((
                e,
                n,
                num(v.get("height")).unwrap_or(0.0),
                v.get("name").and_then(Value::as_str).map(str::to_string),
            )),
            _ => Err("grid: {easting, northing, height?}".into()),
        },
        _ => Err("grid: [easting, northing, height?] items".into()),
    }
}

impl Server {
    pub(super) fn convert_coordinates(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let conv = doc.conventions();
        let kind = match str_arg(args, "height_kind") {
            None | Some("orthometric") => HeightKind::Orthometric,
            Some("ellipsoidal") => HeightKind::Ellipsoidal,
            Some(other) => return Err(format!("height_kind: '{other}' (orthometric or ellipsoidal)")),
        };
        let mut rows = Vec::new();
        let mut from_geographic = Vec::new();
        if args.get("points").is_some() {
            for id in point_targets(self, args, "points")? {
                if let Some(Entity::Point(p)) = doc.entity(id) {
                    rows.push(projection::from_grid(doc, p.position, Some(p.name.clone()))?);
                }
            }
        }
        for v in args.get("grid").and_then(Value::as_array).into_iter().flatten() {
            let (e, n, h, name) = grid_input(v)?;
            rows.push(projection::from_grid(doc, glam::DVec3::new(e, n, h), name)?);
        }
        for (i, v) in args
            .get("geographic")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let lat = degrees(
                conv,
                v.get("latitude").or(v.get("lat")),
                &format!("geographic[{i}].latitude"),
            )?;
            let lon = degrees(
                conv,
                v.get("longitude").or(v.get("lon")),
                &format!("geographic[{i}].longitude"),
            )?;
            let h = v.get("height").and_then(Value::as_f64).unwrap_or(0.0);
            let name = v.get("name").and_then(Value::as_str).map(str::to_string);
            let c = projection::from_geographic(doc, lat, lon, h, kind, name)?;
            from_geographic.push(rows.len());
            rows.push(c);
        }
        if rows.is_empty() {
            return Err(
                "Give points (names, \"selection\" or \"all\"), grid positions or geographic positions"
                    .into(),
            );
        }
        let mut created = Vec::new();
        if flag(args, "create") {
            if from_geographic.is_empty() {
                return Err("create: makes points from geographic positions; none were given".into());
            }
            let first = next_entity_id(doc);
            let mut cs = ChangeSet::new(format!(
                "{} points from latitude/longitude",
                from_geographic.len()
            ));
            let mut free = doc.point_names();
            for &i in &from_geographic {
                let r = &mut rows[i];
                let name = match &r.name {
                    Some(n) if doc.find_point_by_name(n).is_none() => n.clone(),
                    Some(n) => return Err(format!("create: a point named '{n}' already exists")),
                    None => free.next().unwrap_or_default(),
                };
                cs.push_insert_entity(Entity::Point(SurveyPoint {
                    name: name.clone(),
                    position: glam::DVec3::new(r.easting, r.northing, r.height),
                    height_known: true,
                    code: String::new(),
                    description: format!(
                        "From {} {}",
                        projection::geodetic(r.latitude),
                        projection::geodetic(r.longitude)
                    ),
                    layer: doc.current_layer(),
                }));
                r.name = Some(name.clone());
                created.push(name);
            }
            let summary = format!("Created {} points from latitude and longitude", created.len());
            let doc_rows: Vec<Value> = rows.iter().map(|r| converted_json(conv, r)).collect();
            self.session.apply(Box::new(cs), summary);
            return Ok(ToolResult::json(&json!({
                "coordinate_system": self.doc().meta().coord_system.display_name(),
                "geoid": self.session.geoid_status(),
                "positions": doc_rows,
                "created": self.new_entities(first),
            })));
        }
        let report = flag(args, "report").then(|| {
            rows.iter()
                .map(|r| projection::report(conv, r))
                .collect::<Vec<_>>()
                .join("\n\n")
        });
        Ok(ToolResult::json(&json!({
            "coordinate_system": doc.meta().coord_system.display_name(),
            "geoid": self.session.geoid_status(),
            "positions": rows.iter().map(|r| converted_json(conv, r)).collect::<Vec<_>>(),
            "report": report,
        })))
    }

    pub(super) fn convert_heights(&mut self, args: &Value) -> Outcome {
        let to_orthometric = match str_arg(args, "to") {
            Some("orthometric") => true,
            Some("ellipsoidal") => false,
            _ => return Err("to: orthometric (from GNSS ellipsoidal heights) or ellipsoidal".into()),
        };
        let ids = point_targets(self, args, "points")?;
        let summary = self.session.convert_heights(&ids, to_orthometric)?;
        Ok(ToolResult::text(summary))
    }

    pub(super) fn setout(&mut self, args: &Value) -> Outcome {
        let ids = point_targets(self, args, "points")?;
        let doc = self.doc();
        let conv = doc.conventions();
        let height = |key: &str| -> Result<f64, String> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(0.0),
                Some(Value::Number(n)) => n.as_f64().ok_or_else(|| format!("{key}: a height")),
                Some(Value::String(s)) => conv.parse_length(s).map_err(|e| format!("{key}: {e}")),
                Some(_) => Err(format!("{key}: a height")),
            }
        };
        let station = str_arg(args, "station").ok_or("station: the instrument station (a point name)")?;
        let backsight = match args.get("backsight") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };
        let set = setout::setout(doc, station, &backsight, &ids, height("hi")?, height("ht")?)?;
        Ok(ToolResult::json(&setout_json(doc, &set, flag(args, "report"))))
    }
}

fn setout_json(doc: &Document, set: &setout::Setout, report: bool) -> Value {
    let conv = doc.conventions();
    json!({
        "station": set.station,
        "backsight": set.backsight.as_ref().map(|(t, b)| json!({ "given": t, "bearing": conv.format_bearing(*b) })),
        "backsight_distance": set.backsight_distance.map(|d| round(d, 4)),
        "distances": if set.ground { "ground" } else { "grid" },
        "rows": set.rows.iter().map(|r| json!({
            "point": r.name,
            "angle": r.angle.map(|a| conv.format_angle(a)),
            "bearing": r.bearing.map(|b| conv.format_bearing(b)),
            "distance": round(r.distance, 4),
            "grid_distance": round(r.grid_distance, 4),
            "dz": r.dz.map(|d| round(d, 4)),
            "slope_distance": r.slope.map(|d| round(d, 4)),
            "zenith": r.zenith.map(|z| conv.format_angle(z)),
            "description": r.description,
        })).collect::<Vec<_>>(),
        "report": report.then(|| set.report(conv)),
    })
}
