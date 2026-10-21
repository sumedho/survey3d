//! Conversions between the document and interchange formats.

use std::collections::HashMap;

use survey_io::dxf::{DxfDrawing, DxfEntity, DxfGeometry, DxfLayer};
use survey_model::{Color, Document, Entity, Polyline, SurveyPoint};

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
            }),
        };
        cmd.add(layer, entity);
    }
    cmd
}

/// The whole document as a DXF drawing.
pub fn dxf_export(doc: &Document) -> DxfDrawing {
    let layers = doc
        .layers()
        .iter()
        .map(|l| DxfLayer {
            name: l.name.clone(),
            color: [l.color.0[0], l.color.0[1], l.color.0[2]],
            visible: l.visible,
        })
        .collect();
    let entities = doc
        .entities()
        .map(|(_, e)| DxfEntity {
            layer: doc
                .layer(e.layer())
                .map_or_else(|| "0".to_string(), |l| l.name.clone()),
            geometry: match e {
                Entity::Point(p) => DxfGeometry::Point(p.position),
                Entity::Polyline(l) => DxfGeometry::Polyline {
                    vertices: l.vertices.clone(),
                    bulges: l.bulges.clone(),
                    closed: l.closed,
                },
            },
        })
        .collect();
    DxfDrawing {
        layers,
        entities,
        warnings: Vec::new(),
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
                    geometry: DxfGeometry::Point(DVec3::new(500_005.0, 7_000_002.0, 3.0)),
                },
            ],
            warnings: vec![],
        };
        history.execute(Box::new(dxf_import(&doc, &drawing, "test.dxf")), &mut doc);
        assert_eq!(doc.entity_count(), 2);
        assert!(doc.layer_by_name("KERB").is_some());
        assert_eq!(doc.layers().len(), 2, "layer 0 reused, KERB created");

        let back = read_dxf(write_dxf(&dxf_export(&doc)).as_bytes()).expect("parse");
        let mut kinds: Vec<_> = back.entities.iter().map(|e| e.layer.clone()).collect();
        kinds.sort();
        assert_eq!(kinds, vec!["0".to_string(), "KERB".to_string()]);
        assert!(
            back.entities
                .iter()
                .any(|e| e.geometry == drawing.entities[0].geometry)
        );
    }
}
