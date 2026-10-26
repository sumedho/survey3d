//! Stake-out export: the selected points and lines to a controller file.

use egui::RichText;
use egui_cad::CadTheme;
use survey_io::stakeout::StakeFormat;

pub struct StakeExportWindow {
    pub open: bool,
    format: StakeFormat,
}

impl Default for StakeExportWindow {
    fn default() -> Self {
        Self {
            open: false,
            format: StakeFormat::JobXml,
        }
    }
}

impl StakeExportWindow {
    /// `points` and `lines` are the selection's counts. Returns the format to
    /// export when asked.
    pub fn show(&mut self, ctx: &egui::Context, points: usize, lines: usize) -> Option<StakeFormat> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let mut out = None;
        let mut open = self.open;
        egui::Window::new("Export stake-out file")
            .open(&mut open)
            .default_width(380.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Controller format");
                    egui::ComboBox::from_id_salt("stake_format")
                        .selected_text(self.format.label())
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            for f in StakeFormat::ALL {
                                ui.selectable_value(&mut self.format, f, f.label());
                            }
                        });
                });
                let what = match (points, lines) {
                    (0, 0) => "Nothing is selected: every point is exported.".to_string(),
                    (p, 0) => format!("{p} selected points."),
                    (0, l) => format!("The vertices of {l} selected polylines."),
                    (p, l) => format!("{p} selected points and the vertices of {l} polylines."),
                };
                ui.label(what);
                let note = match self.format {
                    StakeFormat::JobXml => {
                        "Selected polylines also go as lines between their vertices, for line stake-out in Trimble Access."
                    }
                    StakeFormat::Gsi => "Names and codes are limited to 16 characters; heights in millimetres.",
                    StakeFormat::Rw5 => "Stored points (SP records) in metres, for SurvCE and SurvPC jobs.",
                    StakeFormat::Csv => "Name, northing, easting, height and code, comma separated.",
                };
                ui.label(RichText::new(note).small().color(theme.text_secondary));
                ui.add_space(4.0);
                if ui.button(RichText::new("Export…").strong()).clicked() {
                    out = Some(self.format);
                }
            });
        self.open = open;
        out
    }
}
