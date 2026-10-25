//! Site calibration in a project: pairs of points named in the drawing (the
//! GNSS-measured point and the control it should match), the fit, and applying
//! it to the measured data as one undoable step. The calibration window and
//! the MCP server both work through here.

use std::fmt::Write as _;

use glam::{DVec2, DVec3};
use survey_core::Conventions;
use survey_core::calibration::{Fit, Pair, PairSetup, SiteCalibration, VerticalMethod, fit};
use survey_model::{ChangeSet, Document, Entity, EntityId};

/// A pair found in the drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPair {
    pub setup: PairSetup,
    pub measured_id: EntityId,
    /// The control point, unless its coordinates were typed.
    pub control_id: Option<EntityId>,
    pub pair: Pair,
    /// Both heights are known, so the pair can be used vertically.
    pub has_heights: bool,
}

/// A calibration's fit, with the pairs it used.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteFit {
    pub pairs: Vec<ResolvedPair>,
    pub fit: Fit,
}

impl SiteFit {
    /// Pair `i`'s misfit against the calibration, whether or not the pair was
    /// used (a left-out pair's residual shows how far it is from the rest).
    pub fn check(&self, i: usize) -> (Option<DVec2>, Option<f64>) {
        let (Some(p), Some(r)) = (self.pairs.get(i), self.fit.residuals.get(i)) else {
            return (None, None);
        };
        let c = &self.fit.calibration;
        let h = r.horizontal.or_else(|| {
            c.horizontal
                .map(|h| p.pair.control.truncate() - h.apply(p.pair.measured.truncate()))
        });
        let v = r.vertical.or_else(|| {
            c.vertical
                .filter(|_| p.has_heights)
                .map(|v| p.pair.control.z - (p.pair.measured.z + v.shift_at(p.pair.measured.truncate())))
        });
        (h, v)
    }
}

/// What to transform.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Every editable object except the control points.
    AllExceptControl,
    /// These objects (control points are still left alone).
    Objects(Vec<EntityId>),
}

/// Rows with both names blank are skipped.
fn is_blank(s: &PairSetup) -> bool {
    s.measured.trim().is_empty() && s.control.trim().is_empty()
}

/// Finds each pair's points (control may also be typed as `E,N[,Z]`).
pub fn resolve(doc: &Document, setups: &[PairSetup]) -> Result<Vec<ResolvedPair>, String> {
    let conv = doc.conventions();
    let mut out = Vec::new();
    for (i, s) in setups.iter().enumerate() {
        if is_blank(s) {
            continue;
        }
        let row = i + 1;
        let name = s.measured.trim();
        let (measured_id, measured) = doc
            .find_point_by_name(name)
            .ok_or_else(|| format!("Pair {row}: no measured point named '{name}'"))?;
        let control_text = s.control.trim();
        if control_text.is_empty() {
            return Err(format!("Pair {row}: enter the control point (a name or E,N[,Z])"));
        }
        let (control_id, control, control_height) = match doc.find_point_by_name(control_text) {
            Some((id, p)) => (Some(id), p.position, p.height_known),
            None => match conv.parse_coord(control_text) {
                Ok(c) => (
                    None,
                    DVec3::new(c.easting, c.northing, c.height.unwrap_or(0.0)),
                    c.height.is_some(),
                ),
                Err(_) => {
                    return Err(format!(
                        "Pair {row}: no control point named '{control_text}' (or type its coordinates)"
                    ));
                }
            },
        };
        if control_id == Some(measured_id) {
            return Err(format!(
                "Pair {row}: the measured and control points are the same point"
            ));
        }
        let has_heights = measured.height_known && control_height;
        out.push(ResolvedPair {
            setup: s.clone(),
            measured_id,
            control_id,
            pair: Pair {
                measured: measured.position,
                control,
                horizontal: s.horizontal,
                vertical: s.vertical && has_heights,
            },
            has_heights,
        });
    }
    if out.is_empty() {
        return Err("Add pairs: each a GNSS-measured point and the control point it should match.".into());
    }
    Ok(out)
}

/// Fits the calibration set up in `cal`.
pub fn compute(doc: &Document, cal: &SiteCalibration) -> Result<SiteFit, String> {
    let pairs = resolve(doc, &cal.pairs)?;
    let only: Vec<Pair> = pairs.iter().map(|p| p.pair).collect();
    let fit = fit(&only, &cal.options).map_err(|e| e.to_string())?;
    Ok(SiteFit { pairs, fit })
}

/// The objects `target` names that will move: editable, and not control points.
pub fn targets(doc: &Document, site: &SiteFit, target: &Target) -> Vec<EntityId> {
    let control: Vec<EntityId> = site.pairs.iter().filter_map(|p| p.control_id).collect();
    let keep = |id: &EntityId| doc.is_entity_editable(*id) && !control.contains(id);
    match target {
        Target::AllExceptControl => doc.entities().map(|(id, _)| id).filter(keep).collect(),
        Target::Objects(ids) => ids
            .iter()
            .copied()
            .filter(|id| doc.contains(*id))
            .filter(keep)
            .collect(),
    }
}

/// Fits the calibration and transforms `target` with it, saving the
/// calibration in the project: one undoable change, and a summary for the log.
pub fn apply(doc: &Document, cal: &SiteCalibration, target: &Target) -> Result<(ChangeSet, String), String> {
    let site = compute(doc, cal)?;
    let ids = targets(doc, &site, target);
    if ids.is_empty() {
        return Err("Nothing to calibrate: select the measured objects first.".into());
    }
    let c = site.fit.calibration;
    let turn = -c.rotation().to_degrees();
    let mut cs = ChangeSet::new(format!("Site calibration ({} objects)", ids.len()));
    let mut moved_points = Vec::new();
    for id in &ids {
        let Some(mut e) = doc.entity(*id).cloned() else {
            continue;
        };
        // Heights only move where they are known.
        let heights = match &e {
            Entity::Point(p) => p.height_known,
            Entity::Polyline(l) => l.heights_known,
            Entity::Text(_) => false,
        };
        if heights {
            e.map_positions(|p| c.apply(p));
        } else {
            e.map_positions(|p| c.apply_plan(p));
        }
        e.rotate_text(turn);
        if let Entity::Point(p) = &e {
            moved_points.push(p.position.truncate());
        }
        cs.push_update_entity(*id, e);
    }
    let mut meta = doc.meta().clone();
    meta.calibration = SiteCalibration {
        applied: Some(c),
        ..cal.clone()
    };
    cs = cs.set_meta(meta);

    let conv = doc.conventions();
    let mut summary = format!(
        "Site calibration applied to {} objects: {}",
        ids.len(),
        parameters(conv, &site)
    );
    let outside = outside_control(&site, &moved_points);
    if outside > 0 {
        let _ = write!(
            summary,
            ". {outside} point{} lie outside the control area, where the calibration is extrapolated",
            if outside == 1 { "" } else { "s" }
        );
    }
    if doc.surfaces().next().is_some() {
        summary.push_str(". Surfaces are not moved: rebuild any made from these points");
    }
    Ok((cs, summary))
}

/// One line of the fitted parameters.
pub fn parameters(conv: &Conventions, site: &SiteFit) -> String {
    let c = &site.fit.calibration;
    let mut parts = Vec::new();
    if let Some(h) = c.horizontal {
        parts.push(format!(
            "scale {:.8} ({:+.1} ppm), rotation {}",
            h.scale(),
            (h.scale() - 1.0) * 1e6,
            format_rotation(conv, c.rotation())
        ));
    }
    if let Some(v) = c.vertical {
        parts.push(format!(
            "height shift {}{}",
            conv.format_length_with_unit(v.constant),
            if site.fit.vertical_method == Some(VerticalMethod::InclinedPlane) {
                format!(
                    ", tilt N {:+.1} ppm E {:+.1} ppm",
                    v.slope_north * 1e6,
                    v.slope_east * 1e6
                )
            } else {
                String::new()
            }
        ));
    }
    if let Some(m) = site.fit.max_horizontal() {
        parts.push(format!("max H residual {}", conv.format_length_with_unit(m)));
    }
    if let Some(m) = site.fit.max_vertical() {
        parts.push(format!("max V residual {}", conv.format_length_with_unit(m)));
    }
    parts.join(", ")
}

/// A rotation (clockwise radians) as a signed angle.
pub fn format_rotation(conv: &Conventions, radians: f64) -> String {
    let text = conv.format_angle(survey_core::Angle::from_radians(radians.abs()));
    // No sign on an angle that rounds to zero.
    let zero = conv.format_angle(survey_core::Angle::from_radians(0.0));
    if radians < 0.0 && text != zero {
        format!("-{text}")
    } else {
        text
    }
}

/// How many of `points` (calibrated) lie outside the hull of the horizontal
/// control. Calibrations should enclose the work.
pub fn outside_control(site: &SiteFit, points: &[DVec2]) -> usize {
    let control: Vec<DVec2> = site
        .pairs
        .iter()
        .filter(|p| p.pair.horizontal)
        .map(|p| p.pair.control.truncate())
        .collect();
    let hull = convex_hull(&control);
    if hull.len() < 3 {
        return 0;
    }
    // A little slack so points on the edge (the control's own pairs) count as inside.
    points.iter().filter(|p| !inside_convex(&hull, **p, 0.01)).count()
}

/// Convex hull, anticlockwise (Andrew's monotone chain).
fn convex_hull(points: &[DVec2]) -> Vec<DVec2> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<DVec2> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &DVec2>> = if pass == 0 {
            Box::new(pts.iter())
        } else {
            Box::new(pts.iter().rev())
        };
        for p in iter {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0 {
                hull.pop();
            }
            hull.push(*p);
        }
        hull.pop();
    }
    hull
}

fn inside_convex(hull: &[DVec2], p: DVec2, slack: f64) -> bool {
    let n = hull.len();
    (0..n).all(|i| {
        let (a, b) = (hull[i], hull[(i + 1) % n]);
        let edge = b - a;
        // Signed distance to the left of the edge.
        edge.perp_dot(p - a) / edge.length().max(1e-12) >= -slack
    })
}

/// A text report: the pairs and residuals, the parameters and the checks.
pub fn report(conv: &Conventions, site: &SiteFit, cal: &SiteCalibration) -> String {
    let mut s = String::new();
    let (ea, nb) = conv.horizontal_labels();
    let _ = writeln!(s, "SITE CALIBRATION");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "{:<12} {:<14} {:>2} {:>2} {:>10} {:>10} {:>10}",
        "Measured",
        "Control",
        "H",
        "V",
        format!("d{ea}"),
        format!("d{nb}"),
        "dH"
    );
    let flag = |b: bool| if b { "y" } else { "-" };
    for (p, r) in site.pairs.iter().zip(&site.fit.residuals) {
        let (de, dn) = r
            .horizontal
            .map(|v| {
                let (a, b) = conv.horizontal_values(v.extend(0.0));
                (format!("{a:+.3}"), format!("{b:+.3}"))
            })
            .unwrap_or_default();
        let _ = writeln!(
            s,
            "{:<12} {:<14} {:>2} {:>2} {:>10} {:>10} {:>10}",
            p.setup.measured.trim(),
            p.setup.control.trim(),
            flag(p.pair.horizontal),
            flag(p.pair.vertical),
            de,
            dn,
            r.vertical.map(|v| format!("{v:+.3}")).unwrap_or_default()
        );
    }
    let _ = writeln!(s);
    let fit = &site.fit;
    if let Some(h) = fit.calibration.horizontal {
        let _ = writeln!(
            s,
            "Horizontal      {} pairs, {} scale",
            fit.horizontal_points,
            if cal.options.fixed_scale { "fixed" } else { "free" }
        );
        let _ = writeln!(
            s,
            "  Scale         {:.8} ({:+.1} ppm)",
            h.scale(),
            (h.scale() - 1.0) * 1e6
        );
        let _ = writeln!(s, "  Rotation      {}", format_rotation(conv, h.rotation()));
        let shift = h.shift();
        let _ = writeln!(
            s,
            "  Shift         {ea} {}  {nb} {} (at the origin)",
            conv.format_length(conv.horizontal_values(shift.extend(0.0)).0),
            conv.format_length(conv.horizontal_values(shift.extend(0.0)).1)
        );
        let _ = writeln!(
            s,
            "  Origin        {}",
            conv.format_coord(h.origin.extend(0.0), false)
        );
    }
    if let Some(v) = fit.calibration.vertical {
        let plane = fit.vertical_method == Some(VerticalMethod::InclinedPlane);
        let method = if plane { "inclined plane" } else { "constant shift" };
        let _ = writeln!(s, "Vertical        {} points, {method}", fit.vertical_points);
        let _ = writeln!(s, "  Shift         {}", conv.format_length_with_unit(v.constant));
        if plane {
            let _ = writeln!(
                s,
                "  Slope         N {:+.1} ppm  E {:+.1} ppm",
                v.slope_north * 1e6,
                v.slope_east * 1e6
            );
        }
    }
    if let Some(m) = fit.max_horizontal() {
        let _ = writeln!(
            s,
            "Max residual    H {} (limit {})",
            conv.format_length_with_unit(m),
            conv.format_length_with_unit(cal.options.tolerance_h)
        );
    }
    if let Some(m) = fit.max_vertical() {
        let _ = writeln!(
            s,
            "                V {} (limit {})",
            conv.format_length_with_unit(m),
            conv.format_length_with_unit(cal.options.tolerance_v)
        );
    }
    if !fit.warnings.is_empty() {
        let _ = writeln!(s);
        for w in &fit.warnings {
            let _ = writeln!(s, "! {w}");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_core::calibration::{Calibration, Horizontal, Vertical};
    use survey_model::{Polyline, SurveyPoint};

    fn doc_layer() -> survey_model::LayerId {
        Document::new().current_layer()
    }

    fn point(name: &str, p: DVec3, height_known: bool) -> Entity {
        Entity::Point(SurveyPoint {
            name: name.into(),
            position: p,
            height_known,
            code: String::new(),
            description: String::new(),
            layer: doc_layer(),
        })
    }

    /// A GNSS job (G1–G4 measured, plus a line and a detail point) and local
    /// control C1–C4, related by a known calibration.
    fn job() -> (Document, Calibration) {
        let angle = 0.5f64.to_radians();
        let truth = Calibration {
            horizontal: Some(Horizontal {
                origin: DVec2::new(331_000.0, 6_250_000.0),
                origin_control: DVec2::new(1_000.0, 2_000.0),
                a: 1.0001 * angle.cos(),
                b: 1.0001 * angle.sin(),
            }),
            vertical: Some(Vertical {
                origin: DVec2::new(331_000.0, 6_250_000.0),
                constant: -40.0,
                slope_north: 0.0,
                slope_east: 0.0,
            }),
        };
        let mut doc = Document::new();
        let mut put = |e: Entity| {
            let id = doc.alloc_entity_id();
            doc.set_entity(id, Some(e));
        };
        let corners = [(-200.0, -150.0), (220.0, -170.0), (210.0, 190.0), (-190.0, 160.0)];
        for (i, (e, n)) in corners.iter().enumerate() {
            let g = DVec3::new(331_000.0 + e, 6_250_000.0 + n, 50.0 + i as f64);
            put(point(&format!("G{}", i + 1), g, true));
            put(point(&format!("C{}", i + 1), truth.apply(g), true));
        }
        put(point("D1", DVec3::new(331_010.0, 6_250_020.0, 55.0), true));
        put(point("FAR", DVec3::new(332_000.0, 6_250_000.0, 55.0), true));
        put(Entity::Polyline(Polyline {
            name: String::new(),
            vertices: vec![
                DVec3::new(331_000.0, 6_250_000.0, 0.0),
                DVec3::new(331_050.0, 6_250_000.0, 0.0),
            ],
            bulges: Vec::new(),
            closed: false,
            code: String::new(),
            layer: doc_layer(),
            heights_known: false,
        }));
        (doc, truth)
    }

    fn setup() -> SiteCalibration {
        SiteCalibration {
            pairs: (1..=4)
                .map(|i| PairSetup {
                    measured: format!("G{i}"),
                    control: format!("C{i}"),
                    horizontal: true,
                    vertical: true,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn applies_to_everything_but_the_control() {
        let (mut doc, truth) = job();
        let cal = setup();
        let site = compute(&doc, &cal).expect("fit");
        assert!(site.fit.max_horizontal().expect("h") < 1e-6);
        let (mut cs, summary) = apply(&doc, &cal, &Target::AllExceptControl).expect("apply");
        assert!(summary.contains("applied to 7 objects"), "{summary}");
        assert!(summary.contains("1 point lie"), "{summary}");
        let before = |n: &str| doc.find_point_by_name(n).expect("point").1.position;
        let (d1, c1) = (before("D1"), before("C1"));
        survey_model::Command::apply(&mut cs, &mut doc);
        let after = |n: &str| doc.find_point_by_name(n).expect("point").1.position;
        assert!(after("D1").distance(truth.apply(d1)) < 1e-6);
        assert_eq!(after("C1"), c1, "control stays put");
        assert!(after("G1").distance(c1) < 1e-6, "measured lands on control");
        // The line had no heights: moved in plan only.
        let line = doc
            .entities()
            .find_map(|(_, e)| match e {
                Entity::Polyline(l) => Some(l.clone()),
                _ => None,
            })
            .expect("line");
        assert_eq!(line.vertices[0].z, 0.0);
        assert!(doc.meta().calibration.applied.is_some());
        assert_eq!(doc.meta().calibration.pairs.len(), 4);
    }

    #[test]
    fn typed_control_and_errors() {
        let (doc, _) = job();
        let c1 = doc.find_point_by_name("C1").expect("c1").1.position;
        let mut cal = setup();
        cal.pairs[0].control = format!("{:.4},{:.4}", c1.x, c1.y);
        let site = compute(&doc, &cal).expect("fit");
        assert!(site.pairs[0].control_id.is_none());
        assert!(!site.pairs[0].pair.vertical, "no typed height, no vertical");
        assert_eq!(site.fit.vertical_points, 3);
        cal.pairs[1].measured = "NOPE".into();
        assert!(compute(&doc, &cal).expect_err("unknown point").contains("NOPE"));
        assert!(compute(&doc, &SiteCalibration::default()).is_err());
        let text = report(doc.conventions(), &site, &setup());
        assert!(text.contains("SITE CALIBRATION") && text.contains("Rotation"));
    }

    #[test]
    fn hull_test() {
        let sq = [DVec2::ZERO, DVec2::X, DVec2::ONE, DVec2::Y, DVec2::splat(0.5)];
        let hull = convex_hull(&sq);
        assert_eq!(hull.len(), 4);
        assert!(inside_convex(&hull, DVec2::splat(0.5), 0.0));
        assert!(inside_convex(&hull, DVec2::new(1.005, 0.5), 0.01));
        assert!(!inside_convex(&hull, DVec2::new(2.0, 0.5), 0.01));
    }
}
