//! A searchable command palette (Ctrl/Cmd+K): type to filter every command,
//! arrow keys to move, Enter to run, Escape to close. Recently used commands
//! are listed first when the query is empty.

use egui::{Align2, CornerRadius, Key, Order, RichText, Sense, Stroke, vec2};

use crate::chrome::keycap;
use crate::fonts;
use crate::icons;
use crate::theme::CadTheme;

#[derive(Clone, Debug)]
pub struct PaletteItem {
    /// Returned when the item is chosen.
    pub id: String,
    pub title: String,
    pub category: String,
    pub description: String,
    pub shortcut: Option<String>,
    pub icon: &'static str,
}

#[derive(Default)]
pub struct CommandPalette {
    open: bool,
    query: String,
    selected: usize,
    recent: Vec<String>,
    focus_pending: bool,
}

const MAX_ROWS: usize = 12;
const MAX_RECENT: usize = 6;

impl CommandPalette {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focus_pending = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn toggle(&mut self) {
        if self.open { self.close() } else { self.open() }
    }

    fn matches<'a>(&self, items: &'a [PaletteItem]) -> Vec<&'a PaletteItem> {
        if self.query.trim().is_empty() {
            let mut out: Vec<&PaletteItem> = self
                .recent
                .iter()
                .filter_map(|id| items.iter().find(|i| &i.id == id))
                .collect();
            out.extend(items.iter().filter(|i| !self.recent.contains(&i.id)));
            return out;
        }
        let mut scored: Vec<(i32, &PaletteItem)> = items
            .iter()
            .filter_map(|item| {
                let title = fuzzy_score(&self.query, &item.title);
                let other =
                    fuzzy_score(&self.query, &format!("{} {}", item.category, item.id)).map(|s| s - 20);
                title.max(other).map(|s| (s, item))
            })
            .collect();
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(_, i)| i).collect()
    }

    /// Shows the palette if open; returns the id of a chosen item.
    pub fn show(&mut self, ctx: &egui::Context, items: &[PaletteItem]) -> Option<String> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let matches = self.matches(items);
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        let shown = matches.len().min(MAX_ROWS);
        if down && shown > 0 {
            self.selected = (self.selected + 1) % shown;
        }
        if up && shown > 0 {
            self.selected = (self.selected + shown - 1) % shown;
        }
        self.selected = self.selected.min(shown.saturating_sub(1));
        if esc {
            self.close();
            return None;
        }
        let mut chosen = enter
            .then(|| matches.get(self.selected).map(|i| i.id.clone()))
            .flatten();

        // Dim the application behind the palette.
        let screen = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::new(Order::Middle, egui::Id::unique("palette_dim")))
            .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(90));

        let width = (screen.width() - 32.0).min(560.0);
        let area = egui::Area::new(egui::Id::unique("egui_cad_command_palette"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_TOP, vec2(0.0, 72.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme.overlay)
                    .stroke(Stroke::new(1.0, theme.border_strong))
                    .corner_radius(CornerRadius::same(theme.radius_large + 2))
                    .shadow(theme.overlay_shadow())
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(icons::MAGNIFYING_GLASS)
                                    .font(fonts::icon(16.0))
                                    .color(theme.text_secondary),
                            );
                            let edit = egui::TextEdit::singleline(&mut self.query)
                                .hint_text("Search commands and tools…")
                                .frame(egui::Frame::NONE)
                                .font(fonts::ui(14.0))
                                .desired_width(f32::INFINITY)
                                .show(ui);
                            if self.focus_pending {
                                edit.response.request_focus();
                                self.focus_pending = false;
                            }
                            if edit.response.changed() {
                                self.selected = 0;
                            }
                        });
                        ui.add_space(4.0);
                        ui.separator();
                        if matches.is_empty() {
                            ui.add_space(8.0);
                            ui.label(RichText::new("No matching commands").color(theme.text_secondary));
                            ui.add_space(8.0);
                        }
                        for (row, item) in matches.iter().take(MAX_ROWS).enumerate() {
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                            if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                                self.selected = row;
                            }
                            let active = row == self.selected;
                            if active {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(theme.radius),
                                    theme.accent_subtle,
                                );
                            }
                            let painter = ui.painter();
                            painter.text(
                                rect.left_center() + vec2(10.0, 0.0),
                                Align2::LEFT_CENTER,
                                item.icon,
                                fonts::icon(15.0),
                                if active {
                                    theme.accent
                                } else {
                                    theme.text_secondary
                                },
                            );
                            painter.text(
                                rect.left_center() + vec2(34.0, 0.0),
                                Align2::LEFT_CENTER,
                                &item.title,
                                fonts::ui(13.0),
                                theme.text,
                            );
                            let mut right = rect.right() - 8.0;
                            if let Some(sc) = &item.shortcut {
                                let mut child = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(egui::Rect::from_min_max(
                                            egui::pos2(rect.right() - 140.0, rect.top() + 6.0),
                                            egui::pos2(right, rect.bottom()),
                                        ))
                                        .layout(egui::Layout::right_to_left(egui::Align::Min)),
                                );
                                let r = keycap(&mut child, sc);
                                right = r.rect.left() - 8.0;
                            }
                            ui.painter().text(
                                egui::pos2(right, rect.center().y),
                                Align2::RIGHT_CENTER,
                                &item.category,
                                fonts::ui(11.5),
                                theme.text_secondary,
                            );
                            let resp = resp.on_hover_text(&item.description);
                            if resp.clicked() {
                                chosen = Some(item.id.clone());
                            }
                        }
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            for (k, what) in [("↑↓", "navigate"), ("Enter", "run"), ("Esc", "close")] {
                                keycap(ui, k);
                                ui.label(RichText::new(what).size(11.0).color(theme.text_secondary));
                                ui.add_space(6.0);
                            }
                        });
                    });
            });

        // Click outside closes.
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
            && ctx
                .pointer_interact_pos()
                .is_some_and(|p| !area.response.rect.contains(p));
        if clicked_outside {
            self.close();
        }
        if let Some(id) = &chosen {
            self.recent.retain(|r| r != id);
            self.recent.insert(0, id.clone());
            self.recent.truncate(MAX_RECENT);
            self.close();
        }
        chosen
    }
}

/// Subsequence match score (higher is better): rewards matches at word starts
/// and consecutive characters. `None` if `query` is not a subsequence of `text`.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut prev_match: Option<usize> = None;
    for (ti, &c) in t.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            let word_start = ti == 0 || !t[ti - 1].is_alphanumeric();
            score += 1;
            if word_start {
                score += 8;
            }
            if prev_match == Some(ti.wrapping_sub(1)) {
                score += 5;
            }
            prev_match = Some(ti);
            qi += 1;
        }
    }
    (qi == q.len()).then(|| score - t.len() as i32 / 8)
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

    #[test]
    fn fuzzy_matching() {
        assert!(fuzzy_score("ze", "Zoom extents").is_some());
        assert!(fuzzy_score("xyz", "Zoom extents").is_none());
        // Prefix / word-start matches rank above scattered ones.
        let prefix = fuzzy_score("pol", "Polyline").unwrap_or(i32::MIN);
        let scattered = fuzzy_score("pol", "Export Overlay").unwrap_or(i32::MIN);
        assert!(prefix > scattered);
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }
}
