//! Area subdivision of a parcel (a closed polyline): cut a given area off
//! with a line parallel to an edge, or with a line swung about a hinge point.

use glam::{DVec2, DVec3};
use survey_core::arc::Arc;
use survey_core::subdivide::{Cut, parallel, signed_area, swing};
use survey_core::{Bearing, Conventions};
use survey_model::{ChangeSet, Document, Entity, EntityId, LayerId, Polyline};

/// Arc tessellation tolerance for parcels (metres).
const ARC_TOLERANCE: f64 = 0.0005;

/// The parcel being divided, relative to `origin`.
pub struct Parcel {
    pub id: EntityId,
    pub name: String,
    layer: LayerId,
    pub origin: DVec3,
    pub polygon: Vec<DVec2>,
    /// Straight edges: (polygon edge index, original segment number from 0).
    pub edges: Vec<(usize, usize)>,
    /// Polygon indices of the original corners.
    pub corners: Vec<usize>,
}

impl Parcel {
    /// The closed polyline as a parcel; `None` if it is not closed.
    pub fn load(id: EntityId, line: &Polyline) -> Option<Self> {
        if !line.closed || line.vertices.len() < 3 {
            return None;
        }
        let origin = line.vertices[0];
        let mut polygon = Vec::new();
        let mut edges = Vec::new();
        let mut corners = Vec::new();
        for (seg, (a, b, bulge)) in line.segments().enumerate() {
            corners.push(polygon.len());
            match Arc::from_bulge(a, b, bulge) {
                Some(arc) => {
                    let pts = arc.tessellate(ARC_TOLERANCE);
                    polygon.extend(pts[..pts.len() - 1].iter().map(|p| (*p - origin).truncate()));
                }
                None => {
                    edges.push((polygon.len(), seg));
                    polygon.push((a - origin).truncate());
                }
            }
        }
        Some(Self {
            id,
            name: if line.name.is_empty() {
                "Parcel".into()
            } else {
                line.name.clone()
            },
            layer: line.layer,
            origin,
            polygon,
            edges,
            corners,
        })
    }

    pub fn area(&self) -> f64 {
        signed_area(&self.polygon).abs()
    }

    /// A plan position relative to the parcel's origin.
    pub fn local(&self, p: DVec3) -> DVec2 {
        (p - self.origin).truncate()
    }

    /// Corner `i` (from 0), relative to the origin.
    pub fn corner(&self, i: usize) -> Option<DVec2> {
        self.corners.get(i).map(|k| self.polygon[*k])
    }

    /// "Edge 2  90°00'00"  25.000" for straight edge `i` (an index into `edges`).
    pub fn edge_label(&self, conv: &Conventions, i: usize) -> String {
        let Some((k, seg)) = self.edges.get(i) else {
            return "—".into();
        };
        let a = self.polygon[*k];
        let b = self.polygon[(*k + 1) % self.polygon.len()];
        let d = b - a;
        let brg = Bearing::from_delta(d.x, d.y)
            .map(|b| conv.format_bearing(b))
            .unwrap_or_default();
        format!("Edge {}  {brg}  {}", seg + 1, conv.format_length(d.length()))
    }
}

/// How the cut line is found.
#[derive(Clone, Copy, Debug)]
pub enum Method {
    /// Parallel to straight edge `i` (an index into the parcel's `edges`).
    Parallel(usize),
    /// Swung about a point (relative to the parcel's origin).
    Swing(DVec2),
}

/// The cuts that take `area` (m²) off the parcel: one for a parallel cut,
/// possibly several for a swing (each with its bearing).
pub fn cuts(parcel: &Parcel, method: Method, area: f64) -> Result<Vec<(Option<Bearing>, Cut)>, String> {
    match method {
        Method::Parallel(i) => {
            let (edge, _) = *parcel
                .edges
                .get(i)
                .ok_or("The parcel has no such straight edge")?;
            parallel(&parcel.polygon, edge, area)
                .map(|c| vec![(None, c)])
                .map_err(|e| e.to_string())
        }
        Method::Swing(hinge) => swing(&parcel.polygon, hinge, area)
            .map(|v| v.into_iter().map(|(b, c)| (Some(b), c)).collect())
            .map_err(|e| e.to_string()),
    }
}

/// Bearing and total length of the cut line.
pub fn cut_line(cut: &Cut) -> (Option<Bearing>, f64) {
    let len = cut.segments.iter().map(|(a, b)| a.distance(*b)).sum();
    let brg = cut.segments.first().and_then(|(a, b)| {
        let d = *b - *a;
        Bearing::from_delta(d.x, d.y)
    });
    (brg, len)
}

/// Draws the cut line, and with `parcels` the two new parcels (named
/// "<parcel> A" for the part cut off and "<parcel> B" for the remainder).
pub fn draw(doc: &Document, parcel: &Parcel, cut: &Cut, parcels: bool) -> (ChangeSet, String) {
    let to_world = |p: DVec2| parcel.origin + p.extend(0.0);
    let mut cs = ChangeSet::new(format!("Subdivide {}", parcel.name));
    for (a, b) in &cut.segments {
        cs.push_insert_entity(Entity::Polyline(Polyline {
            name: format!("{} cut", parcel.name),
            vertices: vec![to_world(*a), to_world(*b)],
            bulges: Vec::new(),
            closed: false,
            code: String::new(),
            layer: doc.current_layer(),
            heights_known: true,
        }));
    }
    if parcels {
        for (suffix, ring) in [("A", &cut.part), ("B", &cut.remainder)] {
            cs.push_insert_entity(Entity::Polyline(Polyline {
                name: format!("{} {suffix}", parcel.name),
                vertices: ring.iter().map(|p| to_world(*p)).collect(),
                bulges: Vec::new(),
                closed: true,
                code: String::new(),
                layer: parcel.layer,
                heights_known: true,
            }));
        }
    }
    let what = if parcels {
        "cut line and parcels"
    } else {
        "cut line"
    };
    (cs, format!("Subdivide {}: drew the {what}", parcel.name))
}

pub fn report(conv: &Conventions, parcel: &Parcel, bearing: Option<Bearing>, cut: &Cut) -> String {
    let mut s = format!("SUBDIVISION  {}\n\n", parcel.name);
    s.push_str(&format!("Parcel area     {}\n", conv.format_area(parcel.area())));
    s.push_str(&format!("Area cut off    {}\n", conv.format_area(cut.part_area)));
    s.push_str(&format!(
        "Remainder       {}\n\n",
        conv.format_area(cut.remainder_area)
    ));
    if let Some(b) = bearing {
        s.push_str(&format!("Cut bearing     {}\n", conv.format_bearing(b)));
    }
    for (i, (a, b)) in cut.segments.iter().enumerate() {
        let (pa, pb) = (parcel.origin + a.extend(0.0), parcel.origin + b.extend(0.0));
        s.push_str(&format!(
            "Cut {}  from {}\n       to   {}\n       length {}\n",
            i + 1,
            conv.format_coord(pa, false),
            conv.format_coord(pb, false),
            conv.format_length_with_unit(a.distance(*b))
        ));
    }
    s
}
