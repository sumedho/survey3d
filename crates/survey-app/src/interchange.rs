//! Conversions between the document and interchange formats.

use std::collections::HashMap;

use survey_core::style::{LineStyle, Part, Prim, Symbol, outline, parts};
use survey_io::dxf::{BlockShape, DxfBlock, DxfDrawing, DxfEntity, DxfGeometry, DxfLayer, DxfLinetype};
use survey_model::{Color, Document, Entity, Polyline, SurveyPoint, TextAlign, TextNote};

use crate::import_command::{ImportEntities, LayerRef};

/// Builds an import command for a DXF drawing. Layers are matched to existing
/// ones by name (case-insensitive); unknown layers are created with the DXF colour.
pub fn dxf_import(doc: &Document, drawing: &DxfDrawing, file_name: &str) -> ImportEntities {
    let mut cmd = ImportEntities::new(format!(
        "Import {} objects from {file_name}",
        drawing.entities.len()
    ));
    let dxf_layers: HashMap<String, &DxfLayer> = drawing
        .layers
        .iter()
        .map(|l| (l.name.to_lowercase(), l))
        .collect();
    let mut refs: HashMap<String, LayerRef> = HashMap::new();
    let mut next: u64 = doc.next_point_name().parse().unwrap_or(1);

    for e in &drawing.entities {
        let key = e.layer.to_lowercase();
        let layer = *refs
            .entry(key.clone())
            .or_insert_with(|| match doc.layer_by_name(&e.layer) {
                Some(existing) => LayerRef::Existing(existing.id),
                None => {
                    let (color, visible) = dxf_layers
                        .get(&key)
                        .map_or(([255, 255, 255], true), |l| (l.color, l.visible));
                    cmd.add_layer(e.layer.clone(), Color::rgb(color[0], color[1], color[2]), visible)
                }
            });
        if matches!(e.geometry, DxfGeometry::Insert { .. }) {
            continue;
        }
        let entity = match &e.geometry {
            DxfGeometry::Point(p) => {
                next += 1;
                Entity::Point(SurveyPoint {
                    name: (next - 1).to_string(),
                    position: *p,
                    height_known: true,
                    code: String::new(),
                    description: String::new(),
                    layer: doc.current_layer(),
                })
            }
            DxfGeometry::Polyline {
                vertices,
                bulges,
                closed,
            } => Entity::Polyline(Polyline {
                name: String::new(),
                vertices: vertices.clone(),
                bulges: bulges.clone(),
                closed: *closed,
                code: String::new(),
                layer: doc.current_layer(),
                heights_known: true,
            }),
            // TEXT is placed by the left end of its baseline; notes by the middle of their capitals.
            DxfGeometry::Text {
                at,
                height,
                text,
                rotation,
            } => {
                let up = glam::DVec2::from_angle(rotation.to_radians()).perp() * (height / 2.0);
                Entity::Text(TextNote {
                    text: text.clone(),
                    position: *at + up.extend(0.0),
                    height: height / DxfExportOptions::default().metres_per_mm(),
                    rotation: rotation.rem_euclid(360.0),
                    align: TextAlign::Left,
                    layer: doc.current_layer(),
                })
            }
            DxfGeometry::Insert { .. } => continue,
        };
        cmd.add(layer, entity);
    }
    cmd
}

/// Sizes for symbols, line types and text in an exported DXF, which (unlike
/// the screen) has to fix them in drawing units: they are the paper sizes of
/// a plan at `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DxfExportOptions {
    /// Plan scale denominator.
    pub scale: f64,
    /// Paper millimetres per screen point of symbol size.
    pub symbol_scale: f64,
    /// Point name height on paper (millimetres); 0 leaves names out.
    pub text_size: f64,
}

impl Default for DxfExportOptions {
    fn default() -> Self {
        let sheet = survey_model::SheetRecord::default();
        Self {
            scale: sheet.scale,
            symbol_scale: sheet.symbol_scale,
            text_size: sheet.text_size,
        }
    }
}

impl DxfExportOptions {
    /// Sizes from a plan sheet.
    pub fn from_sheet(sheet: &survey_model::SheetRecord) -> Self {
        Self {
            scale: sheet.scale,
            symbol_scale: sheet.symbol_scale,
            text_size: sheet.text_size,
        }
    }

    /// Drawing units (metres) per paper millimetre.
    fn metres_per_mm(&self) -> f64 {
        self.scale / 1000.0
    }
}

/// Layer for point names.
pub const NAMES_LAYER: &str = "POINT_NAMES";

fn block_name(symbol: Symbol) -> String {
    format!("S3D_{}", symbol.key().to_uppercase())
}

fn linetype_name(style: LineStyle) -> String {
    format!("S3D_{}", style.key().to_uppercase())
}

/// A symbol's primitives as block shapes (unit radius).
fn symbol_block(symbol: Symbol) -> DxfBlock {
    let p = |c: [f32; 2]| [f64::from(c[0]), f64::from(c[1])];
    let mut shapes = Vec::new();
    for part in parts(symbol) {
        let prim = match part {
            Part::Prim(prim) => prim,
            Part::Ring { outer, inner } => {
                // Quads around the band.
                let n = outer.len();
                for k in 0..n {
                    let next = (k + 1) % n;
                    shapes.push(BlockShape::Solid(vec![
                        p(outer[k]),
                        p(outer[next]),
                        p(inner[next]),
                        p(inner[k]),
                    ]));
                }
                continue;
            }
        };
        match prim {
            Prim::Circle { c, r, fill } => shapes.push(BlockShape::Circle {
                center: p(c),
                radius: f64::from(r),
                filled: fill,
            }),
            Prim::Square { c, h, fill } => {
                let ring = vec![
                    p([c[0] - h, c[1] - h]),
                    p([c[0] + h, c[1] - h]),
                    p([c[0] + h, c[1] + h]),
                    p([c[0] - h, c[1] + h]),
                ];
                shapes.push(if fill {
                    BlockShape::Solid(ring)
                } else {
                    BlockShape::Polygon(ring)
                });
            }
            Prim::Polygon { c, fill, .. } | Prim::Star { c, fill, .. } => {
                let ring: Vec<[f64; 2]> = outline(&prim).into_iter().map(p).collect();
                if !fill {
                    shapes.push(BlockShape::Polygon(ring));
                } else if ring.len() <= 4 {
                    shapes.push(BlockShape::Solid(ring));
                } else {
                    // A fan of triangles from the centre (stars are not convex).
                    for k in 0..ring.len() {
                        shapes.push(BlockShape::Solid(vec![p(c), ring[k], ring[(k + 1) % ring.len()]]));
                    }
                }
            }
            Prim::Segment { a, b } => shapes.push(BlockShape::Line(p(a), p(b))),
            Prim::Hole { .. } => {}
        }
    }
    DxfBlock {
        name: block_name(symbol),
        shapes,
    }
}

/// The whole document as a DXF drawing: layers, points (with their code's
/// symbol as a block and their name as text) and linework (with its code's
/// line type and colour).
pub fn dxf_export(doc: &Document, opts: &DxfExportOptions) -> DxfDrawing {
    let mut layers: Vec<DxfLayer> = doc
        .layers()
        .iter()
        .map(|l| DxfLayer {
            name: l.name.clone(),
            color: [l.color.0[0], l.color.0[1], l.color.0[2]],
            visible: l.visible,
        })
        .collect();
    let library = &doc.meta().feature_codes;
    let k = opts.metres_per_mm();
    // A code's style and the colour it draws in when not by layer.
    let style_of = |code: &str| {
        let f = library.feature_for(code)?;
        let own = match doc.layer_by_name(&f.layer) {
            Some(_) if f.style.by_layer => None,
            _ => Some(f.color),
        };
        Some((f.style, own))
    };
    // In order of first use.
    let mut symbols: Vec<Symbol> = Vec::new();
    let mut linetypes: Vec<LineStyle> = Vec::new();
    let mut entities = Vec::new();
    let mut names = false;
    for (_, e) in doc.entities() {
        let layer = doc
            .layer(e.layer())
            .map_or_else(|| "0".to_string(), |l| l.name.clone());
        match e {
            Entity::Point(p) => {
                let style = style_of(&p.code);
                let color = style.and_then(|(_, c)| c);
                let mut point = DxfEntity::new(layer.clone(), DxfGeometry::Point(p.position));
                point.style.color = color;
                entities.push(point);
                let mut radius = 0.0;
                if let Some((s, _)) = style {
                    radius = f64::from(s.symbol_size) / 2.0 * opts.symbol_scale * k;
                    if !symbols.contains(&s.symbol) {
                        symbols.push(s.symbol);
                    }
                    let mut insert = DxfEntity::new(
                        layer.clone(),
                        DxfGeometry::Insert {
                            block: block_name(s.symbol),
                            at: p.position,
                            scale: radius,
                            rotation: 0.0,
                        },
                    );
                    insert.style.color = color;
                    entities.push(insert);
                }
                if opts.text_size > 0.0 && !p.name.is_empty() {
                    let height = opts.text_size * survey_io::plot::CAP_HEIGHT * k;
                    let gap = radius + 0.6 * k;
                    entities.push(DxfEntity::new(
                        NAMES_LAYER,
                        DxfGeometry::Text {
                            at: p.position + glam::DVec3::new(gap, -height / 2.0, 0.0),
                            height,
                            text: p.name.clone(),
                            rotation: 0.0,
                        },
                    ));
                    names = true;
                }
            }
            Entity::Polyline(l) => {
                let mut line = DxfEntity::new(
                    layer,
                    DxfGeometry::Polyline {
                        vertices: l.vertices.clone(),
                        bulges: l.bulges.clone(),
                        closed: l.closed,
                    },
                );
                // A code's style wins; otherwise the layer's line type.
                let line_style = match style_of(&l.code) {
                    Some((s, color)) => {
                        line.style.color = color;
                        s.line_style
                    }
                    None => doc
                        .layer(l.layer)
                        .map_or(LineStyle::Solid, |layer| layer.line_style),
                };
                if line_style != LineStyle::Solid {
                    line.style.linetype = Some(linetype_name(line_style));
                    if !linetypes.contains(&line_style) {
                        linetypes.push(line_style);
                    }
                }
                entities.push(line);
            }
            Entity::Text(t) => {
                // One TEXT per line, each placed by the left end of its baseline.
                let height = t.height * k;
                let size = height / survey_io::plot::CAP_HEIGHT;
                let along = glam::DVec2::from_angle(t.rotation.to_radians());
                let up = along.perp();
                let lines: Vec<&str> = t.text.lines().collect();
                let fraction = crate::scene_build::align_fraction(t.align);
                for (i, line) in lines.iter().enumerate() {
                    let width = survey_io::plot::text_width(line, size, false);
                    let middle = ((lines.len() - 1) as f64 / 2.0 - i as f64)
                        * size
                        * crate::scene_build::TEXT_LINE_SPACING;
                    let at =
                        t.position.truncate() + along * (-width * fraction) + up * (middle - height / 2.0);
                    entities.push(DxfEntity::new(
                        layer.clone(),
                        DxfGeometry::Text {
                            at: at.extend(t.position.z),
                            height,
                            text: line.to_string(),
                            rotation: t.rotation,
                        },
                    ));
                }
            }
        }
    }
    if names && !layers.iter().any(|l| l.name.eq_ignore_ascii_case(NAMES_LAYER)) {
        layers.push(DxfLayer {
            name: NAMES_LAYER.into(),
            color: [255, 255, 255],
            visible: true,
        });
    }
    // Dash lengths on paper (0.3 mm units, as plan sheets draw them) in metres.
    let unit = 0.3 * k;
    let linetypes = linetypes
        .into_iter()
        .map(|s| DxfLinetype {
            name: linetype_name(s),
            description: s.label().into(),
            pattern: s
                .pattern()
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let v = f64::from(*v);
                    match (i % 2 == 0, v <= 1.0) {
                        (true, true) => 0.0,
                        (true, false) => v * unit,
                        (false, _) => -v * unit,
                    }
                })
                .collect(),
        })
        .collect();
    DxfDrawing {
        layers,
        entities,
        warnings: Vec::new(),
        blocks: symbols.into_iter().map(symbol_block).collect(),
        linetypes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use survey_io::dxf::{read_dxf, write_dxf};
    use survey_model::History;

    #[test]
    fn dxf_round_trip_through_document() {
        let mut doc = Document::new();
        let mut history = History::new("t");
        let drawing = DxfDrawing {
            layers: vec![DxfLayer {
                name: "KERB".into(),
                color: [255, 0, 0],
                visible: true,
            }],
            entities: vec![
                DxfEntity {
                    layer: "KERB".into(),
                    style: Default::default(),
                    geometry: DxfGeometry::Polyline {
                        vertices: vec![
                            DVec3::new(500_000.0, 7_000_000.0, 1.0),
                            DVec3::new(500_010.0, 7_000_000.0, 1.0),
                        ],
                        bulges: vec![0.5, 0.0],
                        closed: false,
                    },
                },
                DxfEntity {
                    layer: "0".into(),
                    style: Default::default(),
                    geometry: DxfGeometry::Point(DVec3::new(500_005.0, 7_000_002.0, 3.0)),
                },
            ],
            ..Default::default()
        };
        history.execute(Box::new(dxf_import(&doc, &drawing, "test.dxf")), &mut doc);
        assert_eq!(doc.entity_count(), 2);
        assert!(doc.layer_by_name("KERB").is_some());
        assert_eq!(doc.layers().len(), 2, "layer 0 reused, KERB created");

        let back =
            read_dxf(write_dxf(&dxf_export(&doc, &DxfExportOptions::default())).as_bytes()).expect("parse");
        let mut kinds: Vec<_> = back.entities.iter().map(|e| e.layer.clone()).collect();
        kinds.sort();
        assert_eq!(kinds, vec!["0".to_string(), "KERB".to_string()]);
        assert!(
            back.entities
                .iter()
                .any(|e| e.geometry == drawing.entities[0].geometry)
        );
    }

    #[test]
    fn dxf_export_symbols_linetypes_and_names() {
        use survey_core::field_to_finish::FeatureCode;
        use survey_model::{ChangeSet, SurveyPoint};
        let mut doc = Document::new();
        let mut history = History::new("t");
        let layer = doc.layers()[0].id;
        let mut meta = doc.meta().clone();
        let mut mh = FeatureCode::new("MH", "Manhole", "SERVICES", [200, 0, 200], false);
        mh.style.symbol = Symbol::CircleX;
        mh.style.by_layer = false;
        let mut kb = FeatureCode::new("KB", "Kerb", "ROAD", [255, 255, 0], true);
        kb.style.line_style = LineStyle::Dashed;
        meta.feature_codes.codes.extend([mh, kb]);
        let cmd = ChangeSet::new("seed")
            .set_meta(meta)
            .insert_entity(Entity::Point(SurveyPoint {
                name: "MH1".into(),
                position: DVec3::new(10.0, 20.0, 5.0),
                height_known: true,
                code: "MH".into(),
                description: String::new(),
                layer,
            }))
            .insert_entity(Entity::Polyline(Polyline {
                name: String::new(),
                vertices: vec![DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0)],
                bulges: Vec::new(),
                closed: false,
                code: "KB".into(),
                layer,
                heights_known: true,
            }));
        history.execute(Box::new(cmd), &mut doc);
        let opts = DxfExportOptions {
            scale: 500.0,
            symbol_scale: 0.2,
            text_size: 2.0,
        };
        let d = dxf_export(&doc, &opts);
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(d.blocks[0].name, "S3D_CIRCLE_X");
        assert_eq!(d.linetypes.len(), 1);
        // Dashed is 6 on, 4 off in 0.3 mm units: 0.9 m and 0.6 m at 1:500.
        assert!((d.linetypes[0].pattern[0] - 0.9).abs() < 1e-9);
        assert!((d.linetypes[0].pattern[1] + 0.6).abs() < 1e-9);
        let insert = d.entities.iter().find_map(|e| match &e.geometry {
            DxfGeometry::Insert { scale, .. } => Some((*scale, e.style.color)),
            _ => None,
        });
        // 7 pt symbol: 0.7 mm radius on paper, 0.35 m at 1:500, in the code's own colour.
        let (scale, color) = insert.expect("insert");
        assert!((scale - 0.35).abs() < 1e-9);
        assert_eq!(color, Some([200, 0, 200]));
        assert!(d.entities.iter().any(|e| e.layer == NAMES_LAYER
            && matches!(&e.geometry, DxfGeometry::Text { text, .. } if text == "MH1")));
        assert!(d.layers.iter().any(|l| l.name == NAMES_LAYER));
        let kerb = d
            .entities
            .iter()
            .find(|e| matches!(e.geometry, DxfGeometry::Polyline { .. }))
            .expect("kerb");
        assert_eq!(kerb.style.linetype.as_deref(), Some("S3D_DASHED"));
    }
}
