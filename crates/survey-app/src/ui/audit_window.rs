//! Viewer for the project audit trail.

use egui::RichText;
use egui_cad::CadTheme;
use survey_model::{AuditAction, AuditRecord};
use survey_store::AuditVerification;

use crate::project::Project;

#[derive(Default)]
pub struct AuditWindow {
    pub open: bool,
    verification: Option<Result<AuditVerification, String>>,
}

const SHOWN: usize = 300;

impl AuditWindow {
    pub fn show(&mut self, ctx: &egui::Context, project: &Project) {
        if !self.open {
            self.verification = None;
            return;
        }
        let theme = CadTheme::current(ctx);
        let mut open = self.open;
        egui::Window::new("Audit trail")
            .open(&mut open)
            .default_size([640.0, 420.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .button("Verify integrity")
                        .on_hover_text("Recomputes the hash chain of all saved entries")
                        .clicked()
                    {
                        self.verification = Some(project.store.verify_audit().map_err(|e| e.to_string()));
                    }
                    match &self.verification {
                        Some(Ok(v)) if v.is_valid() => {
                            ui.label(
                                RichText::new(format!("✔ {} entries verified", v.entries))
                                    .color(theme.success),
                            );
                        }
                        Some(Ok(v)) => {
                            ui.label(
                                RichText::new(format!(
                                    "✘ chain broken at entry {} — the file was modified outside survey3d",
                                    v.first_invalid.unwrap_or_default()
                                ))
                                .color(theme.error),
                            );
                        }
                        Some(Err(e)) => {
                            ui.label(RichText::new(e).color(theme.error));
                        }
                        None => {}
                    }
                });
                ui.separator();
                let pending = project.history.pending_audit();
                let stored = project.store.audit_entries(SHOWN).unwrap_or_default();
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    egui::Grid::new("audit_grid")
                        .striped(true)
                        .num_columns(4)
                        .spacing([12.0, 3.0])
                        .show(ui, |ui| {
                            for h in ["Time (UTC)", "User", "Action", "Change"] {
                                ui.label(RichText::new(h).small().strong());
                            }
                            ui.end_row();
                            for r in pending.iter().rev() {
                                row(ui, r, Some("unsaved"), &theme);
                            }
                            for e in &stored {
                                row(ui, &e.record, None, &theme);
                            }
                        });
                    if stored.len() == SHOWN {
                        ui.label(RichText::new(format!("Showing the latest {SHOWN} saved entries.")).small());
                    }
                });
            });
        self.open = open;
    }
}

fn row(ui: &mut egui::Ui, r: &AuditRecord, tag: Option<&str>, theme: &CadTheme) {
    ui.label(RichText::new(format_utc(r.timestamp_ms)).monospace().small());
    ui.label(&r.user);
    let action = match r.action {
        AuditAction::Do => RichText::new("edit"),
        AuditAction::Undo => RichText::new("undo").color(theme.warning),
        AuditAction::Redo => RichText::new("redo").color(theme.warning),
    };
    ui.label(action);
    ui.horizontal(|ui| {
        ui.label(&r.label);
        if let Some(tag) = tag {
            ui.label(RichText::new(tag).small().color(theme.accent));
        }
    });
    ui.end_row();
}

/// `YYYY-MM-DD HH:MM:SS` from Unix milliseconds (UTC), without a date library.
pub fn format_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        tod / 3600,
        tod % 3600 / 60,
        tod % 60
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_formatting() {
        assert_eq!(super::format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(super::format_utc(1_792_540_800_000), "2026-10-21 00:00:00");
        assert_eq!(
            super::format_utc(951_782_400_000 + 3_661_000),
            "2000-02-29 01:01:01"
        );
    }
}
