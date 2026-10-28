//! Image control points (`imgcp`): pick a feature on the image where it is
//! drawn now, then its true position (a pick, a point name or E,N); each pair
//! refits the image (Helmert from two points, or affine from three) as one
//! undoable step. `u` drops the last pair, `f` switches the fit.

use egui_cad::snap::Pick;
use glam::DVec2;
use survey_core::Conventions;
use survey_core::georef::{ControlPoint, FitKind};
use survey_model::{ChangeSet, ImageRecord, RecordId};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome};

pub const ID: &str = "image_control";

#[derive(Default)]
pub struct ImageControlTool {
    image: Option<RecordId>,
    /// The image pixel picked, and where it was picked, waiting for its true position.
    pixel: Option<(DVec2, glam::DVec3)>,
}

impl ImageControlTool {
    fn record<'a>(&self, ctx: &'a ToolCtx<'_>) -> Option<&'a ImageRecord> {
        ctx.doc.record::<ImageRecord>(self.image?)
    }

    fn choose(&mut self, ctx: &mut ToolCtx<'_>, text: &str) -> bool {
        let t = text.trim();
        let found = ctx
            .doc
            .images()
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(t) || t.parse::<i64>().is_ok_and(|n| n == r.id.0))
            .map(|r| (r.id, r.name.clone()));
        match found {
            Some((id, name)) => {
                self.image = Some(id);
                ctx.log.info(format!("Image {name}"));
                true
            }
            None => false,
        }
    }

    /// Stores `rec` refitted, reporting the fit.
    fn store(&self, ctx: &mut ToolCtx<'_>, mut rec: ImageRecord, what: &str) {
        let Some(id) = self.image else { return };
        let conv = ctx.conventions().clone();
        match rec.refit() {
            Ok(Some(f)) => {
                let lines: Vec<String> = rec
                    .control_points
                    .iter()
                    .zip(&f.residuals)
                    .enumerate()
                    .map(|(i, (p, r))| {
                        format!(
                            "  {}  {}{}",
                            i + 1,
                            conv.format_length(r.length()),
                            if p.enabled { "" } else { " (check)" }
                        )
                    })
                    .collect();
                ctx.log.result(format!(
                    "{what}: {} RMS {} from {} points, {} per pixel\n{}",
                    rec.fit.label(),
                    conv.format_length_with_unit(f.rms),
                    f.used,
                    conv.format_length(rec.transform.pixel_size()),
                    lines.join("\n")
                ));
            }
            Ok(None) => ctx.log.result(format!(
                "{what}: {} of {} points for a {} fit{}",
                rec.control_points.iter().filter(|p| p.enabled).count(),
                rec.fit.min_points(),
                rec.fit.label(),
                if rec.control_points.len() == 1 {
                    " (image moved onto the first)"
                } else {
                    ""
                }
            )),
            Err(e) => {
                ctx.log.error(e);
                return;
            }
        }
        let label = format!("{what} on image {}", rec.name);
        ctx.execute(ChangeSet::new(label).update_record(id, rec));
    }
}

impl Tool for ImageControlTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match (self.image, self.pixel) {
            (None, _) => "Image control points — image name:".into(),
            (Some(_), None) => "Image control points — pick a feature on the image (u undo last, f Helmert/affine, Enter to finish):".into(),
            (Some(_), Some(_)) => "Image control points — its true position (pick, point name or E,N):".into(),
        }
    }

    fn activate(&mut self, ctx: &mut ToolCtx<'_>) -> ToolOutcome {
        match ctx.doc.images() {
            [] => {
                ctx.log
                    .warn("There are no images: add one first (Images window or the image tool)");
                ToolOutcome::Done
            }
            [only] => {
                self.image = Some(only.id);
                ctx.log.info(format!("Image {}", only.name));
                ToolOutcome::Continue
            }
            _ => ToolOutcome::Continue,
        }
    }

    fn wants_point(&self) -> bool {
        self.image.is_some()
    }

    fn claims_text(&self, _conv: &Conventions, text: &str) -> bool {
        self.image.is_none() || matches!(text.trim(), "u" | "f")
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        match event {
            ToolEvent::Text(text) if self.image.is_none() => {
                if !self.choose(ctx, text) {
                    ctx.log.error(format!("No image named '{}'", text.trim()));
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                let Some(mut rec) = self.record(ctx).cloned() else {
                    return ToolOutcome::Done;
                };
                match text.trim() {
                    "u" => {
                        self.pixel = None;
                        if rec.control_points.pop().is_some() {
                            self.store(ctx, rec, "Control point removed");
                        } else {
                            ctx.log.warn("No control points to undo");
                        }
                    }
                    "f" => {
                        rec.fit = match rec.fit {
                            FitKind::Helmert => FitKind::Affine,
                            FitKind::Affine => FitKind::Helmert,
                        };
                        let label = format!("{} fit", rec.fit.label());
                        self.store(ctx, rec, &label);
                    }
                    other => ctx.log.error(format!("Unknown option '{other}'")),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Point(pick) => {
                let Some(rec) = self.record(ctx).cloned() else {
                    return ToolOutcome::Done;
                };
                match self.pixel.take() {
                    None => {
                        let Some(px) = rec.transform.inverse(pick.point.truncate()) else {
                            ctx.log.error("The image's placement cannot be inverted");
                            return ToolOutcome::Continue;
                        };
                        let (w, h) = (f64::from(rec.width), f64::from(rec.height));
                        if px.x < -0.5 || px.y < -0.5 || px.x > w + 0.5 || px.y > h + 0.5 {
                            ctx.log.warn("That is off the image: pick a feature on it");
                            return ToolOutcome::Continue;
                        }
                        let px = px.clamp(DVec2::ZERO, DVec2::new(w, h));
                        ctx.log.info(format!("Image pixel {:.1}, {:.1}", px.x, px.y));
                        self.pixel = Some((px, pick.point));
                    }
                    Some((px, _)) => {
                        let mut rec = rec;
                        rec.control_points.push(ControlPoint {
                            pixel: px.to_array(),
                            world: pick.point.truncate().to_array(),
                            enabled: true,
                        });
                        let what = format!("Control point {}", rec.control_points.len());
                        self.store(ctx, rec, &what);
                    }
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish => ToolOutcome::Done,
            ToolEvent::Cancel => {
                if self.pixel.take().is_some() {
                    ToolOutcome::Continue
                } else {
                    ToolOutcome::Done
                }
            }
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        // From the image point picked to where it belongs.
        if let (Some((_, from)), Some(c)) = (self.pixel, cursor) {
            out.rubber.push([from, c.point]);
            out.points.push(from);
        }
    }
}
