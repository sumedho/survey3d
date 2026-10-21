//! ASCII DXF reading and writing for survey linework.
//!
//! Reads `POINT`, `LINE`, `LWPOLYLINE`, `POLYLINE`/`VERTEX` (2D and 3D), `ARC` and
//! `CIRCLE` from the `ENTITIES` section, plus layer names and colours. Arcs and
//! circles become polylines with bulges. Other entity types are skipped and
//! reported as warnings.
//!
//! Writes R12 (`AC1009`) DXF, which virtually every CAD and GIS package reads:
//! points as `POINT`, straight polylines as 3D `POLYLINE`s (preserving heights),
//! and polylines with arcs as 2D `POLYLINE`s with bulges.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glam::DVec3;

use crate::IoError;

#[derive(Clone, Debug, PartialEq)]
pub struct DxfLayer {
    pub name: String,
    pub color: [u8; 3],
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DxfGeometry {
    Point(DVec3),
    Polyline {
        vertices: Vec<DVec3>,
        /// Bulge of the segment starting at each vertex (empty = all straight).
        bulges: Vec<f64>,
        closed: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DxfEntity {
    pub layer: String,
    pub geometry: DxfGeometry,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DxfDrawing {
    pub layers: Vec<DxfLayer>,
    pub entities: Vec<DxfEntity>,
    /// Non-fatal problems (skipped entity types, unsupported orientations).
    pub warnings: Vec<String>,
}

// ====================================================================== reading

type Pair<'a> = (i32, &'a str);

fn tokenize(text: &str) -> Result<Vec<Pair<'_>>, IoError> {
    let mut lines = text.lines();
    let mut pairs = Vec::new();
    while let Some(code) = lines.next() {
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        let code: i32 = code
            .parse()
            .map_err(|_| IoError::Invalid(format!("bad DXF group code '{code}'")))?;
        let value = lines.next().unwrap_or("").trim();
        pairs.push((code, value));
    }
    Ok(pairs)
}

/// The group codes of one DXF object (everything up to the next code 0).
struct Record<'a> {
    kind: &'a str,
    pairs: &'a [Pair<'a>],
}

impl<'a> Record<'a> {
    fn str(&self, code: i32) -> Option<&'a str> {
        self.pairs.iter().find(|(c, _)| *c == code).map(|(_, v)| *v)
    }

    fn f64(&self, code: i32) -> Option<f64> {
        self.str(code).and_then(|v| v.parse().ok())
    }

    fn f64_or(&self, code: i32, default: f64) -> f64 {
        self.f64(code).unwrap_or(default)
    }

    fn int(&self, code: i32) -> i64 {
        self.str(code).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    fn layer(&self) -> String {
        self.str(8).unwrap_or("0").to_string()
    }

    /// `Some(true)` for a mirrored OCS (extrusion −Z), `None` for unsupported orientations.
    fn mirrored(&self) -> Option<bool> {
        let (ex, ey, ez) = (
            self.f64_or(210, 0.0),
            self.f64_or(220, 0.0),
            self.f64_or(230, 1.0),
        );
        if ex.abs() > 1e-9 || ey.abs() > 1e-9 {
            None
        } else {
            Some(ez < 0.0)
        }
    }
}

fn records<'a>(pairs: &'a [Pair<'a>]) -> Vec<Record<'a>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pairs.len() {
        if pairs[i].0 != 0 {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < pairs.len() && pairs[end].0 != 0 {
            end += 1;
        }
        out.push(Record {
            kind: pairs[i].1,
            pairs: &pairs[start..end],
        });
        i = end;
    }
    out
}

/// Reads an ASCII DXF file.
pub fn read_dxf(bytes: &[u8]) -> Result<DxfDrawing, IoError> {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return Err(IoError::Invalid(
            "binary DXF is not supported; save as ASCII DXF".into(),
        ));
    }
    let text = String::from_utf8_lossy(bytes);
    let pairs = tokenize(&text)?;
    let recs = records(&pairs);

    let mut drawing = DxfDrawing::default();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut section = "";
    let mut i = 0;
    while i < recs.len() {
        let r = &recs[i];
        match r.kind {
            "SECTION" => section = r.str(2).unwrap_or(""),
            "ENDSEC" => section = "",
            "LAYER" if section == "TABLES" => {
                let aci = r.int(62);
                let color = r
                    .str(420)
                    .and_then(|v| v.parse::<u32>().ok())
                    .map(|c| [(c >> 16) as u8, (c >> 8) as u8, c as u8])
                    .unwrap_or_else(|| aci_to_rgb(aci.unsigned_abs() as u8));
                drawing.layers.push(DxfLayer {
                    name: r.str(2).unwrap_or("0").to_string(),
                    color,
                    visible: aci >= 0,
                });
            }
            kind if section == "ENTITIES" => {
                let parsed = match kind {
                    "POLYLINE" => {
                        // Collect the following VERTEX records up to SEQEND.
                        let mut verts = Vec::new();
                        while i + 1 < recs.len() && recs[i + 1].kind == "VERTEX" {
                            i += 1;
                            verts.push(&recs[i]);
                        }
                        if i + 1 < recs.len() && recs[i + 1].kind == "SEQEND" {
                            i += 1;
                        }
                        Some(polyline(r, &verts))
                    }
                    "LWPOLYLINE" => Some(lwpolyline(r)),
                    "LINE" => Some(line(r)),
                    "POINT" => Some(point(r)),
                    "ARC" => Some(arc(r)),
                    "CIRCLE" => Some(circle(r)),
                    _ => None,
                };
                match parsed {
                    Some(Some(geometry)) => drawing.entities.push(DxfEntity {
                        layer: r.layer(),
                        geometry,
                    }),
                    Some(None) => {
                        *skipped
                            .entry(format!("{kind} (unsupported orientation)"))
                            .or_default() += 1
                    }
                    None => *skipped.entry(kind.to_string()).or_default() += 1,
                }
            }
            _ => {}
        }
        i += 1;
    }
    for (kind, n) in skipped {
        drawing.warnings.push(format!("skipped {n} {kind}"));
    }
    if drawing.entities.is_empty() {
        let hint = drawing
            .warnings
            .first()
            .map(|w| format!(" ({w})"))
            .unwrap_or_default();
        return Err(IoError::NoPoints(hint));
    }
    Ok(drawing)
}

/// Maps an OCS point to world coordinates (only Z-up and mirrored Z-down are supported).
fn ocs(p: DVec3, mirrored: bool) -> DVec3 {
    if mirrored { DVec3::new(-p.x, p.y, -p.z) } else { p }
}

fn xyz(r: &Record<'_>, base: i32) -> DVec3 {
    DVec3::new(
        r.f64_or(base, 0.0),
        r.f64_or(base + 10, 0.0),
        r.f64_or(base + 20, 0.0),
    )
}

fn point(r: &Record<'_>) -> Option<DxfGeometry> {
    // POINT coordinates are already in world coordinates.
    Some(DxfGeometry::Point(xyz(r, 10)))
}

fn line(r: &Record<'_>) -> Option<DxfGeometry> {
    Some(DxfGeometry::Polyline {
        vertices: vec![xyz(r, 10), xyz(r, 11)],
        bulges: Vec::new(),
        closed: false,
    })
}

fn lwpolyline(r: &Record<'_>) -> Option<DxfGeometry> {
    let mirrored = r.mirrored()?;
    let elevation = r.f64_or(38, 0.0);
    let mut vertices = Vec::new();
    let mut bulges = Vec::new();
    for &(code, value) in r.pairs {
        let v: f64 = value.parse().unwrap_or(0.0);
        match code {
            10 => {
                vertices.push(DVec3::new(v, 0.0, elevation));
                bulges.push(0.0);
            }
            20 => {
                if let Some(last) = vertices.last_mut() {
                    last.y = v;
                }
            }
            42 => {
                if let Some(last) = bulges.last_mut() {
                    *last = v;
                }
            }
            _ => {}
        }
    }
    finish_polyline(vertices, bulges, r.int(70) & 1 != 0, mirrored)
}

fn polyline(r: &Record<'_>, verts: &[&Record<'_>]) -> Option<DxfGeometry> {
    let flags = r.int(70);
    if flags & (16 | 64) != 0 {
        return None; // polygon meshes / polyface meshes
    }
    let is_3d = flags & 8 != 0;
    let mirrored = if is_3d { false } else { r.mirrored()? };
    let elevation = r.f64_or(30, 0.0);
    let mut vertices = Vec::new();
    let mut bulges = Vec::new();
    for v in verts {
        // Skip spline frame control points.
        if v.int(70) & 16 != 0 {
            continue;
        }
        let mut p = xyz(v, 10);
        if !is_3d {
            p.z = elevation;
        }
        vertices.push(p);
        bulges.push(v.f64_or(42, 0.0));
    }
    finish_polyline(vertices, bulges, flags & 1 != 0, mirrored)
}

fn finish_polyline(
    vertices: Vec<DVec3>,
    mut bulges: Vec<f64>,
    closed: bool,
    mirrored: bool,
) -> Option<DxfGeometry> {
    if vertices.len() < 2 {
        return None;
    }
    let vertices = vertices.into_iter().map(|p| ocs(p, mirrored)).collect();
    if mirrored {
        for b in &mut bulges {
            *b = -*b;
        }
    }
    if bulges.iter().all(|b| *b == 0.0) {
        bulges.clear();
    }
    Some(DxfGeometry::Polyline {
        vertices,
        bulges,
        closed,
    })
}

fn arc(r: &Record<'_>) -> Option<DxfGeometry> {
    let mirrored = r.mirrored()?;
    let c = xyz(r, 10);
    let radius = r.f64_or(40, 0.0);
    let (start, end) = (r.f64_or(50, 0.0).to_radians(), r.f64_or(51, 0.0).to_radians());
    let sweep = (end - start).rem_euclid(std::f64::consts::TAU);
    if radius <= 0.0 || sweep == 0.0 {
        return None;
    }
    let at = |a: f64| DVec3::new(c.x + radius * a.cos(), c.y + radius * a.sin(), c.z);
    finish_polyline(
        vec![at(start), at(end)],
        vec![(sweep / 4.0).tan(), 0.0],
        false,
        mirrored,
    )
}

fn circle(r: &Record<'_>) -> Option<DxfGeometry> {
    let mirrored = r.mirrored()?;
    let c = xyz(r, 10);
    let radius = r.f64_or(40, 0.0);
    if radius <= 0.0 {
        return None;
    }
    let (a, b) = (c + DVec3::X * radius, c - DVec3::X * radius);
    finish_polyline(vec![a, b], vec![1.0, 1.0], true, mirrored)
}

// ====================================================================== writing

/// Writes an R12 ASCII DXF.
pub fn write_dxf(drawing: &DxfDrawing) -> String {
    let mut out = String::new();
    let mut pair = |code: i32, value: &dyn std::fmt::Display| {
        let _ = writeln!(out, "{code:>3}\n{value}");
    };
    pair(0, &"SECTION");
    pair(2, &"HEADER");
    pair(9, &"$ACADVER");
    pair(1, &"AC1009");
    pair(0, &"ENDSEC");

    pair(0, &"SECTION");
    pair(2, &"TABLES");
    pair(0, &"TABLE");
    pair(2, &"LAYER");
    pair(70, &drawing.layers.len());
    for l in &drawing.layers {
        let aci = i32::from(rgb_to_aci(l.color));
        pair(0, &"LAYER");
        pair(2, &l.name);
        pair(70, &0);
        pair(62, &if l.visible { aci } else { -aci });
        pair(6, &"CONTINUOUS");
    }
    pair(0, &"ENDTAB");
    pair(0, &"ENDSEC");

    pair(0, &"SECTION");
    pair(2, &"ENTITIES");
    for e in &drawing.entities {
        match &e.geometry {
            DxfGeometry::Point(p) => {
                pair(0, &"POINT");
                pair(8, &e.layer);
                pair(10, &p.x);
                pair(20, &p.y);
                pair(30, &p.z);
            }
            DxfGeometry::Polyline {
                vertices,
                bulges,
                closed,
            } => {
                let has_arcs = bulges.iter().any(|b| *b != 0.0);
                pair(0, &"POLYLINE");
                pair(8, &e.layer);
                pair(66, &1);
                pair(10, &0.0);
                pair(20, &0.0);
                // 2D polylines carry one elevation; 3D polylines keep each vertex height.
                pair(
                    30,
                    &if has_arcs {
                        vertices.first().map_or(0.0, |v| v.z)
                    } else {
                        0.0
                    },
                );
                pair(70, &(i32::from(*closed) | if has_arcs { 0 } else { 8 }));
                for (i, v) in vertices.iter().enumerate() {
                    pair(0, &"VERTEX");
                    pair(8, &e.layer);
                    pair(10, &v.x);
                    pair(20, &v.y);
                    pair(30, &v.z);
                    let bulge = bulges.get(i).copied().unwrap_or(0.0);
                    if bulge != 0.0 {
                        pair(42, &bulge);
                    }
                    pair(70, &if has_arcs { 0 } else { 32 });
                }
                pair(0, &"SEQEND");
                pair(8, &e.layer);
            }
        }
    }
    pair(0, &"ENDSEC");
    pair(0, &"EOF");
    out
}

// ====================================================================== colours

/// RGB of an AutoCAD Colour Index entry (standard palette).
pub fn aci_to_rgb(aci: u8) -> [u8; 3] {
    match aci {
        1 => [255, 0, 0],
        2 => [255, 255, 0],
        3 => [0, 255, 0],
        4 => [0, 255, 255],
        5 => [0, 0, 255],
        6 => [255, 0, 255],
        8 => [128, 128, 128],
        9 => [192, 192, 192],
        10..=249 => {
            // 24 hues in 15° steps; within each, five shades alternating full and half saturation.
            let hue = f64::from((aci / 10 - 1) as u16) * 15.0;
            let sub = aci % 10;
            let value = [255.0, 165.0, 127.0, 76.0, 38.0][usize::from(sub / 2)];
            let pastel = sub % 2 == 1;
            let h = hue / 60.0;
            let x = 1.0 - (h % 2.0 - 1.0).abs();
            let (r, g, b) = match h as u32 {
                0 => (1.0, x, 0.0),
                1 => (x, 1.0, 0.0),
                2 => (0.0, 1.0, x),
                3 => (0.0, x, 1.0),
                4 => (x, 0.0, 1.0),
                _ => (1.0, 0.0, x),
            };
            let shade = |c: f64| {
                let c = if pastel { c + (1.0 - c) * 0.5 } else { c };
                (c * value).round() as u8
            };
            [shade(r), shade(g), shade(b)]
        }
        250..=255 => {
            let v = [51, 91, 132, 173, 214, 255][usize::from(aci - 250)];
            [v, v, v]
        }
        _ => [255, 255, 255], // 7 (white/black) and BYLAYER/BYBLOCK
    }
}

/// Nearest AutoCAD Colour Index for an RGB colour.
pub fn rgb_to_aci(rgb: [u8; 3]) -> u8 {
    (1..=255u8)
        .min_by_key(|&i| {
            let c = aci_to_rgb(i);
            (0..3)
                .map(|k| (i32::from(c[k]) - i32::from(rgb[k])).pow(2))
                .sum::<i32>()
        })
        .unwrap_or(7)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dxf(body: &str) -> String {
        format!("0\nSECTION\n2\nENTITIES\n{body}0\nENDSEC\n0\nEOF\n")
    }

    #[test]
    fn reads_common_entities() {
        let text = dxf(concat!(
            "0\nPOINT\n8\nCTRL\n10\n500000.123\n20\n7000000.456\n30\n12.5\n",
            "0\nLINE\n8\nFENCE\n10\n0\n20\n0\n30\n1\n11\n10\n21\n0\n31\n2\n",
            "0\nLWPOLYLINE\n8\nBDY\n90\n3\n70\n1\n38\n5\n10\n0\n20\n0\n42\n1\n10\n2\n20\n0\n10\n2\n20\n2\n",
            "0\nARC\n8\nKERB\n10\n0\n20\n0\n30\n0\n40\n1\n50\n0\n51\n90\n",
            "0\nCIRCLE\n8\nMH\n10\n5\n20\n5\n30\n0\n40\n0.5\n",
            "0\nTEXT\n8\nANNO\n1\nhello\n",
        ));
        let d = read_dxf(text.as_bytes()).expect("parse");
        assert_eq!(d.entities.len(), 5);
        assert_eq!(
            d.entities[0].geometry,
            DxfGeometry::Point(DVec3::new(500000.123, 7000000.456, 12.5))
        );
        match &d.entities[2].geometry {
            DxfGeometry::Polyline {
                vertices,
                bulges,
                closed,
            } => {
                assert!(*closed);
                assert_eq!(vertices.len(), 3);
                assert_eq!(vertices[1], DVec3::new(2.0, 0.0, 5.0));
                assert_eq!(bulges, &vec![1.0, 0.0, 0.0]);
            }
            other => panic!("{other:?}"),
        }
        match &d.entities[3].geometry {
            DxfGeometry::Polyline { vertices, bulges, .. } => {
                assert!((vertices[1] - DVec3::new(0.0, 1.0, 0.0)).length() < 1e-12);
                assert!((bulges[0] - (std::f64::consts::FRAC_PI_8).tan()).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(d.warnings, vec!["skipped 1 TEXT".to_string()]);
    }

    #[test]
    fn mirrored_ocs_arcs_flip() {
        let text = dxf("0\nARC\n8\n0\n10\n1\n20\n0\n30\n0\n40\n1\n50\n0\n51\n90\n210\n0\n220\n0\n230\n-1\n");
        let d = read_dxf(text.as_bytes()).expect("parse");
        match &d.entities[0].geometry {
            DxfGeometry::Polyline { vertices, bulges, .. } => {
                assert!((vertices[0] - DVec3::new(-2.0, 0.0, 0.0)).length() < 1e-12);
                assert!(bulges[0] < 0.0, "mirroring reverses the sweep");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn write_then_read_round_trips_exactly() {
        let drawing = DxfDrawing {
            layers: vec![
                DxfLayer {
                    name: "BDY".into(),
                    color: [255, 0, 0],
                    visible: true,
                },
                DxfLayer {
                    name: "HIDDEN".into(),
                    color: [0, 0, 255],
                    visible: false,
                },
            ],
            entities: vec![
                DxfEntity {
                    layer: "BDY".into(),
                    geometry: DxfGeometry::Point(DVec3::new(512345.678901, 6912345.123456, 101.25)),
                },
                DxfEntity {
                    layer: "BDY".into(),
                    geometry: DxfGeometry::Polyline {
                        vertices: vec![DVec3::new(1.0, 2.0, 3.0), DVec3::new(4.0, 5.0, 6.5)],
                        bulges: vec![],
                        closed: false,
                    },
                },
                DxfEntity {
                    layer: "HIDDEN".into(),
                    geometry: DxfGeometry::Polyline {
                        vertices: vec![DVec3::new(0.0, 0.0, 2.0), DVec3::new(10.0, 0.0, 2.0)],
                        bulges: vec![1.0, 1.0],
                        closed: true,
                    },
                },
            ],
            warnings: vec![],
        };
        let text = write_dxf(&drawing);
        let back = read_dxf(text.as_bytes()).expect("parse");
        assert_eq!(back.entities, drawing.entities);
        assert_eq!(back.layers, drawing.layers);
    }

    #[test]
    fn aci_palette() {
        assert_eq!(aci_to_rgb(10), [255, 0, 0]);
        assert_eq!(aci_to_rgb(11), [255, 128, 128]);
        assert_eq!(aci_to_rgb(12), [165, 0, 0]);
        assert_eq!(aci_to_rgb(50), [255, 255, 0]);
        for aci in [1u8, 2, 3, 4, 5, 6, 8, 9, 250, 254] {
            assert_eq!(aci_to_rgb(rgb_to_aci(aci_to_rgb(aci))), aci_to_rgb(aci));
        }
    }

    #[test]
    fn rejects_binary_and_empty() {
        assert!(matches!(
            read_dxf(b"AutoCAD Binary DXF\r\n\x1a\0"),
            Err(IoError::Invalid(_))
        ));
        assert!(matches!(read_dxf(dxf("").as_bytes()), Err(IoError::NoPoints(_))));
    }
}
