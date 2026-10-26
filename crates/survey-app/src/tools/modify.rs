//! Move, copy, rotate and scale.
//!
//! Each works on the current selection; with nothing selected the tool first
//! asks for objects (click or window, Enter to finish). Displacements are in
//! plan only, so heights never change by snapping to something at another level.

use egui_cad::snap::Pick;
use glam::{DVec2, DVec3};
use survey_core::point_input::ResolvedPoint;
use survey_core::{Bearing, Conventions};
use survey_model::{ChangeSet, Entity, EntityId};

use super::{Preview, Tool, ToolCtx, ToolEvent, ToolOutcome, resolved};

pub const MOVE_ID: &str = "move";
pub const COPY_ID: &str = "copy";
pub const ROTATE_ID: &str = "rotate";
pub const SCALE_ID: &str = "scale";

/// Ghost previews are skipped beyond this many vertices.
const MAX_GHOST_VERTICES: usize = 20_000;

/// Collects the objects a modify tool works on.
#[derive(Default)]
pub struct Gather {
    pub ids: Vec<EntityId>,
    /// Outline of the objects (for previews), relative to nothing: world coordinates.
    ghost_lines: Vec<Vec<DVec3>>,
    ghost_points: Vec<DVec3>,
}

pub enum Gathered {
    /// Still picking objects.
    Picking,
    Ready,
    Cancelled,
}

impl Gather {
    pub fn is_ready(&self) -> bool {
        !self.ids.is_empty()
    }

    /// Takes the editable selection, if any.
    pub fn activate(&mut self, ctx: &mut ToolCtx<'_>) -> bool {
        let doc = ctx.doc;
        ctx.selection.retain(|id| doc.is_entity_editable(id));
        if ctx.selection.is_empty() {
            return false;
        }
        self.ids = ctx.selection.ids().iter().copied().collect();
        self.capture(ctx);
        true
    }

    /// Handles an event while picking objects.
    pub fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: &ToolEvent<'_>) -> Gathered {
        match event {
            ToolEvent::Point(_) => {
                if let Some(id) = ctx.hovered {
                    ctx.selection.toggle(id);
                }
                Gathered::Picking
            }
            ToolEvent::Region(ids) => {
                for id in ids {
                    ctx.selection.insert(*id);
                }
                Gathered::Picking
            }
            ToolEvent::Finish => {
                if self.activate(ctx) {
                    ctx.log.info(objects(self.ids.len()));
                    Gathered::Ready
                } else {
                    Gathered::Cancelled
                }
            }
            ToolEvent::Cancel => Gathered::Cancelled,
            ToolEvent::Text(_) => Gathered::Picking,
        }
    }

    fn capture(&mut self, ctx: &ToolCtx<'_>) {
        self.ghost_lines.clear();
        self.ghost_points.clear();
        let mut count = 0;
        for id in &self.ids {
            match ctx.doc.entity(*id) {
                Some(Entity::Point(p)) => {
                    self.ghost_points.push(p.position);
                    count += 1;
                }
                Some(Entity::Text(t)) => {
                    self.ghost_points.push(t.position);
                    count += 1;
                }
                Some(Entity::Dimension(d)) => {
                    let [a, b] = d.line();
                    self.ghost_lines
                        .push(vec![d.from, a.extend(d.line_at.z), b.extend(d.line_at.z), d.to]);
                    count += 4;
                }
                Some(Entity::Polyline(l)) => {
                    for part in l.tessellate(0.01) {
                        count += part.len();
                        self.ghost_lines.push(part);
                    }
                }
                None => {}
            }
            if count > MAX_GHOST_VERTICES {
                self.ghost_lines.clear();
                self.ghost_points.clear();
                return;
            }
        }
    }

    /// Draws the objects transformed by `f`.
    pub fn preview(&self, f: impl Fn(DVec3) -> DVec3, out: &mut Preview) {
        out.polylines
            .extend(self.ghost_lines.iter().map(|l| l.iter().map(|p| f(*p)).collect()));
        out.points.extend(self.ghost_points.iter().map(|p| f(*p)));
    }
}

/// "1 object", "3 objects".
pub fn objects(n: usize) -> String {
    if n == 1 {
        "1 object".into()
    } else {
        format!("{n} objects")
    }
}

const PICK_PROMPT: &str = "Select objects (click or window), Enter when done:";

// ---------------------------------------------------------------- move / copy

pub struct MoveTool {
    copy: bool,
    gather: Gather,
    base: Option<ResolvedPoint>,
    copies: usize,
}

impl MoveTool {
    pub fn new(copy: bool) -> Self {
        Self {
            copy,
            gather: Gather::default(),
            base: None,
            copies: 0,
        }
    }

    fn verb(&self) -> &'static str {
        if self.copy { "Copy" } else { "Move" }
    }

    fn commit(&mut self, ctx: &mut ToolCtx<'_>, to: DVec3) -> ToolOutcome {
        let Some(base) = self.base else {
            return ToolOutcome::Continue;
        };
        let d = (to - base.position).truncate().extend(0.0);
        let n = self.gather.ids.len();
        let conv = ctx.conventions();
        let summary = format!("dE {}  dN {}", conv.format_length(d.x), conv.format_length(d.y));
        let mut cs = ChangeSet::new(format!("{} {}", self.verb(), objects(n)));
        let mut names = ctx.doc.point_names();
        for id in &self.gather.ids {
            let Some(mut e) = ctx.doc.entity(*id).cloned() else {
                continue;
            };
            e.map_positions(|p| p + d);
            if self.copy {
                if let Entity::Point(p) = &mut e {
                    p.name = names.next().unwrap_or_default();
                }
                cs.push_insert_entity(e);
            } else {
                cs.push_update_entity(*id, e);
            }
        }
        ctx.log
            .result(format!("{} {}: {summary}", self.verb(), objects(n)));
        ctx.execute(cs);
        if self.copy {
            self.copies += 1;
            ToolOutcome::Continue
        } else {
            ToolOutcome::Done
        }
    }
}

impl Tool for MoveTool {
    fn id(&self) -> &'static str {
        if self.copy { COPY_ID } else { MOVE_ID }
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        let verb = self.verb();
        match (self.gather.is_ready(), self.base) {
            (false, _) => format!("{verb} — {PICK_PROMPT}"),
            (true, None) => format!("{verb} — base point:"),
            (true, Some(_)) if self.copy && self.copies > 0 => {
                "Copy — next second point (Enter to finish):".to_string()
            }
            (true, Some(_)) => format!("{verb} — second point:"),
        }
    }

    fn activate(&mut self, ctx: &mut ToolCtx<'_>) -> ToolOutcome {
        self.gather.activate(ctx);
        ToolOutcome::Continue
    }

    fn wants_point(&self) -> bool {
        self.gather.is_ready()
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.base
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        if !self.gather.is_ready() {
            return match self.gather.on_event(ctx, &event) {
                Gathered::Cancelled => ToolOutcome::Done,
                Gathered::Picking | Gathered::Ready => ToolOutcome::Continue,
            };
        }
        match event {
            ToolEvent::Point(pick) => match self.base {
                None => {
                    self.base = Some(resolved(&pick));
                    ToolOutcome::Continue
                }
                Some(_) => self.commit(ctx, pick.point),
            },
            ToolEvent::Text(other) => {
                ctx.log.error(format!("Unknown option '{other}'"));
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        if let (Some(base), Some(c)) = (self.base, cursor) {
            let d = (c.point - base.position).truncate().extend(0.0);
            self.gather.preview(|p| p + d, out);
            out.rubber.push([base.position, c.point]);
        }
    }
}

// ---------------------------------------------------------------- rotate

/// Rotates about a base point. The angle is given as a *change of bearing*: a
/// reference bearing (picked, typed, or Enter for north) and the bearing it
/// should become — e.g. rotate a local survey so a line on 90° reads 91°12'30".
#[derive(Default)]
pub struct RotateTool {
    gather: Gather,
    base: Option<ResolvedPoint>,
    reference: Option<Bearing>,
}

/// Rotates `p` about `centre` (plan) clockwise by `angle` radians.
fn rotate_cw(p: DVec3, centre: DVec2, angle: f64) -> DVec3 {
    (centre + DVec2::from_angle(-angle).rotate(p.truncate() - centre)).extend(p.z)
}

impl RotateTool {
    fn bearing_to(&self, p: DVec3) -> Option<Bearing> {
        let base = self.base?.position;
        Bearing::from_delta(p.x - base.x, p.y - base.y)
    }

    fn commit(&mut self, ctx: &mut ToolCtx<'_>, target: Bearing) -> ToolOutcome {
        let (Some(base), Some(reference)) = (self.base, self.reference) else {
            return ToolOutcome::Continue;
        };
        let angle = target.radians() - reference.radians();
        let centre = base.position.truncate();
        let n = self.gather.ids.len();
        let mut cs = ChangeSet::new(format!("Rotate {}", objects(n)));
        for id in &self.gather.ids {
            if let Some(mut e) = ctx.doc.entity(*id).cloned() {
                e.map_positions(|p| rotate_cw(p, centre, angle));
                e.rotate_text(-angle.to_degrees());
                cs.push_update_entity(*id, e);
            }
        }
        let conv = ctx.conventions();
        ctx.log.result(format!(
            "Rotated {} by {} (clockwise)",
            objects(n),
            conv.format_angle(survey_core::Angle::from_radians(angle).normalized_signed())
        ));
        ctx.execute(cs);
        ToolOutcome::Done
    }
}

impl Tool for RotateTool {
    fn id(&self) -> &'static str {
        ROTATE_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match (self.gather.is_ready(), self.base, self.reference) {
            (false, _, _) => format!("Rotate — {PICK_PROMPT}"),
            (true, None, _) => "Rotate — centre of rotation:".into(),
            (true, Some(_), None) => "Rotate — reference bearing (pick, type, or Enter for 0°):".into(),
            (true, Some(_), Some(_)) => "Rotate — new bearing (pick or type):".into(),
        }
    }

    fn activate(&mut self, ctx: &mut ToolCtx<'_>) -> ToolOutcome {
        self.gather.activate(ctx);
        ToolOutcome::Continue
    }

    fn wants_point(&self) -> bool {
        self.gather.is_ready()
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        self.base.is_some() && conv.parse_bearing(text).is_ok()
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.base
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        if !self.gather.is_ready() {
            return match self.gather.on_event(ctx, &event) {
                Gathered::Cancelled => ToolOutcome::Done,
                Gathered::Picking | Gathered::Ready => ToolOutcome::Continue,
            };
        }
        match event {
            ToolEvent::Point(pick) => {
                if self.base.is_none() {
                    self.base = Some(resolved(&pick));
                    return ToolOutcome::Continue;
                }
                let Some(bearing) = self.bearing_to(pick.point) else {
                    ctx.log.warn("Pick a point away from the centre");
                    return ToolOutcome::Continue;
                };
                if self.reference.is_none() {
                    self.reference = Some(bearing);
                    ToolOutcome::Continue
                } else {
                    self.commit(ctx, bearing)
                }
            }
            ToolEvent::Text(text) => {
                match ctx.conventions().parse_bearing(text) {
                    Ok(b) if self.reference.is_none() && self.base.is_some() => self.reference = Some(b),
                    Ok(b) if self.base.is_some() => return self.commit(ctx, b),
                    Ok(_) => ctx.log.error("Pick the centre of rotation first"),
                    Err(e) => ctx.log.error(e.to_string()),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish if self.base.is_some() && self.reference.is_none() => {
                self.reference = Some(Bearing::from_degrees(0.0));
                ToolOutcome::Continue
            }
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }

    fn preview(&self, _conv: &Conventions, cursor: Option<&Pick>, out: &mut Preview) {
        let (Some(base), Some(c)) = (self.base, cursor) else {
            return;
        };
        out.rubber.push([base.position, c.point]);
        if let (Some(reference), Some(now)) = (self.reference, self.bearing_to(c.point)) {
            let angle = now.radians() - reference.radians();
            let centre = base.position.truncate();
            self.gather.preview(|p| rotate_cw(p, centre, angle), out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clockwise_rotation_follows_bearings() {
        // A point due north of the centre, rotated 90° clockwise, ends up due east.
        let p = rotate_cw(DVec3::new(0.0, 10.0, 3.0), DVec2::ZERO, 90f64.to_radians());
        assert!((p - DVec3::new(10.0, 0.0, 3.0)).length() < 1e-12);
    }
}

/// Scales the plan about a base point: by a factor (`1.000123`, e.g. a
/// combined scale factor for ground to grid) or by a measured length and the
/// length it should be (`100.012 100.000`). Heights, text and symbol sizes
/// stay as they are.
#[derive(Default)]
pub struct ScaleTool {
    gather: Gather,
    base: Option<ResolvedPoint>,
}

/// The scale factor in `text`: one number, or two lengths (as measured, as
/// it should be).
fn scale_factor(conv: &Conventions, text: &str) -> Option<Result<f64, String>> {
    let parts: Vec<&str> = text.split([' ', ',']).filter(|t| !t.is_empty()).collect();
    let factor = match parts[..] {
        [f] => f.parse::<f64>().ok()?,
        [a, b] => {
            let (a, b) = (conv.parse_length(a).ok()?, conv.parse_length(b).ok()?);
            if a <= 0.0 {
                return Some(Err("The measured length must be greater than zero".into()));
            }
            b / a
        }
        _ => return None,
    };
    Some(if factor.is_finite() && factor > 0.0 {
        Ok(factor)
    } else {
        Err("The scale factor must be greater than zero".into())
    })
}

impl ScaleTool {
    fn commit(&mut self, ctx: &mut ToolCtx<'_>, factor: f64) -> ToolOutcome {
        let Some(base) = self.base else {
            return ToolOutcome::Continue;
        };
        let centre = base.position.truncate();
        let n = self.gather.ids.len();
        let mut cs = ChangeSet::new(format!("Scale {} by {factor:.9}", objects(n)));
        for id in &self.gather.ids {
            if let Some(mut e) = ctx.doc.entity(*id).cloned() {
                e.map_positions(|p| (centre + (p.truncate() - centre) * factor).extend(p.z));
                cs.push_update_entity(*id, e);
            }
        }
        ctx.log.result(format!(
            "Scaled {} by {factor:.9} ({:+.1} ppm) about {}",
            objects(n),
            (factor - 1.0) * 1e6,
            ctx.conventions().format_coord(base.position, false)
        ));
        ctx.execute(cs);
        ToolOutcome::Done
    }
}

impl Tool for ScaleTool {
    fn id(&self) -> &'static str {
        SCALE_ID
    }

    fn prompt(&self, _conv: &Conventions) -> String {
        match (self.gather.is_ready(), self.base) {
            (false, _) => format!("Scale — {PICK_PROMPT}"),
            (true, None) => "Scale — base point (stays put):".into(),
            (true, Some(_)) => {
                "Scale — factor (e.g. 0.99982), or measured and true lengths (100.012 100.000):".into()
            }
        }
    }

    fn activate(&mut self, ctx: &mut ToolCtx<'_>) -> ToolOutcome {
        self.gather.activate(ctx);
        ToolOutcome::Continue
    }

    fn wants_point(&self) -> bool {
        self.gather.is_ready() && self.base.is_none()
    }

    fn claims_text(&self, conv: &Conventions, text: &str) -> bool {
        self.base.is_some() && scale_factor(conv, text).is_some()
    }

    fn base_point(&self) -> Option<ResolvedPoint> {
        self.base
    }

    fn on_event(&mut self, ctx: &mut ToolCtx<'_>, event: ToolEvent<'_>) -> ToolOutcome {
        if !self.gather.is_ready() {
            return match self.gather.on_event(ctx, &event) {
                Gathered::Cancelled => ToolOutcome::Done,
                Gathered::Picking | Gathered::Ready => ToolOutcome::Continue,
            };
        }
        match event {
            ToolEvent::Point(pick) => {
                if self.base.is_none() {
                    self.base = Some(resolved(&pick));
                }
                ToolOutcome::Continue
            }
            ToolEvent::Text(text) => {
                let conv = ctx.conventions().clone();
                match scale_factor(&conv, text) {
                    Some(Ok(f)) if self.base.is_some() => return self.commit(ctx, f),
                    Some(Ok(_)) => ctx.log.error("Pick or type the base point first"),
                    Some(Err(e)) => ctx.log.error(e),
                    None => ctx.log.error("Type a factor, or the measured and true lengths"),
                }
                ToolOutcome::Continue
            }
            ToolEvent::Region(_) => ToolOutcome::Continue,
            ToolEvent::Finish | ToolEvent::Cancel => ToolOutcome::Done,
        }
    }

    fn preview(&self, _conv: &Conventions, _cursor: Option<&Pick>, out: &mut Preview) {
        if let Some(base) = self.base {
            out.points.push(base.position);
        }
    }
}
