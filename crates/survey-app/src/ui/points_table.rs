//! The points table: every survey point in a sortable, filterable grid with
//! in-place editing. Rows are virtualised, so large jobs stay responsive.

use std::cmp::Ordering;

use egui::{RichText, Sense};
use egui_cad::chrome::{IconButton, switch_row};
use egui_cad::{CadTheme, fonts, icons};
use egui_extras::{Column, TableBuilder};
use survey_core::conventions::CoordOrder;
use survey_model::{ChangeSet, Command, Document, Entity, EntityId, Selection, SurveyPoint};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Name,
    /// First horizontal coordinate in display order.
    A,
    B,
    Z,
    Code,
    Description,
    Layer,
}

const FIELDS: [Field; 7] = [
    Field::Name,
    Field::A,
    Field::B,
    Field::Z,
    Field::Code,
    Field::Description,
    Field::Layer,
];

pub enum PointsAction {
    Select {
        id: EntityId,
        toggle: bool,
    },
    ZoomTo(EntityId),
    Edit(Box<dyn Command>),
    /// A value that could not be applied.
    Error(String),
}

#[derive(Clone, PartialEq)]
struct CacheKey {
    revision: u64,
    selection: u64,
    filter: String,
    selected_only: bool,
    sort: (Field, bool),
}

pub struct PointsWindow {
    pub open: bool,
    filter: String,
    selected_only: bool,
    /// Column and ascending.
    sort: (Field, bool),
    /// Cell being edited, its text, and whether it still needs keyboard focus.
    editing: Option<(EntityId, Field, String, bool)>,
    cache: Option<(CacheKey, Vec<EntityId>)>,
}

impl Default for PointsWindow {
    fn default() -> Self {
        Self {
            open: false,
            filter: String::new(),
            selected_only: false,
            sort: (Field::Name, true),
            editing: None,
            cache: None,
        }
    }
}

/// Numeric-aware comparison so 2 < 10 and 1001 < 1001A.
fn natural(a: &str, b: &str) -> Ordering {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.total_cmp(&y),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

fn coord(doc: &Document, p: &SurveyPoint, f: Field) -> f64 {
    let (a, b) = doc.conventions().horizontal_values(p.position);
    match f {
        Field::A => a,
        Field::B => b,
        _ => p.position.z,
    }
}

impl PointsWindow {
    fn rows(&mut self, doc: &Document, selection: &Selection) -> &[EntityId] {
        let key = CacheKey {
            revision: doc.revision(),
            selection: if self.selected_only {
                selection.revision()
            } else {
                0
            },
            filter: self.filter.clone(),
            selected_only: self.selected_only,
            sort: self.sort,
        };
        if self.cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let needle = self.filter.trim().to_lowercase();
            let mut rows: Vec<(EntityId, &SurveyPoint)> = doc
                .entities()
                .filter_map(|(id, e)| match e {
                    Entity::Point(p) => Some((id, p)),
                    _ => None,
                })
                .filter(|(id, _)| !self.selected_only || selection.contains(*id))
                .filter(|(_, p)| {
                    needle.is_empty()
                        || p.name.to_lowercase().contains(&needle)
                        || p.code.to_lowercase().contains(&needle)
                        || p.description.to_lowercase().contains(&needle)
                })
                .collect();
            let (field, ascending) = self.sort;
            let layer_name = |p: &SurveyPoint| doc.layer(p.layer).map(|l| l.name.clone()).unwrap_or_default();
            rows.sort_by(|(_, x), (_, y)| {
                let o = match field {
                    Field::Name => natural(&x.name, &y.name),
                    Field::A | Field::B => coord(doc, x, field).total_cmp(&coord(doc, y, field)),
                    // Unknown heights sort last.
                    Field::Z => match (x.height_known, y.height_known) {
                        (true, true) => x.position.z.total_cmp(&y.position.z),
                        (a, b) => b.cmp(&a),
                    },
                    Field::Code => natural(&x.code, &y.code),
                    Field::Description => x.description.cmp(&y.description),
                    Field::Layer => layer_name(x).cmp(&layer_name(y)),
                };
                if ascending { o } else { o.reverse() }
            });
            self.cache = Some((key, rows.into_iter().map(|(id, _)| id).collect()));
        }
        self.cache.as_ref().map_or(&[], |(_, rows)| rows.as_slice())
    }

    fn header(&self, doc: &Document, f: Field) -> String {
        let conv = doc.conventions();
        let (a, b) = conv.horizontal_labels();
        match f {
            Field::Name => "Name".into(),
            Field::A => a.to_string(),
            Field::B => b.to_string(),
            Field::Z => conv.axis_labels.height.clone(),
            Field::Code => "Code".into(),
            Field::Description => "Description".into(),
            Field::Layer => "Layer".into(),
        }
    }

    fn text(doc: &Document, p: &SurveyPoint, f: Field) -> String {
        let conv = doc.conventions();
        match f {
            Field::Name => p.name.clone(),
            Field::A | Field::B => conv.format_length(coord(doc, p, f)),
            Field::Z if p.height_known => conv.format_length(p.position.z),
            Field::Z => String::new(),
            Field::Code => p.code.clone(),
            Field::Description => p.description.clone(),
            Field::Layer => doc.layer(p.layer).map(|l| l.name.clone()).unwrap_or_default(),
        }
    }

    /// The edited point, or an error message.
    fn apply(doc: &Document, p: &SurveyPoint, f: Field, text: &str) -> Result<SurveyPoint, String> {
        let conv = doc.conventions();
        let mut p = p.clone();
        let length = |t: &str| conv.parse_length(t).map_err(|e| e.to_string());
        match f {
            Field::Name => p.name = text.trim().to_string(),
            Field::Code => p.code = text.trim().to_string(),
            Field::Description => p.description = text.to_string(),
            Field::A | Field::B => {
                let v = length(text)?;
                let first = f == Field::A;
                let east = match conv.coord_order {
                    CoordOrder::EastingNorthing => first,
                    CoordOrder::NorthingEasting => !first,
                };
                if east {
                    p.position.x = v;
                } else {
                    p.position.y = v;
                }
            }
            Field::Z => {
                if text.trim().is_empty() {
                    p.height_known = false;
                    p.position.z = 0.0;
                } else {
                    p.position.z = length(text)?;
                    p.height_known = true;
                }
            }
            Field::Layer => return Err("Change layers in the Properties panel".into()),
        }
        Ok(p)
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        selection: &Selection,
    ) -> Option<PointsAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Points")
            .open(&mut open)
            .default_size([780.0, 460.0])
            .min_size([600.0, 240.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text(format!(
                                "{} Filter name, code or description",
                                icons::MAGNIFYING_GLASS
                            ))
                            .desired_width(280.0),
                    );
                    switch_row(ui, &mut self.selected_only, "Selected only");
                });
                ui.add_space(4.0);
                let total = doc
                    .entities()
                    .filter(|(_, e)| matches!(e, Entity::Point(_)))
                    .count();
                let rows: Vec<EntityId> = self.rows(doc, selection).to_vec();
                action = self.table(ui, &theme, doc, selection, &rows);
                ui.label(
                    RichText::new(format!(
                        "{} of {total} points · click to select (Shift adds) · double-click a cell to edit",
                        rows.len()
                    ))
                    .small()
                    .color(theme.text_secondary),
                );
            });
        self.open = open;
        if !self.open {
            self.editing = None;
        }
        action
    }

    fn table(
        &mut self,
        ui: &mut egui::Ui,
        theme: &CadTheme,
        doc: &Document,
        selection: &Selection,
        rows: &[EntityId],
    ) -> Option<PointsAction> {
        let mut action = None;
        let row_height = 22.0;
        let available = ui.available_height() - 24.0;
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .sense(Sense::click())
            .max_scroll_height(available.max(120.0))
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(26.0));
        for f in FIELDS {
            // Fixed widths: wide enough for grid coordinates such as 6 912 345.678.
            table = table.column(match f {
                Field::Name => Column::exact(80.0),
                Field::A | Field::B => Column::exact(104.0),
                Field::Z => Column::exact(76.0),
                Field::Code => Column::exact(80.0),
                Field::Description => Column::exact(150.0),
                Field::Layer => Column::remainder().at_least(60.0),
            });
        }
        table
            .header(row_height, |mut header| {
                header.col(|_| {});
                for f in FIELDS {
                    header.col(|ui| {
                        let mut label = self.header(doc, f);
                        if self.sort.0 == f {
                            label.push_str(if self.sort.1 { " ↑" } else { " ↓" });
                        }
                        let text = RichText::new(label)
                            .font(fonts::medium(12.0))
                            .color(theme.text_secondary);
                        if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() {
                            self.sort = if self.sort.0 == f {
                                (f, !self.sort.1)
                            } else {
                                (f, true)
                            };
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, rows.len(), |mut row| {
                    let id = rows[row.index()];
                    let Some(Entity::Point(p)) = doc.entity(id) else {
                        return;
                    };
                    row.set_selected(selection.contains(id));
                    row.col(|ui| {
                        if IconButton::new(icons::CROSSHAIR, "Zoom to point")
                            .size(20.0)
                            .show(ui)
                            .clicked()
                        {
                            action = Some(PointsAction::ZoomTo(id));
                        }
                    });
                    for f in FIELDS {
                        let (_, cell) = row.col(|ui| match &mut self.editing {
                            Some((eid, ef, text, focus)) if *eid == id && *ef == f => {
                                let r = ui.add(
                                    egui::TextEdit::singleline(text)
                                        .font(fonts::mono(12.0))
                                        .desired_width(f32::INFINITY),
                                );
                                if std::mem::take(focus) {
                                    r.request_focus();
                                }
                                if r.lost_focus() {
                                    let commit = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    if commit {
                                        match Self::apply(doc, p, f, text) {
                                            Ok(edited) if edited != *p => {
                                                let label = format!("Edit point {}", p.name);
                                                action = Some(PointsAction::Edit(Box::new(
                                                    ChangeSet::new(label)
                                                        .update_entity(id, Entity::Point(edited)),
                                                )));
                                            }
                                            Ok(_) => {}
                                            Err(e) => action = Some(PointsAction::Error(e)),
                                        }
                                    }
                                    self.editing = None;
                                }
                            }
                            _ => {
                                let value = Self::text(doc, p, f);
                                let numeric = matches!(f, Field::A | Field::B | Field::Z);
                                let mut rt = RichText::new(value);
                                if numeric {
                                    rt = rt.font(fonts::mono(12.0));
                                }
                                if f == Field::Code {
                                    rt = rt.color(theme.accent);
                                }
                                ui.add(egui::Label::new(rt).truncate().selectable(false));
                            }
                        });
                        if cell.double_clicked() && f != Field::Layer {
                            match doc.layer(p.layer).filter(|l| l.locked) {
                                Some(l) => {
                                    action = Some(PointsAction::Error(format!(
                                        "Point {} is on locked layer {}: unlock it to edit",
                                        p.name, l.name
                                    )));
                                }
                                None => self.editing = Some((id, f, Self::text(doc, p, f), true)),
                            }
                        }
                    }
                    if row.response().clicked() && self.editing.is_none() {
                        let toggle = row.response().ctx.input(|i| i.modifiers.shift);
                        action = Some(PointsAction::Select { id, toggle });
                    }
                });
            });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut names = vec!["10", "2", "B", "1001A", "a", "1"];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, vec!["1", "2", "10", "1001A", "a", "B"]);
    }
}
