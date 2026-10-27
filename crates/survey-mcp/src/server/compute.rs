//! Survey computations: volumes, stockpiles, network and traverse
//! adjustment, figure closure, resection, sections and subdivision. Each
//! returns its results (and report); with apply or draw it also changes the
//! drawing in one undoable step.

use std::collections::BTreeSet;

use glam::DVec3;
use serde_json::{Value, json};
use survey_app::compute::{closure, network, resection, sections, subdivide, traverse, volumes};
use survey_core::adjustment::Observation;
use survey_core::closure::{Unknown, courses_from_polyline, figure, solve};
use survey_core::resection::Weights;
use survey_core::traverse::{Adjustment, Direction, Leg, TraverseRecord};
use survey_core::volume::StockpileBase;
use survey_core::{Angle, Bearing, Conventions};
use survey_model::{ChangeSet, Entity, RecordId};

use super::drawing::{polyline_arg, position};
use super::surfaces::surface_arg;
use super::{Outcome, Server, ToolResult, flag, next_entity_id, str_arg};

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

fn round3(v: f64) -> f64 {
    (v * 1e3).round() / 1e3
}

/// Seconds of arc, to 0.1″.
fn seconds(radians: f64) -> f64 {
    (radians.to_degrees() * 36_000.0).round() / 10.0
}

/// A text or number argument as typed on the command line.
fn typed(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// An angle in the project's angle format.
fn angle(conv: &Conventions, v: Option<&Value>, key: &str) -> Result<Option<Angle>, String> {
    typed(v)
        .map(|t| conv.parse_angle(&t).map_err(|e| format!("{key}: {e}")))
        .transpose()
}

/// A bearing in the project's bearing format.
fn bearing(conv: &Conventions, v: Option<&Value>, key: &str) -> Result<Option<Bearing>, String> {
    typed(v)
        .map(|t| conv.parse_bearing(&t).map_err(|e| format!("{key}: {e}")))
        .transpose()
}

/// A length in the project's units (a number, or text with a unit).
fn length(conv: &Conventions, v: Option<&Value>, key: &str) -> Result<Option<f64>, String> {
    typed(v)
        .map(|t| conv.parse_length(&t).map_err(|e| format!("{key}: {e}")))
        .transpose()
}

fn positive(args: &Value, key: &str) -> Result<Option<f64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(x) if x > 0.0 => Ok(Some(x)),
            _ => Err(format!("{key}: a number greater than 0")),
        },
    }
}

/// A 1-based choice among `n` solutions.
fn solution(args: &Value, n: usize) -> Result<usize, String> {
    match args.get("solution").and_then(Value::as_u64) {
        None => Ok(0),
        Some(k) if k >= 1 && (k as usize) <= n => Ok(k as usize - 1),
        Some(k) => Err(format!(
            "solution: {k} — there {} {n}",
            if n == 1 { "is" } else { "are" }
        )),
    }
}

fn names(args: &Value, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| typed(Some(v)).ok_or_else(|| format!("{key}: a list of names")))
            .collect(),
        Some(_) => Err(format!("{key}: a list of names")),
    }
}

fn bearing_json(conv: &Conventions, b: Option<Bearing>) -> Value {
    b.map_or(Value::Null, |b| json!(conv.format_bearing(b)))
}

/// The sections of surfaces the line crosses.
fn crossed(series: Vec<sections::Series>) -> Vec<sections::Series> {
    series.into_iter().filter(|s| !s.runs.is_empty()).collect()
}

impl Server {
    pub(super) fn compute_volumes(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let conv = doc.conventions();
        let side = |surface: &str, level: &str| -> Result<volumes::Side, String> {
            match (args.get(surface), args.get(level)) {
                (Some(v), None) => surface_arg(doc, Some(v), surface).map(volumes::Side::Surface),
                (None, Some(v)) => Ok(volumes::Side::Level(
                    length(conv, Some(v), level)?.ok_or(format!("{level}: a height"))?,
                )),
                _ => Err(format!(
                    "Give {surface} (a surface) or {level} (a height), not both"
                )),
            }
        };
        let base = side("base", "base_level")?;
        let top = side("top", "top_level")?;
        let boundary = args
            .get("boundary")
            .map(|v| polyline_arg(doc, v, "boundary"))
            .transpose()?;
        let density = positive(args, "density")?;
        let v = volumes::volumes(doc, base, top, boundary, density)?;
        let r = &v.result;
        let mut out = json!({
            "base": base.describe(doc),
            "top": top.describe(doc),
            "fill": round3(r.fill),
            "cut": round3(r.cut),
            "net": round3(r.net()),
            "fill_area": round3(r.fill_area),
            "cut_area": round3(r.cut_area),
            "area": round3(r.area),
            "units": "m³ and m² (fill: top above base; cut: top below)",
            "report": v.report,
        });
        if let Some(d) = density {
            out["tonnes"] =
                json!({ "fill": round3(r.fill * d), "cut": round3(r.cut * d), "net": round3(r.net() * d) });
        }
        if flag(args, "create_surface") {
            let surface = volumes::cut_fill_surface(doc, base, top, v.difference);
            let name = surface.name.clone();
            self.session.create_surface(surface);
            let id = self.doc().surface_by_name(&name).map(|(id, _)| id.0);
            out["created_surface"] = json!({ "id": id, "name": name });
        }
        Ok(ToolResult::json(&out))
    }

    pub(super) fn stockpile_volume(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let conv = doc.conventions();
        let surface = surface_arg(doc, args.get("surface"), "surface")?;
        let toe = polyline_arg(doc, args.get("toe").ok_or("toe: the closed toe polyline")?, "toe")?;
        let base = match args.get("base") {
            None | Some(Value::Null) => StockpileBase::Toe,
            Some(Value::String(s)) if s == "toe" => StockpileBase::Toe,
            Some(Value::String(s)) if s == "lowest_toe" => StockpileBase::LowestToe,
            Some(v) => StockpileBase::Level(
                length(conv, Some(v), "base")?.ok_or("base: toe, lowest_toe or a level")?,
            ),
        };
        let density = positive(args, "density")?;
        let name = str_arg(args, "name").unwrap_or("");
        let (pile, report) = volumes::stockpile_volume(doc, surface, toe, base, density, name)?;
        let r = &pile.result;
        let mut out = json!({
            "volume": round3(r.net()),
            "above_base": round3(r.fill),
            "below_base": round3(r.cut),
            "plan_area": round3(r.area),
            "toe_heights": [round3(pile.base_min), round3(pile.base_max)],
            "units": "m³ and m²",
            "report": report,
        });
        if let Some(d) = density {
            out["tonnes"] = json!(round3(r.net() * d));
        }
        Ok(ToolResult::json(&out))
    }

    pub(super) fn adjust_network(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let conv = doc.conventions();
        let mut sources = BTreeSet::new();
        for name in names(args, "sources")? {
            sources.insert(
                network::source_key(doc, &name)
                    .ok_or_else(|| format!("sources: no saved traverse or raw data file named {name}"))?,
            );
        }
        if sources.is_empty() {
            sources = network::all_sources(doc);
        }
        if sources.is_empty() {
            return Err(
                "Nothing to adjust: save a traverse (adjust_traverse with save) or import raw data".into(),
            );
        }
        let mut free = network::free_by_default(doc, &sources);
        free.extend(names(args, "free")?);
        for name in names(args, "fixed")? {
            free.remove(&name);
        }
        let mut options = doc.meta().network_options;
        if let Some(s) = positive(args, "direction_sigma")? {
            options.direction_sigma = Angle::from_degrees(s / 3600.0);
        }
        if let Some(mm) = positive(args, "distance_sigma_mm")? {
            options.distance_sigma = mm / 1000.0;
        }
        if let Some(ppm) = args.get("distance_ppm").and_then(Value::as_f64) {
            options.distance_ppm = ppm.max(0.0);
        }
        if let Some(mm) = args.get("centring_mm").and_then(Value::as_f64) {
            options.centring = mm.max(0.0) / 1000.0;
        }
        if let Some(mm) = positive(args, "height_sigma_mm")? {
            options.height_sigma = mm / 1000.0;
        }
        if let Some(h) = args.get("heights").and_then(Value::as_bool) {
            options.heights = h;
        }
        let setups = network::setups(doc, &sources);
        match args.get("scale_factor") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s == "project" => {
                // The combined factor at the centre of the known stations.
                let known: Vec<DVec3> = setups
                    .iter()
                    .flat_map(|s| std::iter::once(&s.station).chain(s.observations.iter().map(|o| &o.target)))
                    .filter_map(|n| doc.find_point_by_name(n).map(|(_, p)| p.position))
                    .collect();
                let centre = known.iter().sum::<DVec3>() / known.len().max(1) as f64;
                options.scale_factor = doc
                    .combined_factor_at(centre)
                    .ok_or("scale_factor 'project' needs a projected coordinate system (set_project)")?;
            }
            Some(v) => {
                options.scale_factor = v
                    .as_f64()
                    .filter(|k| (0.9..=1.1).contains(k))
                    .ok_or("scale_factor: a factor near 1, or 'project'")?;
            }
        }
        let (net, r) = network::run(doc, &setups, &free, &options)?;
        let stations: Vec<Value> = net
            .stations
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let p = r.positions[i];
                let sd = r.sd[i];
                json!({
                    "name": s.name,
                    "e": round(p.x),
                    "n": round(p.y),
                    "z": (r.sd[i].z > 0.0 || s.fixed_height).then(|| round(p.z)),
                    "held": if s.fixed_plan { "fixed" } else if net.new_stations.contains(&i) { "new" } else { "adjusted" },
                    "sd_mm": [round3(sd.x * 1000.0), round3(sd.y * 1000.0), round3(sd.z * 1000.0)],
                    "ellipse_95_mm": r.ellipses[i].map(|e| json!({
                        "semi_major": round3(e.semi_major * 1000.0),
                        "semi_minor": round3(e.semi_minor * 1000.0),
                        "bearing": conv.format_angle(e.bearing),
                    })),
                    "moves_mm": (!s.fixed_plan).then(|| round3((p - s.position).truncate().length() * 1000.0)),
                })
            })
            .collect();
        let mut residuals: Vec<(f64, Value)> = net
            .observations
            .iter()
            .zip(&r.residuals)
            .zip(&net.labels)
            .map(|((o, v), label)| {
                let (value, unit) = match o {
                    Observation::Direction { .. } | Observation::Bearing { .. } => (seconds(v.value), "″"),
                    Observation::Distance { .. } | Observation::HeightDiff { .. } => (round3(v.value * 1000.0), "mm"),
                };
                let w = v.standardised.unwrap_or(0.0);
                (
                    w.abs(),
                    json!({ "observation": label, "residual": value, "unit": unit, "standardised": v.standardised.map(round3) }),
                )
            })
            .collect();
        residuals.sort_by(|a, b| b.0.total_cmp(&a.0));
        let suspect = residuals.iter().filter(|(w, _)| *w > 3.0).count();
        let verdict = match r.chi_square {
            Some(c) if c.pass => "passed: residuals agree with the a priori precision",
            Some(c) if c.value > c.upper => {
                "failed: residuals larger than the a priori precision allows (σ₀ > 1); look for blunders \
                 (standardised residual over 3), else loosen the standard deviations"
            }
            Some(_) => {
                "failed: the a priori precision is pessimistic (σ₀ < 1); tighten the standard deviations"
            }
            None => "no redundancy: nothing to test",
        };
        let mut out = json!({
            "sources": sources.iter().map(|k| &k[2..]).collect::<Vec<_>>(),
            "sigma0": round3(r.sigma0),
            "dof": r.dof,
            "observations": net.observations.len(),
            "iterations": r.iterations,
            "chi_square": r.chi_square.map(|c| json!({ "value": round3(c.value), "lower": round3(c.lower), "upper": round3(c.upper), "pass": c.pass })),
            "verdict": verdict,
            "suspect_observations": suspect,
            "largest_residuals": residuals.into_iter().take(10).map(|(_, v)| v).collect::<Vec<_>>(),
            "stations": stations,
            "free": free,
            "options": {
                "direction_sigma_seconds": round3(options.direction_sigma.degrees() * 3600.0),
                "distance_sigma_mm": round3(options.distance_sigma * 1000.0),
                "distance_ppm": options.distance_ppm,
                "centring_mm": round3(options.centring * 1000.0),
                "heights": options.heights,
                "height_sigma_mm": round3(options.height_sigma * 1000.0),
                "scale_factor": options.scale_factor,
            },
        });
        if flag(args, "report") {
            out["report"] = json!(survey_core::network::report(&net, &r, &options, conv));
        }
        if flag(args, "apply") {
            let (cs, summary) = network::apply(doc, &net, &r, &options);
            self.session.apply(Box::new(cs), summary.clone());
            out["applied"] = json!(summary);
        } else {
            out["next"] = json!(format!(
                "Nothing changed. Call again with apply: true to update {} points.",
                network::updates(&net, &r)
            ));
        }
        Ok(ToolResult::json(&out))
    }

    /// A traverse record from a saved one (`traverse`) or the arguments.
    fn traverse_record(&self, args: &Value) -> Result<TraverseRecord, String> {
        let doc = self.doc();
        let conv = doc.conventions();
        let mut rec = match args.get("traverse") {
            Some(Value::Number(n)) => {
                let id = RecordId(n.as_i64().unwrap_or(-1));
                doc.record::<TraverseRecord>(id)
                    .cloned()
                    .ok_or_else(|| format!("traverse: no saved traverse with id {n}"))?
            }
            Some(Value::String(name)) => doc
                .traverses()
                .iter()
                .find(|t| t.name == *name)
                .map(|t| t.record.clone())
                .ok_or_else(|| format!("traverse: no saved traverse named {name}"))?,
            Some(_) => return Err("traverse: a saved traverse's name or id".into()),
            None => {
                let start = str_arg(args, "start").ok_or("start: the point the traverse starts from")?;
                let legs = args
                    .get("legs")
                    .and_then(Value::as_array)
                    .filter(|l| !l.is_empty())
                    .ok_or("legs: one or more legs")?;
                let mut out = Vec::new();
                for (i, l) in legs.iter().enumerate() {
                    let n = i + 1;
                    let key = |k: &str| format!("legs[{n}].{k}");
                    let direction = match (
                        angle(conv, l.get("angle"), &key("angle"))?,
                        angle(conv, l.get("angle_left"), &key("angle_left"))?,
                        l.get("deflection"),
                        bearing(conv, l.get("bearing"), &key("bearing"))?,
                    ) {
                        (Some(a), None, None, None) => Direction::Angle(a),
                        (None, Some(a), None, None) => Direction::AngleLeft(a),
                        (None, None, Some(d), None) => Direction::Deflection(
                            traverse::parse_deflection(conv, &typed(Some(d)).unwrap_or_default())
                                .map_err(|e| format!("{}: {e}", key("deflection")))?,
                        ),
                        (None, None, None, Some(b)) => Direction::Bearing(b),
                        _ => {
                            return Err(format!(
                                "legs[{n}]: give one of angle, angle_left, deflection (e.g. '12.3000 R') or bearing"
                            ));
                        }
                    };
                    let distance = length(conv, l.get("distance"), &key("distance"))?
                        .ok_or_else(|| format!("{}: required", key("distance")))?;
                    let to = typed(l.get("to")).unwrap_or_else(|| format!("T{n}"));
                    let mut leg = Leg::new(to, direction, distance);
                    leg.zenith = angle(conv, l.get("zenith"), &key("zenith"))?;
                    leg.dz = length(conv, l.get("dz"), &key("dz"))?;
                    leg.instrument_height = length(conv, l.get("hi"), &key("hi"))?.unwrap_or(0.0);
                    leg.target_height = length(conv, l.get("ht"), &key("ht"))?.unwrap_or(0.0);
                    out.push(leg);
                }
                TraverseRecord {
                    name: str_arg(args, "save").unwrap_or("").to_string(),
                    start: start.to_string(),
                    backsight: traverse::parse_reference(
                        doc,
                        conv,
                        &typed(args.get("backsight")).unwrap_or_default(),
                    )
                    .map_err(|e| format!("backsight: {e}"))?,
                    legs: out,
                    close_on: str_arg(args, "close_on").map(str::to_string),
                    closing_reference: traverse::parse_reference(
                        doc,
                        conv,
                        &typed(args.get("closing_reference")).unwrap_or_default(),
                    )
                    .map_err(|e| format!("closing_reference: {e}"))?,
                    closing_angle: angle(conv, args.get("closing_angle"), "closing_angle")?,
                    scale_factor: args.get("scale_factor").and_then(Value::as_f64).unwrap_or(1.0),
                    ..TraverseRecord::default()
                }
            }
        };
        if let Some(a) = str_arg(args, "adjustment") {
            rec.adjustment = match a {
                "bowditch" => Adjustment::Bowditch,
                "transit" => Adjustment::Transit,
                "none" => Adjustment::None,
                other => return Err(format!("adjustment: '{other}' is not bowditch, transit or none")),
            };
        }
        if let Some(b) = args.get("adjust_angles").and_then(Value::as_bool) {
            rec.adjust_angles = b;
        }
        Ok(rec)
    }

    pub(super) fn adjust_traverse(&mut self, args: &Value) -> Outcome {
        let mut rec = self.traverse_record(args)?;
        let doc = self.doc();
        let conv = doc.conventions();
        let tolerances = doc.meta().tolerances;
        let (t, r) = traverse::reduce(doc, &rec)?;
        let angular = r.angular_misclosure.map(|a| {
            let check = tolerances.check_angular(a, r.angles);
            json!({
                "misclosure": conv.format_angle(a),
                "seconds": seconds(a.radians()),
                "angles": r.angles,
                "adjusted": r.angles_adjusted,
                "allowed_seconds": check.map(|c| seconds(c.allowed)),
                "pass": check.map(|c| c.pass),
            })
        });
        let linear = r.linear_misclosure.map(|m| {
            let check = tolerances.check_linear(m.length(), r.length);
            json!({
                "de": round(m.x),
                "dn": round(m.y),
                "length": round(m.length()),
                "bearing": bearing_json(conv, Bearing::from_delta(m.x, m.y)),
                "precision": r.precision().map(|n| format!("1:{n:.0}")),
                "allowed": check.map(|c| round(c.allowed)),
                "pass": check.map(|c| c.pass),
            })
        });
        let stations: Vec<Value> = r
            .stations
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "bearing": conv.format_bearing(s.bearing),
                    "distance": round(s.distance),
                    "e": round(s.position.x),
                    "n": round(s.position.y),
                    "z": s.height_known.then(|| round(s.position.z)),
                    "correction_mm": [round3((s.position.x - s.unadjusted.x) * 1000.0), round3((s.position.y - s.unadjusted.y) * 1000.0)],
                })
            })
            .collect();
        let mut out = json!({
            "start": t.start_name,
            "adjustment": r.adjustment.label(),
            "length": round(r.length),
            "closed_on": rec.close_on,
            "angular": angular,
            "linear": linear,
            "vertical_misclosure": r.vertical_misclosure.map(round),
            "stations": stations,
            "report": survey_core::traverse::report(&t, &r, &tolerances, conv),
        });
        if rec.close_on.is_none() {
            out["note"] = json!("Open traverse (no close_on): no misclosure, nothing adjusted");
        }
        let first_new = next_entity_id(doc);
        let mut summaries = Vec::new();
        let mut cs = if flag(args, "apply") {
            let polyline = args.get("polyline").and_then(Value::as_bool).unwrap_or(true);
            let (cs, summary) = traverse::create_points(doc, &t, &r, polyline);
            summaries.push(summary);
            cs
        } else {
            ChangeSet::new(String::new())
        };
        if let Some(name) = str_arg(args, "save") {
            rec.name = name.to_string();
            cs = match doc.traverses().iter().find(|k| k.name == name) {
                Some(k) => cs.update_record(k.id, rec.clone()),
                None => cs.insert_record(rec.clone()),
            };
            summaries.push(format!("saved traverse {name}"));
        }
        if !cs.is_empty() {
            let summary = summaries.join("; ");
            let cs = cs.with_label(format!("Traverse from {}: {summary}", t.start_name));
            self.session.apply(Box::new(cs), summary.clone());
            out["done"] = json!(summary);
            out["added"] = json!(self.new_entities(first_new));
        } else {
            out["next"] = json!(
                "Nothing changed. apply: true creates the stations (and a polyline); save: '<name>' keeps the traverse for adjust_network."
            );
        }
        Ok(ToolResult::json(&out))
    }

    pub(super) fn figure_closure(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let conv = doc.conventions();
        let (courses, title, start) = match (args.get("courses"), args.get("polyline")) {
            (Some(Value::Array(list)), None) if !list.is_empty() => {
                let mut inputs = Vec::new();
                for (i, c) in list.iter().enumerate() {
                    let key = |k: &str| format!("courses[{}].{k}", i + 1);
                    let right = match str_arg(c, "turn") {
                        None | Some("left") => false,
                        Some("right") => true,
                        Some(other) => {
                            return Err(format!("{}: '{other}' is not left or right", key("turn")));
                        }
                    };
                    inputs.push(closure::CourseInput {
                        bearing: bearing(conv, c.get("bearing"), &key("bearing"))?,
                        distance: length(conv, c.get("distance"), &key("distance"))?,
                        radius: length(conv, c.get("radius"), &key("radius"))?,
                        arc_length: length(conv, c.get("arc_length"), &key("arc_length"))?,
                        right,
                    });
                }
                let start = args
                    .get("start")
                    .map(|v| position(doc, Some(v), "start"))
                    .transpose()?;
                (
                    closure::courses(&inputs)?,
                    str_arg(args, "name").unwrap_or("Figure").to_string(),
                    start,
                )
            }
            (None, Some(v)) => {
                let id = polyline_arg(doc, v, "polyline")?;
                let Some(Entity::Polyline(l)) = doc.entity(id) else {
                    return Err("polyline: not a polyline".into());
                };
                let name = if l.name.is_empty() {
                    "Polyline".to_string()
                } else {
                    l.name.clone()
                };
                (
                    courses_from_polyline(&l.vertices, &l.bulges, l.closed),
                    name,
                    l.vertices.first().copied(),
                )
            }
            _ => {
                return Err(
                    "Give courses (a list) or polyline (to check an existing figure), not both".into(),
                );
            }
        };
        let solutions = solve(&courses).map_err(|e| e.to_string())?;
        let pick = solution(args, solutions.len())?;
        let sol = &solutions[pick];
        let fig = figure(&sol.courses);
        let tolerances = doc.meta().tolerances;
        let solved: Vec<Value> = sol
            .solved
            .iter()
            .map(|u| {
                let c = sol.courses[u.course()];
                match u {
                    Unknown::Bearing(i) => {
                        json!({ "course": i + 1, "bearing": bearing_json(conv, c.bearing) })
                    }
                    Unknown::Distance(i) => json!({
                        "course": i + 1,
                        if c.arc.is_some() { "chord" } else { "distance" }: c.distance.map(round),
                        "arc_length": c.arc.and(c.length()).map(round),
                    }),
                }
            })
            .collect();
        let m = fig.misclosure;
        let check = tolerances.check_linear(m.length(), fig.perimeter);
        let mut out = json!({
            "solutions": solutions.len(),
            "solution": pick + 1,
            "solved": solved,
            "reversed_courses": sol.reversed.iter().map(|i| i + 1).collect::<Vec<_>>(),
            "misclosure": round(m.length()),
            "misclosure_bearing": if fig.precision().is_some() { bearing_json(conv, Bearing::from_delta(m.x, m.y)) } else { Value::Null },
            "precision": fig.precision().map(|n| format!("1:{n:.0}")),
            "allowed": check.map(|c| round(c.allowed)),
            "pass": check.map(|c| c.pass),
            "perimeter": round(fig.perimeter),
            "area": round(fig.area),
            "report": survey_core::closure::report(&title, sol, &fig, &tolerances, conv),
        });
        if solutions.len() > 1 {
            out["note"] = json!("Two solutions: pass solution: 2 for the other");
        }
        if flag(args, "draw") {
            let start = start.ok_or("start: the first corner (a point name or [e, n]) to draw from")?;
            let closed = args.get("closed").and_then(Value::as_bool).unwrap_or(true);
            let first_new = next_entity_id(doc);
            let (cs, summary) = closure::draw(doc, start, &fig, &title, closed);
            self.session.apply(Box::new(cs), summary.clone());
            out["drawn"] = json!(summary);
            out["added"] = json!(self.new_entities(first_new));
        }
        Ok(ToolResult::json(&out))
    }

    pub(super) fn resection(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let conv = doc.conventions();
        let list = args
            .get("observations")
            .and_then(Value::as_array)
            .filter(|l| !l.is_empty())
            .ok_or("observations: the known points sighted, each with direction and/or distance")?;
        let mut observed = Vec::new();
        for (i, o) in list.iter().enumerate() {
            let key = |k: &str| format!("observations[{}].{k}", i + 1);
            observed.push(resection::Observed {
                point: typed(o.get("point")).ok_or_else(|| format!("{}: required", key("point")))?,
                direction: angle(conv, o.get("direction"), &key("direction"))?,
                distance: length(conv, o.get("distance"), &key("distance"))?,
            });
        }
        let sightings = resection::sightings(doc, &observed)?;
        let weights = Weights {
            direction: Angle::from_degrees(positive(args, "direction_sigma")?.unwrap_or(5.0) / 3600.0),
            distance: positive(args, "distance_sigma_mm")?.unwrap_or(5.0) / 1000.0,
        };
        let solutions = resection::solve(&sightings, weights)?;
        let pick = solution(args, solutions.len())?;
        let station = str_arg(args, "station")
            .map(str::to_string)
            .unwrap_or_else(|| doc.next_point_name());
        let rows: Vec<Value> = solutions
            .iter()
            .enumerate()
            .map(|(k, r)| {
                json!({
                    "solution": k + 1,
                    "e": round(r.position.x),
                    "n": round(r.position.y),
                    "orientation": r.orientation.map(|o| conv.format_angle(o)),
                    "redundancy": r.redundancy,
                    "sd_mm": r.sigma.map(|s| [round3(s.x * 1000.0), round3(s.y * 1000.0)]),
                    "sigma0": r.sigma0.map(round3),
                    "residuals": sightings.iter().zip(&r.residuals).filter(|_| r.redundancy > 0).map(|((name, _), (d, s))| json!({
                        "point": name,
                        "direction_seconds": d.map(|a| seconds(a.radians())),
                        "distance_mm": s.map(|x| round3(x * 1000.0)),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let r = &solutions[pick];
        let mut out = json!({
            "station": station,
            "solutions": rows,
            "chosen": pick + 1,
            "report": resection::report(conv, &station, &sightings, r),
        });
        if solutions.len() > 1 {
            out["note"] = json!(
                "Two solutions fit (too few observations to tell them apart): pick with solution, ideally from a rough position or an extra observation"
            );
        } else if r.redundancy == 0 {
            out["note"] =
                json!("No redundancy: the position is not checked. Add another direction or distance.");
        }
        if flag(args, "apply") {
            let height = length(conv, args.get("height"), "height")?;
            let first_new = next_entity_id(doc);
            let (cs, summary) = resection::station_command(doc, &station, r, height)?;
            self.session.apply(Box::new(cs), summary.clone());
            out["created"] = json!(summary);
            out["added"] = json!(self.new_entities(first_new));
        }
        Ok(ToolResult::json(&out))
    }

    pub(super) fn sections(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let conv = doc.conventions();
        let id = polyline_arg(
            doc,
            args.get("line").ok_or("line: the polyline to section along")?,
            "line",
        )?;
        let Some(Entity::Polyline(l)) = doc.entity(id) else {
            return Err("line: not a polyline".into());
        };
        let wanted = names(args, "surfaces")?;
        for name in &wanted {
            if doc.surface_by_name(name).is_none() {
                return Err(format!("surfaces: no surface called {name}"));
            }
        }
        let indexes: Vec<_> = sections::indexes(doc)
            .into_iter()
            .filter(|(sid, _)| {
                wanted.is_empty() || doc.surface(*sid).is_some_and(|s| wanted.contains(&s.name))
            })
            .collect();
        if indexes.is_empty() {
            return Err("There are no surfaces to section".into());
        }
        let sample = positive(args, "sample")?;
        let total = sections::length(l);
        // Heights at every `sample` along the line, else at each triangle edge crossed.
        let table = |series: &[sections::Series], from: f64, to: f64| -> Value {
            match sample {
                Some(step) => {
                    let mut xs = Vec::new();
                    let mut x = from;
                    while x <= to + 1e-9 && xs.len() < 5000 {
                        xs.push(x);
                        x += step;
                    }
                    if xs.last().is_some_and(|x| to - x > 1e-6) {
                        xs.push(to);
                    }
                    json!(xs.iter().map(|x| {
                        let mut row = json!({ "at": round(*x) });
                        for s in series {
                            row[&s.name] = json!(s.runs.iter().find_map(|r| sections::height_at(r, *x)).map(round));
                        }
                        row
                    }).collect::<Vec<_>>())
                }
                None => json!(series.iter().map(|s| json!({
                    "surface": s.name,
                    "runs": s.runs.iter().map(|r| r.iter().map(|(d, z)| [round(*d), round(*z)]).collect::<Vec<_>>()).collect::<Vec<_>>(),
                })).collect::<Vec<_>>()),
            }
        };
        let kind = str_arg(args, "kind").unwrap_or("long");
        let name = if l.name.is_empty() {
            "polyline".to_string()
        } else {
            l.name.clone()
        };
        let mut out = match kind {
            "long" => {
                let series = crossed(sections::sections(doc, &indexes, &sections::long_line(l), 0.0));
                let mut out = json!({
                    "line": name,
                    "length": round(total),
                    "kind": "long section (distance is chainage)",
                    "section": table(&series, 0.0, total),
                });
                if flag(args, "report") {
                    out["report"] = json!(sections::report(
                        conv,
                        &format!("Long section {name}"),
                        &series,
                        false
                    ));
                }
                out
            }
            "cross" => {
                let left = length(conv, args.get("left"), "left")?.unwrap_or(20.0);
                let right = length(conv, args.get("right"), "right")?.unwrap_or(20.0);
                let chainages: Vec<f64> = match args.get("chainages") {
                    Some(Value::Array(list)) => list
                        .iter()
                        .map(|v| v.as_f64().ok_or("chainages: numbers"))
                        .collect::<Result<_, _>>()?,
                    _ => sections::stations(total, positive(args, "interval")?.unwrap_or(20.0)),
                };
                if chainages.len() > 500 {
                    return Err(format!(
                        "{} cross sections: use a larger interval",
                        chainages.len()
                    ));
                }
                let mut report = String::new();
                let mut rows = Vec::new();
                for ch in chainages {
                    let line = sections::cross_line(l, ch, left, right);
                    if line.is_empty() {
                        rows.push(json!({ "chainage": round(ch), "error": "off the line" }));
                        continue;
                    }
                    let series = crossed(sections::sections(doc, &indexes, &line, -left));
                    if flag(args, "report") {
                        let title = format!("Cross section Ch {}", conv.format_chainage(ch));
                        report.push_str(&sections::report(conv, &title, &series, true));
                        report.push('\n');
                    }
                    rows.push(json!({ "chainage": round(ch), "section": table(&series, -left, right) }));
                }
                let mut out = json!({
                    "line": name,
                    "length": round(total),
                    "kind": "cross sections (distance is offset, negative left)",
                    "left": left,
                    "right": right,
                    "sections": rows,
                });
                if flag(args, "report") {
                    out["report"] = json!(report);
                }
                out
            }
            other => return Err(format!("kind: '{other}' is not long or cross")),
        };
        if sample.is_none() {
            out["note"] =
                json!("Heights at each triangle edge crossed; pass sample (metres) for a regular table");
        }
        Ok(ToolResult::json(&out))
    }

    pub(super) fn subdivide(&mut self, args: &Value) -> Outcome {
        let doc = self.doc();
        let conv = doc.conventions();
        let id = polyline_arg(
            doc,
            args.get("parcel").ok_or("parcel: the closed polyline")?,
            "parcel",
        )?;
        let Some(Entity::Polyline(l)) = doc.entity(id) else {
            return Err("parcel: not a polyline".into());
        };
        let parcel = subdivide::Parcel::load(id, l).ok_or("parcel: the polyline must be closed")?;
        let unit = conv.linear_unit.metres_per_unit();
        let area =
            positive(args, "area")?.ok_or("area: the area to cut off (square project units)")? * unit * unit;
        let edges: Vec<Value> = (0..parcel.edges.len())
            .map(|i| json!(parcel.edge_label(conv, i)))
            .collect();
        let method = match str_arg(args, "method").unwrap_or("parallel") {
            "parallel" => {
                let n = args
                    .get("edge")
                    .and_then(Value::as_u64)
                    .ok_or("edge: the straight edge (its segment number from 1) the cut runs parallel to")?;
                let i = parcel
                    .edges
                    .iter()
                    .position(|(_, seg)| *seg as u64 + 1 == n)
                    .ok_or_else(|| format!("edge: {n} is not a straight edge of the parcel (see edges)"))?;
                subdivide::Method::Parallel(i)
            }
            "swing" => {
                let hinge = match args.get("hinge") {
                    Some(Value::Number(n)) => {
                        let k = n
                            .as_u64()
                            .filter(|k| *k >= 1)
                            .ok_or("hinge: a corner number from 1")?;
                        parcel
                            .corner(k as usize - 1)
                            .ok_or_else(|| format!("hinge: the parcel has no corner {k}"))?
                    }
                    Some(v) => parcel.local(position(doc, Some(v), "hinge")?),
                    None => return Err("hinge: a corner number, a point name or [e, n]".into()),
                };
                subdivide::Method::Swing(hinge)
            }
            other => return Err(format!("method: '{other}' is not parallel or swing")),
        };
        let cuts = subdivide::cuts(&parcel, method, area)?;
        let pick = solution(args, cuts.len())?;
        let to_world = |p: glam::DVec2| {
            let w = parcel.origin + p.extend(0.0);
            [round(w.x), round(w.y)]
        };
        let rows: Vec<Value> = cuts
            .iter()
            .enumerate()
            .map(|(k, (b, cut))| {
                let (brg, len) = subdivide::cut_line(cut);
                json!({
                    "solution": k + 1,
                    "bearing": bearing_json(conv, b.or(brg)),
                    "cut_length": round(len),
                    "area_cut_off": round(cut.part_area),
                    "remainder": round(cut.remainder_area),
                    "cut_line": cut.segments.iter().map(|(a, b)| [to_world(*a), to_world(*b)]).collect::<Vec<_>>(),
                })
            })
            .collect();
        let (b, cut) = &cuts[pick];
        let mut out = json!({
            "parcel": parcel.name,
            "parcel_area": round(parcel.area()),
            "edges": edges,
            "solutions": rows,
            "chosen": pick + 1,
            "report": subdivide::report(conv, &parcel, b.or(subdivide::cut_line(cut).0), cut),
        });
        match str_arg(args, "draw").unwrap_or("none") {
            "none" => {
                out["next"] = json!(
                    "Nothing drawn: draw 'cut' adds the cut line, 'parcels' also the two parcels (then mkparcel to make them lots)."
                );
            }
            what @ ("cut" | "parcels") => {
                let first_new = next_entity_id(doc);
                let (cs, summary) = subdivide::draw(doc, &parcel, cut, what == "parcels");
                self.session.apply(Box::new(cs), summary.clone());
                out["drawn"] = json!(summary);
                out["added"] = json!(self.new_entities(first_new));
            }
            other => return Err(format!("draw: '{other}' is not none, cut or parcels")),
        }
        Ok(ToolResult::json(&out))
    }
}
