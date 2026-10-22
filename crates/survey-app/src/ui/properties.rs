//! Properties of the selection (or of the project when nothing is selected).

use egui::RichText;
use egui_cad::CadTheme;
use egui_cad::property_grid::{PropertyGridUi, property_grid};
use survey_core::Conventions;
use survey_core::arc::{Arc, polyline_metrics};
use survey_model::{ChangeSet, Command, Document, Entity, EntityId, LayerId, Selection, TextAlign};

/// Shows the panel and returns any edits as commands.
pub fn properties_panel(
    ui: &mut egui::Ui,
    doc: &Document,
    selection: &Selection,
) -> Option<Box<dyn Command>> {
    let ids: Vec<EntityId> = selection.ids().iter().copied().collect();
    match ids.as_slice() {
        [] => project_summary(ui, doc),
        [id] => single(ui, doc, *id),
        many => multiple(ui, doc, many),
    }
}

fn project_summary(ui: &mut egui::Ui, doc: &Document) -> Option<Box<dyn Command>> {
    let meta = doc.meta();
    let (points, lines, texts) = doc.entities().fold((0, 0, 0), |(p, l, t), (_, e)| match e {
        Entity::Point(_) => (p + 1, l, t),
        Entity::Polyline(_) => (p, l + 1, t),
        Entity::Text(_) => (p, l, t + 1),
    });
    property_grid(ui, "project_props", |g| {
        g.section("Project");
        let name = g.text("Name", Some(&meta.name));
        g.readonly("Coordinates", meta.coord_system.display_name());
        g.readonly("Conventions", meta.conventions.name.as_str());
        g.readonly("Units", meta.conventions.linear_unit.label());
        g.section("Contents");
        g.readonly_mono("Points", &points.to_string());
        g.readonly_mono("Polylines", &lines.to_string());
        if texts > 0 {
            g.readonly_mono("Text", &texts.to_string());
        }
        g.readonly_mono("Layers", &doc.layers().len().to_string());
        name.filter(|n| !n.trim().is_empty()).map(|n| {
            let mut m = meta.clone();
            m.name = n.trim().to_string();
            Box::new(ChangeSet::new("Rename project").set_meta(m)) as Box<dyn Command>
        })
    })
}

fn layer_names(doc: &Document) -> (Vec<String>, Vec<LayerId>) {
    doc.layers().iter().map(|l| (l.name.clone(), l.id)).unzip()
}

fn layer_row(g: &mut PropertyGridUi<'_>, doc: &Document, current: Option<LayerId>) -> Option<LayerId> {
    let (names, ids) = layer_names(doc);
    let selected = current.and_then(|c| ids.iter().position(|id| *id == c));
    g.combo("Layer", selected, &names).map(|i| ids[i])
}

fn length_editor(g: &mut PropertyGridUi<'_>, conv: &Conventions, label: &str, value: f64) -> Option<f64> {
    g.parsed(
        label,
        Some(&value),
        |v| conv.format_length(*v),
        |s| conv.parse_length(s).map_err(|e| e.to_string()),
    )
}

fn single(ui: &mut egui::Ui, doc: &Document, id: EntityId) -> Option<Box<dyn Command>> {
    let entity = doc.entity(id)?;
    let conv = doc.conventions();
    let theme = CadTheme::current(ui.ctx());
    ui.label(RichText::new(entity.kind_name()).strong().color(theme.accent));
    let edited: Option<Entity> = property_grid(ui, ("entity_props", id.0), |g| {
        let mut e = entity.clone();
        let mut changed = false;
        match &mut e {
            Entity::Point(p) => {
                g.section("Identity");
                if let Some(v) = g.text("Name", Some(&p.name)) {
                    p.name = v.trim().to_string();
                    changed = true;
                }
                if let Some(v) = g.text("Code", Some(&p.code)) {
                    p.code = v.trim().to_string();
                    changed = true;
                }
                if let Some(v) = g.text("Description", Some(&p.description)) {
                    p.description = v;
                    changed = true;
                }
                if let Some(layer) = layer_row(g, doc, Some(p.layer)) {
                    p.layer = layer;
                    changed = true;
                }
                g.section("Position");
                let (la, lb) = conv.horizontal_labels();
                let (va, vb) = conv.horizontal_values(p.position);
                let (la, lb) = (la.to_string(), lb.to_string());
                let a = length_editor(g, conv, &la, va);
                let b = length_editor(g, conv, &lb, vb);
                if a.is_some() || b.is_some() {
                    let (na, nb) = (a.unwrap_or(va), b.unwrap_or(vb));
                    let (e_, n_) = match conv.coord_order {
                        survey_core::conventions::CoordOrder::EastingNorthing => (na, nb),
                        survey_core::conventions::CoordOrder::NorthingEasting => (nb, na),
                    };
                    p.position.x = e_;
                    p.position.y = n_;
                    changed = true;
                }
                let height = p.height_known.then_some(p.position.z);
                let label = conv.axis_labels.height.clone();
                if let Some(h) = g.parsed_with_hint(
                    &label,
                    Some(&height),
                    "unknown",
                    |h| h.map(|v| conv.format_length(v)).unwrap_or_default(),
                    |s| {
                        if s.trim().is_empty() {
                            Ok(None)
                        } else {
                            conv.parse_length(s).map(Some).map_err(|e| e.to_string())
                        }
                    },
                ) {
                    p.height_known = h.is_some();
                    p.position.z = h.unwrap_or(0.0);
                    changed = true;
                }
            }
            Entity::Polyline(l) => {
                g.section("Identity");
                if let Some(v) = g.text("Name", Some(&l.name)) {
                    l.name = v.trim().to_string();
                    changed = true;
                }
                if let Some(v) = g.text("Code", Some(&l.code)) {
                    l.code = v.trim().to_string();
                    changed = true;
                }
                if let Some(layer) = layer_row(g, doc, Some(l.layer)) {
                    l.layer = layer;
                    changed = true;
                }
                if let Some(c) = g.checkbox("Closed", Some(l.closed)) {
                    l.closed = c;
                    changed = true;
                }
                g.section("Geometry");
                let (len2, len3, area) = polyline_metrics(&l.vertices, &l.bulges, l.closed);
                let arcs: Vec<Arc> = l
                    .segments()
                    .filter_map(|(a, b, bulge)| Arc::from_bulge(a, b, bulge))
                    .collect();
                g.readonly_mono("Vertices", &l.vertices.len().to_string());
                if !arcs.is_empty() {
                    g.readonly_mono("Arc segments", &arcs.len().to_string());
                    // One radius for a single arc or a circle (all arcs share a radius).
                    if arcs.iter().all(|a| (a.radius - arcs[0].radius).abs() < 1e-9) {
                        g.readonly_mono("Radius", &conv.format_length_with_unit(arcs[0].radius));
                    }
                }
                g.readonly_mono("Length (plan)", &conv.format_length_with_unit(len2));
                if l.heights_known {
                    g.readonly_mono("Length (3D)", &conv.format_length_with_unit(len3));
                } else {
                    g.readonly("Heights", "unknown (plan only)");
                }
                let polygon = l.closed && (l.vertices.len() >= 3 || !arcs.is_empty());
                if polygon {
                    let unit = conv.linear_unit.metres_per_unit();
                    g.readonly_mono(
                        "Area",
                        &format!(
                            "{:.*} {}²",
                            usize::from(conv.linear_decimals),
                            area.abs() / (unit * unit),
                            conv.linear_unit.suffix()
                        ),
                    );
                }
            }
            Entity::Text(t) => {
                g.section("Text");
                if let Some(v) = g.text("Text", Some(&t.text)) {
                    t.text = v;
                    changed = true;
                }
                if let Some(layer) = layer_row(g, doc, Some(t.layer)) {
                    t.layer = layer;
                    changed = true;
                }
                let positive = |s: &str| match s.trim().parse::<f64>() {
                    Ok(v) if v > 0.0 => Ok(v),
                    _ => Err("a size in millimetres".to_string()),
                };
                if let Some(h) = g.parsed("Height (mm)", Some(&t.height), |v| format!("{v}"), positive) {
                    t.height = h;
                    changed = true;
                }
                let angle = |s: &str| s.trim().parse::<f64>().map_err(|_| "degrees".to_string());
                if let Some(r) = g.parsed("Rotation (°)", Some(&t.rotation), |v| format!("{v}"), angle) {
                    t.rotation = r.rem_euclid(360.0);
                    changed = true;
                }
                let names: Vec<String> = TextAlign::ALL.iter().map(|a| a.key().to_string()).collect();
                let current = TextAlign::ALL.iter().position(|a| *a == t.align);
                if let Some(i) = g.combo("Align", current, &names) {
                    t.align = TextAlign::ALL[i];
                    changed = true;
                }
            }
        }
        changed.then_some(e)
    });
    edited.map(|e| {
        let label = format!("Edit {} {}", e.kind_name().to_lowercase(), e.name());
        Box::new(ChangeSet::new(label.trim().to_string()).update_entity(id, e)) as Box<dyn Command>
    })
}

fn multiple(ui: &mut egui::Ui, doc: &Document, ids: &[EntityId]) -> Option<Box<dyn Command>> {
    let entities: Vec<(EntityId, &Entity)> = ids
        .iter()
        .filter_map(|id| doc.entity(*id).map(|e| (*id, e)))
        .collect();
    let first_layer = entities.first().map(|(_, e)| e.layer());
    let same_layer = first_layer.filter(|l| entities.iter().all(|(_, e)| e.layer() == *l));
    let theme = CadTheme::current(ui.ctx());
    ui.label(
        RichText::new(format!("{} objects", entities.len()))
            .strong()
            .color(theme.accent),
    );
    property_grid(ui, "multi_props", |g| {
        g.section("Common");
        let layer = layer_row(g, doc, same_layer)?;
        let mut cs = ChangeSet::new(format!("Move {} objects to layer", entities.len()));
        for (id, e) in &entities {
            let mut e = (*e).clone();
            e.set_layer(layer);
            cs.push_update_entity(*id, e);
        }
        Some(Box::new(cs) as Box<dyn Command>)
    })
}
