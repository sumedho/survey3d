//! Surfaces (TINs) in the document: how each is built, its triangles and its style.

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize, Serializer};
use survey_core::field_to_finish::glob_match;
use survey_core::tin::{Tin, TinInput};

use crate::document::Document;
use crate::entity::{Entity, Polyline, SurveyPoint};
use crate::ids::EntityId;
use crate::layer::Color;

/// Chord tolerance for arcs used as breaklines or boundaries (metres).
const ARC_TOLERANCE: f64 = 0.01;

/// What a surface is built from. Rebuilding re-reads the current project data.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfaceDefinition {
    /// Layers whose points (with known heights) are used; empty means every layer.
    pub point_layers: Vec<String>,
    /// Point code patterns (`*`, `?`); empty means any code.
    pub point_codes: Vec<String>,
    /// Layers whose polylines are breaklines; empty means none (unless codes are given).
    pub breakline_layers: Vec<String>,
    /// Polyline code patterns used as breaklines.
    pub breakline_codes: Vec<String>,
    /// Closed polyline limiting the surface.
    pub boundary: Option<EntityId>,
    /// Closed polylines cut out of the surface.
    pub holes: Vec<EntityId>,
    /// Longest triangle edge kept (plan, metres).
    pub max_edge: Option<f64>,
}

/// Contour display for a surface.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContourStyle {
    pub show: bool,
    /// Height interval between contours (metres).
    pub interval: f64,
    /// Every n-th contour is a major (index) contour.
    pub major_every: u32,
    /// Label major contours with their height.
    pub labels: bool,
}

impl Default for ContourStyle {
    fn default() -> Self {
        Self {
            show: false,
            interval: 1.0,
            major_every: 5,
            labels: true,
        }
    }
}

/// How a surface is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfaceStyle {
    pub visible: bool,
    /// Filled, hill-shaded triangles.
    pub shaded: bool,
    /// Colour the shading by height (otherwise `color`).
    pub elevation_colors: bool,
    /// Triangle edges.
    pub triangles: bool,
    pub color: Color,
    pub contours: ContourStyle,
    /// Colour by sign around zero (cut blue, fill red), for difference surfaces.
    pub isopach: bool,
}

impl Default for SurfaceStyle {
    fn default() -> Self {
        Self {
            visible: true,
            shaded: true,
            elevation_colors: true,
            triangles: false,
            color: Color::rgb(148, 163, 184),
            contours: ContourStyle::default(),
            isopach: false,
        }
    }
}

/// A surface model.
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub name: String,
    /// How to rebuild it; `None` for imported or derived surfaces.
    pub definition: Option<SurfaceDefinition>,
    pub tin: Tin,
    pub style: SurfaceStyle,
}

/// The audit trail records a surface's definition and size, not every triangle.
impl Serialize for Surface {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Summary<'a> {
            name: &'a str,
            definition: &'a Option<SurfaceDefinition>,
            vertices: usize,
            triangles: usize,
        }
        Summary {
            name: &self.name,
            definition: &self.definition,
            vertices: self.tin.vertices.len(),
            triangles: self.tin.triangles.len(),
        }
        .serialize(s)
    }
}

fn matches(patterns: &[String], code: &str) -> bool {
    let first = code.split_whitespace().next().unwrap_or("");
    patterns.iter().any(|p| glob_match(p.trim(), first))
}

fn on_layers(doc: &Document, names: &[String], layer: crate::ids::LayerId) -> bool {
    doc.layer(layer)
        .is_some_and(|l| names.iter().any(|n| n.eq_ignore_ascii_case(&l.name)))
}

/// Plan outline of a closed polyline (arcs tessellated), e.g. a boundary.
pub fn outline(doc: &Document, id: EntityId) -> Option<Vec<DVec2>> {
    match doc.entity(id) {
        Some(Entity::Polyline(l)) => {
            let mut pts: Vec<DVec2> = Vec::new();
            for part in l.tessellate(ARC_TOLERANCE) {
                for p in part {
                    if pts.last().is_none_or(|q| q.distance(p.truncate()) > 1e-9) {
                        pts.push(p.truncate());
                    }
                }
            }
            if pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) < 1e-9 {
                pts.pop();
            }
            (pts.len() >= 3).then_some(pts)
        }
        _ => None,
    }
}

/// What a definition selects but cannot use: points without heights and
/// plan-only breaklines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unused {
    pub points_without_height: usize,
    pub plan_only_breaklines: usize,
    /// The boundary polyline is gone (or no longer outlines an area).
    pub missing_boundary: bool,
    /// Hole polylines that are gone.
    pub missing_holes: usize,
}

impl SurfaceDefinition {
    fn selects_point(&self, doc: &Document, p: &SurveyPoint) -> bool {
        (self.point_layers.is_empty() || on_layers(doc, &self.point_layers, p.layer))
            && (self.point_codes.is_empty() || matches(&self.point_codes, &p.code))
    }

    fn selects_breakline(&self, doc: &Document, id: EntityId, l: &Polyline) -> bool {
        let limit = self.boundary == Some(id) || self.holes.contains(&id);
        let by_layer = !self.breakline_layers.is_empty() && on_layers(doc, &self.breakline_layers, l.layer);
        let by_code = !self.breakline_codes.is_empty() && matches(&self.breakline_codes, &l.code);
        !limit && (by_layer || by_code)
    }

    /// Selected objects left out of the surface for want of heights.
    pub fn unused(&self, doc: &Document) -> Unused {
        let mut unused = Unused::default();
        for (id, e) in doc.entities() {
            match e {
                Entity::Point(p) if !p.height_known && self.selects_point(doc, p) => {
                    unused.points_without_height += 1;
                }
                Entity::Polyline(l) if !l.heights_known && self.selects_breakline(doc, id, l) => {
                    unused.plan_only_breaklines += 1;
                }
                _ => {}
            }
        }
        unused.missing_boundary = self.boundary.is_some_and(|id| outline(doc, id).is_none());
        unused.missing_holes = self
            .holes
            .iter()
            .filter(|id| outline(doc, **id).is_none())
            .count();
        unused
    }

    /// Plain-language notes on what the definition selects but cannot use.
    pub fn warnings(&self, doc: &Document) -> Vec<String> {
        let u = self.unused(doc);
        let mut out = Vec::new();
        if u.points_without_height > 0 {
            out.push(format!(
                "{} selected points have no height and were left out",
                u.points_without_height
            ));
        }
        if u.plan_only_breaklines > 0 {
            out.push(format!(
                "{} selected breaklines are plan-only (no heights) and were left out",
                u.plan_only_breaklines
            ));
        }
        if u.missing_boundary {
            out.push("The boundary polyline has been deleted, so the surface is not clipped; choose a new boundary".into());
        }
        if u.missing_holes > 0 {
            out.push(format!(
                "{} hole polylines have been deleted and are not cut out",
                u.missing_holes
            ));
        }
        out
    }

    /// The points, breaklines and limits this definition selects from `doc`.
    pub fn gather(&self, doc: &Document) -> TinInput {
        let mut input = TinInput {
            max_edge: self.max_edge,
            ..Default::default()
        };
        for (id, e) in doc.entities() {
            match e {
                Entity::Point(p) => {
                    if p.height_known && self.selects_point(doc, p) {
                        input.points.push(p.position);
                    }
                }
                Entity::Text(_) => {}
                Entity::Polyline(l) => {
                    // A plan-only line has no heights to model the ground with.
                    if l.heights_known && self.selects_breakline(doc, id, l) {
                        let mut pts: Vec<DVec3> = Vec::new();
                        for part in l.tessellate(ARC_TOLERANCE) {
                            for p in part {
                                if pts.last().is_none_or(|q| q.distance(p) > 1e-9) {
                                    pts.push(p);
                                }
                            }
                        }
                        input.breaklines.push(pts);
                    }
                }
            }
        }
        input.boundary = self.boundary.and_then(|id| outline(doc, id));
        input.holes = self.holes.iter().filter_map(|id| outline(doc, *id)).collect();
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{ChangeSet, Command};
    use crate::entity::{Polyline, SurveyPoint};
    use crate::ids::LayerId;
    use survey_core::tin::build;

    #[test]
    fn gathers_points_breaklines_and_boundary() {
        let mut doc = Document::new();
        let mut cs = ChangeSet::new("data");
        for (i, (x, y, z)) in [
            (0.0, 0.0, 1.0),
            (10.0, 0.0, 2.0),
            (10.0, 10.0, 3.0),
            (0.0, 10.0, 4.0),
        ]
        .into_iter()
        .enumerate()
        {
            cs.push_insert_entity(Entity::Point(SurveyPoint {
                name: i.to_string(),
                position: DVec3::new(x, y, z),
                height_known: true,
                code: if i == 0 { "NS".into() } else { "TB1".into() },
                description: String::new(),
                layer: LayerId(1),
            }));
        }
        cs.push_insert_entity(Entity::Point(SurveyPoint {
            name: "nz".into(),
            position: DVec3::new(5.0, 5.0, 0.0),
            height_known: false,
            code: String::new(),
            description: String::new(),
            layer: LayerId(1),
        }));
        cs.push_insert_entity(Entity::Polyline(Polyline {
            name: "brk".into(),
            vertices: vec![DVec3::new(0.0, 0.0, 1.0), DVec3::new(10.0, 10.0, 3.0)],
            bulges: Vec::new(),
            closed: false,
            code: "TB".into(),
            layer: LayerId(1),
            heights_known: true,
        }));
        cs.apply(&mut doc);
        let def = SurfaceDefinition {
            breakline_codes: vec!["TB*".into()],
            ..Default::default()
        };
        let input = def.gather(&doc);
        assert_eq!(input.points.len(), 4, "points without heights are left out");
        assert_eq!(input.breaklines.len(), 1);
        let tin = build(&input).expect("tin");
        assert_eq!(tin.triangles.len(), 2);
        // Code filter.
        let def = SurfaceDefinition {
            point_codes: vec!["TB*".into()],
            ..Default::default()
        };
        assert_eq!(def.gather(&doc).points.len(), 3);
    }
}
