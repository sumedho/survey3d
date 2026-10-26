use glam::DVec3;
use serde::{Deserialize, Serialize};
use survey_core::Aabb3;
use survey_core::arc::{Arc, bulge_segments, reverse_bulges};

use crate::ids::LayerId;

/// A named survey point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurveyPoint {
    pub name: String,
    /// `(easting, northing, height)`; `z` is 0 when `height_known` is false.
    pub position: DVec3,
    pub height_known: bool,
    pub code: String,
    pub description: String,
    pub layer: LayerId,
}

/// An open or closed 3D polyline (a "string" in survey terms) whose segments
/// may be circular arcs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    pub name: String,
    pub vertices: Vec<DVec3>,
    /// Bulge of the segment starting at each vertex (see [`survey_core::arc`]);
    /// missing entries are straight. Empty for an all-straight polyline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bulges: Vec<f64>,
    pub closed: bool,
    pub code: String,
    pub layer: LayerId,
    /// Whether the vertices have real heights. A polyline drawn from plan
    /// coordinates only (no heights) has `z` 0 and is left out of surfaces.
    #[serde(default = "yes")]
    pub heights_known: bool,
}

fn yes() -> bool {
    true
}

/// How text sits on its insertion point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Centre,
    Right,
}

impl TextAlign {
    pub const ALL: [TextAlign; 3] = [TextAlign::Left, TextAlign::Centre, TextAlign::Right];

    pub fn key(self) -> &'static str {
        match self {
            TextAlign::Left => "left",
            TextAlign::Centre => "centre",
            TextAlign::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|a| a.key().eq_ignore_ascii_case(s) || (s == "center" && *a == TextAlign::Centre))
    }
}

/// Text on the drawing: street names, easement labels, notes. Its height is
/// in paper millimetres, so it prints the same size at any plan scale (and is
/// drawn at a matching size on screen).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextNote {
    pub text: String,
    /// Insertion point: the middle of the text's height, at the left, centre
    /// or right end as `align` says.
    pub position: DVec3,
    /// Paper millimetres.
    pub height: f64,
    /// Reading direction, degrees anticlockwise from east (0 reads left to right).
    pub rotation: f64,
    pub align: TextAlign,
    pub layer: LayerId,
}

impl TextNote {
    pub const DEFAULT_HEIGHT: f64 = 2.5;
}

impl Polyline {
    pub fn bulge(&self, segment: usize) -> f64 {
        self.bulges.get(segment).copied().unwrap_or(0.0)
    }

    /// Segments as `(start, end, bulge)`.
    pub fn segments(&self) -> impl Iterator<Item = (DVec3, DVec3, f64)> + '_ {
        bulge_segments(&self.vertices, &self.bulges, self.closed)
    }

    /// Bulge of every segment (one per vertex; the last is unused when open).
    pub fn padded_bulges(&self) -> Vec<f64> {
        (0..self.vertices.len()).map(|i| self.bulge(i)).collect()
    }

    /// Removes vertex `i`: the segments either side become one (taking the
    /// bulge of the one before). Refuses (`false`) to leave fewer than two
    /// vertices; a closed figure reduced to two straight vertices opens.
    pub fn remove_vertex(&mut self, i: usize) -> bool {
        if self.vertices.len() <= 2 || i >= self.vertices.len() {
            return false;
        }
        if !self.bulges.is_empty() {
            let mut bulges = self.padded_bulges();
            bulges.remove(i);
            self.bulges = bulges;
        }
        self.vertices.remove(i);
        if self.closed && self.vertices.len() < 3 && self.bulges.iter().all(|b| *b == 0.0) {
            self.closed = false;
        }
        true
    }

    /// Reverses the direction of travel without changing the shape.
    pub fn reverse(&mut self) {
        reverse_bulges(&mut self.vertices, &mut self.bulges, self.closed);
    }

    /// One two-vertex polyline per segment, keeping name, code and layer.
    pub fn explode(&self) -> Vec<Polyline> {
        self.segments()
            .map(|(a, b, bulge)| Polyline {
                vertices: vec![a, b],
                bulges: if bulge == 0.0 {
                    Vec::new()
                } else {
                    vec![bulge, 0.0]
                },
                closed: false,
                ..self.clone()
            })
            .collect()
    }

    fn start(&self) -> DVec3 {
        self.vertices[0]
    }

    fn end(&self) -> DVec3 {
        self.vertices[self.vertices.len() - 1]
    }

    /// Appends `next`, whose first vertex coincides with this polyline's last.
    fn append(&mut self, next: &Polyline) {
        let mut bulges = self.padded_bulges();
        bulges.pop();
        bulges.extend(next.padded_bulges());
        self.vertices.extend_from_slice(&next.vertices[1..]);
        self.bulges = bulges;
    }

    /// Straight-line approximation (arcs tessellated to `tolerance`), for display and hit testing.
    pub fn tessellate(&self, tolerance: f64) -> Vec<Vec<DVec3>> {
        self.segments()
            .map(|(a, b, bulge)| match Arc::from_bulge(a, b, bulge) {
                Some(arc) => arc.tessellate(tolerance),
                None => vec![a, b],
            })
            .collect()
    }
}

/// Joins open polylines whose ends meet (within `tolerance`, in plan) into chains.
/// Each result keeps the name, code and layer of the first polyline in its chain;
/// a chain that returns to its start is closed. Closed and too-short inputs pass through.
pub fn join_polylines(lines: &[Polyline], tolerance: f64) -> Vec<Polyline> {
    let meets = |a: DVec3, b: DVec3| a.truncate().distance(b.truncate()) <= tolerance;
    let (mut open, mut out): (Vec<Polyline>, Vec<Polyline>) = lines
        .iter()
        .cloned()
        .partition(|l| !l.closed && l.vertices.len() >= 2);
    open.reverse(); // pop() takes them in the given order
    while let Some(mut chain) = open.pop() {
        loop {
            let found = open.iter().position(|l| {
                meets(chain.end(), l.start())
                    || meets(chain.end(), l.end())
                    || meets(chain.start(), l.end())
                    || meets(chain.start(), l.start())
            });
            let Some(i) = found else { break };
            let mut next = open.remove(i);
            if meets(chain.end(), next.start()) {
                chain.append(&next);
            } else if meets(chain.end(), next.end()) {
                next.reverse();
                chain.append(&next);
            } else {
                if meets(chain.start(), next.start()) {
                    next.reverse();
                }
                // `next` ends where the chain starts.
                let attrs = chain.clone();
                next.append(&chain);
                chain = Polyline {
                    vertices: next.vertices,
                    bulges: next.bulges,
                    ..attrs
                };
            }
        }
        if chain.vertices.len() > 2 && meets(chain.start(), chain.end()) {
            let n = chain.vertices.len() - 1;
            chain.vertices.truncate(n);
            chain.bulges = chain.padded_bulges();
            chain.closed = true;
        }
        if chain.bulges.iter().all(|b| *b == 0.0) {
            chain.bulges.clear();
        }
        out.push(chain);
    }
    out
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Entity {
    Point(SurveyPoint),
    Polyline(Polyline),
    Text(TextNote),
}

impl Entity {
    pub fn layer(&self) -> LayerId {
        match self {
            Entity::Point(p) => p.layer,
            Entity::Polyline(l) => l.layer,
            Entity::Text(t) => t.layer,
        }
    }

    pub fn set_layer(&mut self, layer: LayerId) {
        match self {
            Entity::Point(p) => p.layer = layer,
            Entity::Polyline(l) => l.layer = layer,
            Entity::Text(t) => t.layer = layer,
        }
    }

    /// Applies `f` to every stored position. Bulges are kept, so `f` must be a
    /// rigid motion that preserves orientation (translation or rotation); text
    /// keeps its reading direction (see [`Self::rotate_text`]).
    pub fn map_positions(&mut self, f: impl Fn(DVec3) -> DVec3) {
        match self {
            Entity::Point(p) => p.position = f(p.position),
            Entity::Text(t) => t.position = f(t.position),
            Entity::Polyline(l) => {
                for v in &mut l.vertices {
                    *v = f(*v);
                }
            }
        }
    }

    /// Turns text with a rotation of the drawing (degrees anticlockwise).
    pub fn rotate_text(&mut self, degrees: f64) {
        if let Entity::Text(t) = self {
            t.rotation = (t.rotation + degrees).rem_euclid(360.0);
        }
    }

    /// The name of a point or polyline; the text of a note.
    pub fn name(&self) -> &str {
        match self {
            Entity::Point(p) => &p.name,
            Entity::Polyline(l) => &l.name,
            Entity::Text(t) => &t.text,
        }
    }

    /// Feature code (text has none).
    pub fn code(&self) -> &str {
        match self {
            Entity::Point(p) => &p.code,
            Entity::Polyline(l) => &l.code,
            Entity::Text(_) => "",
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Entity::Point(_) => "Point",
            Entity::Polyline(_) => "Polyline",
            Entity::Text(_) => "Text",
        }
    }

    pub fn bounds(&self) -> Aabb3 {
        match self {
            Entity::Point(p) => Aabb3::from_point(p.position),
            // Text is sized on paper, so only its insertion point has a place in the world.
            Entity::Text(t) => Aabb3::from_point(t.position),
            Entity::Polyline(l) => {
                let mut b = Aabb3::from_points(l.vertices.iter().copied());
                for (a, c, bulge) in l.segments() {
                    if let Some(arc) = Arc::from_bulge(a, c, bulge) {
                        // Extents of the arc: sample its quadrant-crossing tessellation.
                        let tol = (arc.radius * 1e-4).max(1e-6);
                        for p in arc.tessellate(tol) {
                            b.extend(p);
                        }
                    }
                }
                b
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: &[(f64, f64)], bulges: Vec<f64>) -> Polyline {
        Polyline {
            name: String::new(),
            vertices: points.iter().map(|&(x, y)| DVec3::new(x, y, 0.0)).collect(),
            bulges,
            closed: false,
            code: "KB".into(),
            layer: LayerId(1),
            heights_known: true,
        }
    }

    #[test]
    fn join_chains_in_any_direction() {
        let a = line(&[(0.0, 0.0), (1.0, 0.0)], vec![]);
        let b = line(&[(2.0, 0.0), (1.0, 0.0)], vec![0.5, 0.0]); // reversed, with an arc
        let c = line(&[(5.0, 5.0), (6.0, 5.0)], vec![]);
        let d = line(&[(-1.0, 0.0), (0.0, 0.0)], vec![]);
        let joined = join_polylines(&[a, b, c, d], 1e-6);
        assert_eq!(joined.len(), 2);
        let chain = &joined[0];
        let xs: Vec<f64> = chain.vertices.iter().map(|v| v.x).collect();
        assert_eq!(xs, vec![-1.0, 0.0, 1.0, 2.0]);
        // The arc was reversed with its segment.
        assert_eq!(chain.padded_bulges(), vec![0.0, 0.0, -0.5, 0.0]);
        assert_eq!(chain.code, "KB");
    }

    #[test]
    fn join_closes_a_loop_and_explode_splits() {
        let parts = [
            line(&[(0.0, 0.0), (1.0, 0.0)], vec![]),
            line(&[(1.0, 0.0), (1.0, 1.0)], vec![]),
            line(&[(1.0, 1.0), (0.0, 0.0)], vec![]),
        ];
        let joined = join_polylines(&parts, 1e-6);
        assert_eq!(joined.len(), 1);
        assert!(joined[0].closed && joined[0].vertices.len() == 3);
        let exploded = joined[0].explode();
        assert_eq!(exploded.len(), 3);
        assert!(exploded.iter().all(|l| !l.closed && l.vertices.len() == 2));
    }

    #[test]
    fn removing_vertices() {
        // A closed square with an arc on its second side.
        let mut l = line(
            &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
            vec![0.0, 0.5, 0.0, 0.0],
        );
        l.closed = true;
        assert!(l.remove_vertex(2));
        assert_eq!(l.vertices.len(), 3);
        // The merged side takes the arc of the segment before the removed vertex.
        assert_eq!(l.bulges, vec![0.0, 0.5, 0.0]);
        assert!(l.closed);
        assert!(l.remove_vertex(0));
        assert_eq!(l.vertices.len(), 2);
        assert!(l.closed, "an arc still bounds an area");
        assert!(!l.remove_vertex(0), "two vertices is the least");
        assert!(!l.remove_vertex(9));
        // A straight closed figure down to two vertices opens.
        let mut tri = line(&[(0.0, 0.0), (10.0, 0.0), (5.0, 5.0)], Vec::new());
        tri.closed = true;
        assert!(tri.remove_vertex(1));
        assert!(!tri.closed && tri.bulges.is_empty());
    }
}
