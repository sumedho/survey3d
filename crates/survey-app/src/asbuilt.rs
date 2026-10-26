//! As-built comparison: surveyed shots (stake-out checks or as-built pickup)
//! against the design, as design points (matched by name, else the nearest),
//! design lines (offset and chainage) or a design surface (cut and fill),
//! with each difference held against plan and height tolerances.

use std::fmt::Write as _;

use glam::{DVec2, DVec3};
use survey_core::Conventions;
use survey_core::cogo::station_offset;
use survey_core::tin::TinIndex;
use survey_model::{Document, Entity, EntityId, SurfaceId, SurveyPoint, TextAlign, TextNote};

use crate::import_command::ImportEntities;

/// What the shots are compared with.
#[derive(Clone, Debug, PartialEq)]
pub enum Against {
    /// Design points with the same name, after removing `strip` from the start
    /// or end of the shot's name (stake-out shots are often named `101S` or
    /// `S101`); failing that, the nearest design point within `radius`.
    Points {
        strip: String,
        radius: Option<f64>,
    },
    /// The nearest of these polylines.
    Lines(Vec<EntityId>),
    Surface(SurfaceId),
}

impl Against {
    pub fn key(&self) -> &'static str {
        match self {
            Against::Points { .. } => "points",
            Against::Lines(_) => "lines",
            Against::Surface(_) => "surface",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub position: DVec3,
    /// The design point, line or surface it was compared with.
    pub reference: String,
    /// Shot − design, in plan (points only).
    pub delta: Option<DVec2>,
    /// Chainage along and offset from a design line (offset positive right).
    pub chainage: Option<f64>,
    pub offset: Option<f64>,
    /// Shot − design height (positive: the shot is high, i.e. fill to cut).
    pub dz: Option<f64>,
    pub over: bool,
}

impl Row {
    /// The plan difference: the miss from a design point or the offset from a line.
    pub fn plan(&self) -> Option<f64> {
        self.delta.map(DVec2::length).or(self.offset.map(f64::abs))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub against: String,
    pub rows: Vec<Row>,
    /// Shots with nothing to compare with, and why.
    pub unmatched: Vec<(String, String)>,
    pub tolerance_h: f64,
    pub tolerance_v: f64,
}

impl Comparison {
    pub fn over(&self) -> usize {
        self.rows.iter().filter(|r| r.over).count()
    }

    pub fn max_plan(&self) -> Option<f64> {
        self.rows.iter().filter_map(Row::plan).reduce(f64::max)
    }

    pub fn max_dz(&self) -> Option<f64> {
        self.rows
            .iter()
            .filter_map(|r| r.dz.map(f64::abs))
            .reduce(f64::max)
    }

    /// Mean and RMS of the height differences.
    pub fn dz_stats(&self) -> Option<(f64, f64)> {
        let dz: Vec<f64> = self.rows.iter().filter_map(|r| r.dz).collect();
        if dz.is_empty() {
            return None;
        }
        let n = dz.len() as f64;
        Some((
            dz.iter().sum::<f64>() / n,
            (dz.iter().map(|v| v * v).sum::<f64>() / n).sqrt(),
        ))
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "As-built against {}: {} shots compared, {} over tolerance",
            self.against,
            self.rows.len(),
            self.over()
        );
        if let Some(h) = self.max_plan() {
            let _ = write!(s, "; worst plan {h:.3} m");
        }
        if let Some(v) = self.max_dz() {
            let _ = write!(s, ", worst height {v:.3} m");
        }
        if !self.unmatched.is_empty() {
            let _ = write!(s, "; {} not compared", self.unmatched.len());
        }
        s
    }
}

fn signed(conv: &Conventions, v: f64) -> String {
    format!(
        "{}{}",
        if v < 0.0 { "-" } else { "+" },
        conv.format_length(v.abs())
    )
}

/// Compares the points among `shots` with the design.
pub fn compare(
    doc: &Document,
    shots: &[EntityId],
    against: &Against,
    tolerance_h: f64,
    tolerance_v: f64,
) -> Result<Comparison, String> {
    let points: Vec<(EntityId, &SurveyPoint)> = shots
        .iter()
        .filter_map(|id| match doc.entity(*id) {
            Some(Entity::Point(p)) => Some((*id, p)),
            _ => None,
        })
        .collect();
    if points.is_empty() {
        return Err("Select the surveyed points to compare".into());
    }
    let mut rows = Vec::new();
    let mut unmatched = Vec::new();
    let over = |plan: Option<f64>, dz: Option<f64>| {
        plan.is_some_and(|h| h > tolerance_h + 1e-9) || dz.is_some_and(|v| v.abs() > tolerance_v + 1e-9)
    };
    let label;
    match against {
        Against::Points { strip, radius } => {
            label = "design points".to_string();
            let design: Vec<&SurveyPoint> = doc
                .entities()
                .filter_map(|(id, e)| match e {
                    Entity::Point(p) if !shots.contains(&id) => Some(p),
                    _ => None,
                })
                .collect();
            let strip = strip.trim();
            for (_, p) in &points {
                let by_name = |n: &str| design.iter().find(|d| d.name == n).copied();
                let stripped = (!strip.is_empty())
                    .then(|| {
                        p.name
                            .strip_suffix(strip)
                            .or_else(|| p.name.strip_prefix(strip))
                            .filter(|n| !n.is_empty())
                    })
                    .flatten();
                let found = by_name(&p.name)
                    .or_else(|| stripped.and_then(by_name))
                    .or_else(|| {
                        let r = (*radius)?;
                        design
                            .iter()
                            .map(|d| (d.position.truncate().distance(p.position.truncate()), *d))
                            .filter(|(dist, _)| *dist <= r)
                            .min_by(|a, b| a.0.total_cmp(&b.0))
                            .map(|(_, d)| d)
                    });
                let Some(d) = found else {
                    unmatched.push((p.name.clone(), "no design point of that name or nearby".into()));
                    continue;
                };
                let delta = (p.position - d.position).truncate();
                let dz = (p.height_known && d.height_known).then_some(p.position.z - d.position.z);
                rows.push(Row {
                    name: p.name.clone(),
                    position: p.position,
                    reference: d.name.clone(),
                    delta: Some(delta),
                    chainage: None,
                    offset: None,
                    dz,
                    over: over(Some(delta.length()), dz),
                });
            }
        }
        Against::Lines(ids) => {
            let lines: Vec<_> = ids
                .iter()
                .filter_map(|id| match doc.entity(*id) {
                    Some(Entity::Polyline(l)) if l.vertices.len() >= 2 => Some(l),
                    _ => None,
                })
                .collect();
            if lines.is_empty() {
                return Err("Choose the design lines (polylines) to compare with".into());
            }
            label = if lines.len() == 1 {
                format!("line {}", line_name(lines[0]))
            } else {
                format!("{} design lines", lines.len())
            };
            for (_, p) in &points {
                let best = lines
                    .iter()
                    .filter_map(|l| {
                        station_offset(&l.vertices, &l.bulges, l.closed, p.position.truncate())
                            .map(|so| (l, so))
                    })
                    .min_by(|a, b| a.1.offset.abs().total_cmp(&b.1.offset.abs()));
                let Some((l, so)) = best else {
                    unmatched.push((p.name.clone(), "no design line".into()));
                    continue;
                };
                let dz = (p.height_known && l.heights_known).then_some(p.position.z - so.foot.z);
                rows.push(Row {
                    name: p.name.clone(),
                    position: p.position,
                    reference: line_name(l),
                    delta: None,
                    chainage: Some(so.chainage),
                    offset: Some(so.offset),
                    dz,
                    over: over(Some(so.offset.abs()), dz),
                });
            }
        }
        Against::Surface(id) => {
            let s = doc.surface(*id).ok_or("That surface no longer exists")?;
            label = format!("surface {}", s.name);
            let index = TinIndex::new(&s.tin);
            for (_, p) in &points {
                if !p.height_known {
                    unmatched.push((p.name.clone(), "no height".into()));
                    continue;
                }
                let Some(z) = index.height_at(&s.tin, p.position.truncate()) else {
                    unmatched.push((p.name.clone(), "off the surface".into()));
                    continue;
                };
                let dz = p.position.z - z;
                rows.push(Row {
                    name: p.name.clone(),
                    position: p.position,
                    reference: s.name.clone(),
                    delta: None,
                    chainage: None,
                    offset: None,
                    dz: Some(dz),
                    over: over(None, Some(dz)),
                });
            }
        }
    }
    Ok(Comparison {
        against: label,
        rows,
        unmatched,
        tolerance_h,
        tolerance_v,
    })
}

fn line_name(l: &survey_model::Polyline) -> String {
    if !l.name.is_empty() {
        l.name.clone()
    } else if !l.code.is_empty() {
        l.code.clone()
    } else {
        "(unnamed)".into()
    }
}

/// The comparison as a plain-text report.
pub fn report(doc: &Document, c: &Comparison) -> String {
    let conv = doc.conventions();
    let mut out = String::new();
    let _ = writeln!(out, "AS-BUILT COMPARISON — against {}", c.against);
    let _ = writeln!(
        out,
        "Tolerance: plan {}, height {}",
        conv.format_length_with_unit(c.tolerance_h),
        conv.format_length_with_unit(c.tolerance_v)
    );
    let _ = writeln!(out);
    let dash = || "-".to_string();
    let opt = |v: Option<f64>| v.map(|v| signed(conv, v)).unwrap_or_else(dash);
    match c.rows.first().map(|r| (r.delta.is_some(), r.offset.is_some())) {
        Some((true, _)) => {
            let _ = writeln!(
                out,
                "{:<10} {:<10} {:>10} {:>10} {:>10} {:>10}",
                "Shot", "Design", "dE", "dN", "Plan", "dZ"
            );
            for r in &c.rows {
                let d = r.delta.unwrap_or_default();
                let _ = writeln!(
                    out,
                    "{:<10} {:<10} {:>10} {:>10} {:>10} {:>10}{}",
                    r.name,
                    r.reference,
                    signed(conv, d.x),
                    signed(conv, d.y),
                    conv.format_length(d.length()),
                    opt(r.dz),
                    if r.over { "  OVER" } else { "" }
                );
            }
        }
        Some((_, true)) => {
            let _ = writeln!(
                out,
                "{:<10} {:<12} {:>12} {:>10} {:>10}",
                "Shot", "Line", "Chainage", "Offset", "dZ"
            );
            for r in &c.rows {
                let _ = writeln!(
                    out,
                    "{:<10} {:<12} {:>12} {:>10} {:>10}{}",
                    r.name,
                    r.reference,
                    r.chainage.map(|v| conv.format_chainage(v)).unwrap_or_else(dash),
                    opt(r.offset),
                    opt(r.dz),
                    if r.over { "  OVER" } else { "" }
                );
            }
        }
        _ => {
            let _ = writeln!(out, "{:<10} {:>14} {:>14} {:>10}", "Shot", "E", "N", "dZ");
            for r in &c.rows {
                let _ = writeln!(
                    out,
                    "{:<10} {:>14.3} {:>14.3} {:>10}  {}{}",
                    r.name,
                    r.position.x,
                    r.position.y,
                    opt(r.dz),
                    r.dz.map_or("", |v| if v > 0.0 { "fill → cut" } else { "cut → fill" }),
                    if r.over { "  OVER" } else { "" }
                );
            }
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "{} compared, {} over tolerance", c.rows.len(), c.over());
    if let Some(h) = c.max_plan() {
        let _ = writeln!(out, "Worst plan difference {}", conv.format_length_with_unit(h));
    }
    if let Some((mean, rms)) = c.dz_stats() {
        let _ = writeln!(
            out,
            "Height: worst {}, mean {}, RMS {}",
            conv.format_length_with_unit(c.max_dz().unwrap_or(0.0)),
            signed(conv, mean),
            conv.format_length(rms)
        );
    }
    if !c.unmatched.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "NOT COMPARED");
        for (n, why) in &c.unmatched {
            let _ = writeln!(out, "  {n}: {why}");
        }
    }
    out
}

/// Labels beside each compared shot (the height difference, and the plan
/// miss or offset), on the layer `layer`, in one step.
pub fn labels(doc: &Document, c: &Comparison, layer: &str) -> ImportEntities {
    let conv = doc.conventions();
    let mut cmd = ImportEntities::new(format!("Label as-built differences ({})", c.rows.len()));
    let l = cmd.layer_named(doc, Some(layer));
    for r in &c.rows {
        let mut parts = Vec::new();
        if let Some(d) = r.delta {
            parts.push(format!("Δ{}", conv.format_length(d.length())));
        }
        if let Some(o) = r.offset {
            parts.push(format!("O/S {}", signed(conv, o)));
        }
        if let Some(dz) = r.dz {
            parts.push(format!("dZ {}", signed(conv, dz)));
        }
        if parts.is_empty() {
            continue;
        }
        cmd.add(
            l,
            Entity::Text(TextNote {
                text: parts.join(" "),
                position: r.position,
                height: 1.8,
                rotation: 0.0,
                align: TextAlign::Left,
                layer: doc.current_layer(),
            }),
        );
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_model::{ChangeSet, History, Polyline};

    fn point(name: &str, e: f64, n: f64, z: Option<f64>) -> Entity {
        Entity::Point(SurveyPoint {
            name: name.into(),
            position: DVec3::new(e, n, z.unwrap_or(0.0)),
            height_known: z.is_some(),
            code: String::new(),
            description: String::new(),
            layer: survey_model::LayerId(0),
        })
    }

    #[test]
    fn points_lines_and_labels() {
        let mut doc = Document::new();
        let mut history = History::new("t");
        let cs = ChangeSet::new("setup")
            .insert_entity(point("101", 100.0, 100.0, Some(10.0)))
            .insert_entity(point("102", 120.0, 100.0, Some(10.0)))
            .insert_entity(point("101S", 100.010, 99.995, Some(10.004)))
            .insert_entity(point("X1", 120.03, 100.0, Some(9.95)))
            .insert_entity(point("lost", 500.0, 500.0, None))
            .insert_entity(Entity::Polyline(Polyline {
                name: "KERB".into(),
                vertices: vec![DVec3::new(0.0, 0.0, 5.0), DVec3::new(100.0, 0.0, 6.0)],
                bulges: Vec::new(),
                closed: false,
                code: String::new(),
                layer: survey_model::LayerId(0),
                heights_known: true,
            }))
            .insert_entity(point("K1", 50.0, -0.02, Some(5.52)));
        history.execute(Box::new(cs), &mut doc);
        let id = |n: &str| doc.find_point_by_name(n).expect(n).0;
        let shots = [id("101S"), id("X1"), id("lost")];
        let c = compare(
            &doc,
            &shots,
            &Against::Points {
                strip: "S".into(),
                radius: Some(0.1),
            },
            0.02,
            0.02,
        )
        .expect("points");
        assert_eq!(c.rows.len(), 2);
        assert_eq!(c.rows[0].reference, "101");
        assert!((c.rows[0].delta.expect("d") - DVec2::new(0.01, -0.005)).length() < 1e-9);
        // X1 matched by distance: 30 mm out in plan and 50 mm low.
        assert_eq!(c.rows[1].reference, "102");
        assert!(c.rows[1].over && !c.rows[0].over);
        assert_eq!(c.unmatched.len(), 1);
        assert!(report(&doc, &c).contains("OVER"));

        let kerb = doc
            .entities()
            .find(|(_, e)| matches!(e, Entity::Polyline(_)))
            .expect("kerb")
            .0;
        let c = compare(&doc, &[id("K1")], &Against::Lines(vec![kerb]), 0.05, 0.05).expect("lines");
        let r = &c.rows[0];
        assert!((r.chainage.expect("ch") - 50.0).abs() < 1e-9);
        // Below the line heading east is to the right: positive offset.
        assert!((r.offset.expect("os") - 0.02).abs() < 1e-9);
        assert!((r.dz.expect("dz") - 0.02).abs() < 1e-9);
        assert!(!r.over);

        let cmd = labels(&doc, &c, "ASBUILT");
        assert_eq!(cmd.len(), 1);
        history.execute(Box::new(cmd), &mut doc);
        assert!(doc.layer_by_name("ASBUILT").is_some());
        assert!(
            doc.entities()
                .any(|(_, e)| matches!(e, Entity::Text(t) if t.text.contains("O/S +0.020")))
        );
    }
}
