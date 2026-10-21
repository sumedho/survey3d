//! Reports produced by computations (traverses, listings), kept for the session
//! so they can be read, copied or saved as text.

use egui::RichText;
use egui_cad::chrome::IconButton;
use egui_cad::{CadTheme, fonts, icons};

pub struct Report {
    pub title: String,
    pub text: String,
}

#[derive(Default)]
pub struct ReportsWindow {
    pub open: bool,
    reports: Vec<Report>,
    selected: usize,
}

/// What the user asked to do with a report.
pub enum ReportAction {
    /// Save the report (title, text) as a text file.
    Export(String, String),
}

impl ReportsWindow {
    /// Adds a report, selects it and opens the window.
    pub fn add(&mut self, title: impl Into<String>, text: impl Into<String>) {
        self.reports.push(Report {
            title: title.into(),
            text: text.into(),
        });
        self.selected = self.reports.len() - 1;
        self.open = true;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<ReportAction> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("Reports")
            .open(&mut open)
            .default_size([760.0, 460.0])
            .show(ctx, |ui| {
                if self.reports.is_empty() {
                    ui.label(
                        RichText::new("Traverse reports and listings (command: list) appear here.")
                            .color(theme.text_secondary),
                    );
                    return;
                }
                egui::Panel::left("reports_list")
                    .resizable(false)
                    .exact_size(190.0)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for (i, r) in self.reports.iter().enumerate().rev() {
                                if ui.selectable_label(i == self.selected, &r.title).clicked() {
                                    self.selected = i;
                                }
                            }
                        });
                    });
                let Some(report) = self.reports.get(self.selected) else {
                    return;
                };
                let mut remove = false;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&report.title).font(fonts::medium(13.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if IconButton::new(icons::TRASH, "Remove report")
                            .size(24.0)
                            .show(ui)
                            .clicked()
                        {
                            remove = true;
                        }
                        if IconButton::new(icons::DOWNLOAD_SIMPLE, "Save as text")
                            .size(24.0)
                            .show(ui)
                            .clicked()
                        {
                            action = Some(ReportAction::Export(report.title.clone(), report.text.clone()));
                        }
                        if IconButton::new(icons::CLIPBOARD_TEXT, "Copy to clipboard")
                            .size(24.0)
                            .show(ui)
                            .clicked()
                        {
                            ui.ctx().copy_text(report.text.clone());
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&report.text).font(fonts::mono(12.0))).extend());
                });
                if remove {
                    self.reports.remove(self.selected);
                    self.selected = self.selected.min(self.reports.len().saturating_sub(1));
                }
            });
        self.open = open;
        action
    }
}
