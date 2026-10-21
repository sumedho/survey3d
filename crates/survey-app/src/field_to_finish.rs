//! Builds field-to-finish linework from coded points as one command.

use std::collections::{BTreeSet, HashMap};

use survey_core::field_to_finish::{FeatureLibrary, is_generated_name};
use survey_model::{Color, Document, Entity, EntityId, Polyline};

use crate::import_command::{ImportEntities, LayerRef};

pub struct LineworkOptions<'a> {
    /// Only these points (otherwise every point on a visible, unlocked layer).
    pub only: Option<&'a BTreeSet<EntityId>>,
    /// Move coded points onto their feature's layer.
    pub assign_layers: bool,
}

pub struct LineworkSummary {
    pub lines: usize,
    pub replaced: usize,
    pub relayered: usize,
    /// Codes on the processed points that are not in the library.
    pub unknown: BTreeSet<String>,
}

impl LineworkSummary {
    pub fn describe(&self) -> String {
        let mut s = format!("Linework: {} strings", self.lines);
        if self.replaced > 0 {
            s.push_str(&format!(" (replacing {})", self.replaced));
        }
        if self.relayered > 0 {
            s.push_str(&format!(", {} points moved to feature layers", self.relayered));
        }
        if !self.unknown.is_empty() {
            let list: Vec<&str> = self.unknown.iter().map(String::as_str).take(12).collect();
            s.push_str(&format!(". Codes not in the library: {}", list.join(", ")));
        }
        s
    }
}

/// Resolves feature layers by name, creating each missing one once.
struct Layers<'a> {
    doc: &'a Document,
    made: HashMap<String, LayerRef>,
}

impl Layers<'_> {
    fn get(&mut self, cmd: &mut ImportEntities, name: &str, color: [u8; 3]) -> LayerRef {
        if let Some(l) = self.doc.layer_by_name(name) {
            return LayerRef::Existing(l.id);
        }
        *self.made.entry(name.to_ascii_lowercase()).or_insert_with(|| {
            cmd.add_layer(name.to_string(), Color::rgb(color[0], color[1], color[2]), true)
        })
    }
}

/// Builds the linework command. Re-running replaces linework generated earlier
/// (polylines named `CODE….n` with a linework code).
pub fn build_linework(
    doc: &Document,
    library: &FeatureLibrary,
    options: &LineworkOptions<'_>,
) -> (ImportEntities, LineworkSummary) {
    let mut cmd = ImportEntities::new("Field-to-finish linework".into());
    let mut layers = Layers {
        doc,
        made: HashMap::new(),
    };
    let mut summary = LineworkSummary {
        lines: 0,
        replaced: 0,
        relayered: 0,
        unknown: BTreeSet::new(),
    };

    // Coded points in survey (creation) order.
    let points: Vec<(EntityId, &survey_model::SurveyPoint)> = doc
        .entities()
        .filter(|(id, _)| options.only.is_none_or(|only| only.contains(id)) && doc.is_entity_editable(*id))
        .filter_map(|(id, e)| match e {
            Entity::Point(p) if !p.code.trim().is_empty() => Some((id, p)),
            _ => None,
        })
        .collect();
    for (_, p) in &points {
        if library.parse(&p.code).is_none()
            && let Some(first) = p.code.split_whitespace().next()
        {
            summary.unknown.insert(first.to_string());
        }
    }

    let lines = library.build_linework(points.iter().map(|(_, p)| (p.position, p.code.as_str())));
    let new_names: BTreeSet<&str> = lines.iter().map(|l| l.name.as_str()).collect();

    // Earlier generated linework: all of it when processing everything, otherwise
    // only strings being rebuilt.
    for (id, e) in doc.entities() {
        if let Entity::Polyline(l) = e
            && doc.is_entity_editable(id)
            && library
                .find(&l.code)
                .is_some_and(|f| f.linework && is_generated_name(&l.name, &f.code))
            && (options.only.is_none() || new_names.contains(l.name.as_str()))
        {
            cmd.delete(id);
            summary.replaced += 1;
        }
    }

    for line in &lines {
        let feature = &library.codes[line.feature];
        let layer = layers.get(&mut cmd, &feature.layer, feature.color);
        cmd.add(
            layer,
            Entity::Polyline(Polyline {
                name: line.name.clone(),
                vertices: line.vertices.clone(),
                bulges: line.bulges.clone(),
                closed: line.closed,
                code: feature.code.clone(),
                layer: survey_model::LayerId(0),
            }),
        );
        summary.lines += 1;
    }

    if options.assign_layers {
        for (id, p) in &points {
            let Some(parsed) = library.parse(&p.code) else {
                continue;
            };
            let feature = &library.codes[parsed.feature];
            let layer = layers.get(&mut cmd, &feature.layer, feature.color);
            let already = matches!(layer, LayerRef::Existing(l) if l == p.layer);
            if !already {
                cmd.update(*id, layer, Entity::Point((*p).clone()));
                summary.relayered += 1;
            }
        }
    }
    (cmd, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use survey_model::{ChangeSet, History, LayerId, SurveyPoint};

    fn add_points(doc: &mut Document, history: &mut History, coded: &[(f64, f64, &str)]) {
        let mut cs = ChangeSet::new("points");
        for (i, (x, y, code)) in coded.iter().enumerate() {
            cs.push_insert_entity(Entity::Point(SurveyPoint {
                name: (i + 1).to_string(),
                position: DVec3::new(*x, *y, 0.0),
                height_known: false,
                code: (*code).into(),
                description: String::new(),
                layer: LayerId(1),
            }));
        }
        history.execute(Box::new(cs), doc);
    }

    #[test]
    fn builds_layers_and_replaces_on_rerun() {
        let mut doc = Document::new();
        let mut history = History::new("t");
        add_points(
            &mut doc,
            &mut history,
            &[
                (0.0, 0.0, "FNC"),
                (10.0, 0.0, "FNC"),
                (20.0, 5.0, "FNC"),
                (5.0, 5.0, "ZZZ"),
            ],
        );
        let lib = FeatureLibrary::example();
        let opts = LineworkOptions {
            only: None,
            assign_layers: true,
        };
        let (cmd, summary) = build_linework(&doc, &lib, &opts);
        assert_eq!((summary.lines, summary.replaced, summary.relayered), (1, 0, 3));
        assert!(summary.unknown.contains("ZZZ"));
        history.execute(Box::new(cmd), &mut doc);
        let fences = doc.layer_by_name("FENCES").expect("layer created").id;
        let lines: Vec<_> = doc
            .entities()
            .filter_map(|(_, e)| match e {
                Entity::Polyline(l) => Some(l.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 1);
        assert_eq!((lines[0].name.as_str(), lines[0].layer), ("FNC.1", fences));

        // Processing again replaces the string instead of duplicating it.
        let (cmd, summary) = build_linework(&doc, &lib, &opts);
        assert_eq!((summary.lines, summary.replaced, summary.relayered), (1, 1, 0));
        history.execute(Box::new(cmd), &mut doc);
        let count = doc
            .entities()
            .filter(|(_, e)| matches!(e, Entity::Polyline(_)))
            .count();
        assert_eq!(count, 1);
        history.undo(&mut doc);
        history.undo(&mut doc);
        assert!(doc.layer_by_name("FENCES").is_none());
    }
}
