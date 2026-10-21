//! Window / crossing box selection (AutoCAD convention).
//!
//! Dragging left-to-right makes a *window*: only objects entirely inside are
//! selected. Dragging right-to-left makes a *crossing*: anything the box touches
//! is selected.

use cad_render::Camera;
use egui::{Rect, Stroke, StrokeKind, pos2};
use glam::{DVec2, DVec3};

use crate::theme::CadTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxMode {
    Window,
    Crossing,
}

impl BoxMode {
    pub fn from_drag(start: DVec2, end: DVec2) -> Self {
        if end.x >= start.x {
            BoxMode::Window
        } else {
            BoxMode::Crossing
        }
    }
}

/// A selection box in viewport pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionBox {
    pub min: DVec2,
    pub max: DVec2,
    pub mode: BoxMode,
}

impl SelectionBox {
    pub fn from_drag(start: DVec2, end: DVec2) -> Self {
        Self {
            min: start.min(end),
            max: start.max(end),
            mode: BoxMode::from_drag(start, end),
        }
    }

    fn contains(&self, p: DVec2) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    /// Whether the screen segment `a–b` touches the box.
    fn touches_segment(&self, a: DVec2, b: DVec2) -> bool {
        if self.contains(a) || self.contains(b) {
            return true;
        }
        // Liang–Barsky clip of the segment against the box.
        let d = b - a;
        let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
        for (p, q) in [
            (-d.x, a.x - self.min.x),
            (d.x, self.max.x - a.x),
            (-d.y, a.y - self.min.y),
            (d.y, self.max.y - a.y),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }

    /// Whether an object made of `parts` (polylines in world space) is selected.
    /// Points behind a perspective camera never count as inside.
    pub fn selects(&self, cam: &Camera, parts: &[Vec<DVec3>]) -> bool {
        let project = |p: DVec3| cam.world_to_screen(p).map(|(s, _)| s);
        match self.mode {
            BoxMode::Window => parts
                .iter()
                .flatten()
                .all(|p| project(*p).is_some_and(|s| self.contains(s))),
            BoxMode::Crossing => parts.iter().any(|part| {
                if part.len() == 1 {
                    return project(part[0]).is_some_and(|s| self.contains(s));
                }
                part.windows(2).any(|w| match (project(w[0]), project(w[1])) {
                    (Some(a), Some(b)) => self.touches_segment(a, b),
                    _ => false,
                })
            }),
        }
    }

    /// Draws the box: solid blue for window, dashed green for crossing.
    pub fn draw(&self, painter: &egui::Painter, viewport: Rect, theme: &CadTheme) {
        let rect = Rect::from_min_max(
            pos2(
                viewport.min.x + self.min.x as f32,
                viewport.min.y + self.min.y as f32,
            ),
            pos2(
                viewport.min.x + self.max.x as f32,
                viewport.min.y + self.max.y as f32,
            ),
        );
        let color = match self.mode {
            BoxMode::Window => theme.preview,
            BoxMode::Crossing => theme.snap,
        };
        painter.rect_filled(rect, 0.0, color.gamma_multiply(0.12));
        match self.mode {
            BoxMode::Window => {
                painter.rect_stroke(rect, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
            }
            BoxMode::Crossing => {
                let pts = [
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                    rect.left_top(),
                ];
                painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, color), 5.0, 3.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_core::Aabb3;

    fn cam() -> Camera {
        let mut c = Camera {
            target: DVec3::new(500_000.0, 7_000_000.0, 0.0),
            distance: 20.0,
            viewport: DVec2::new(800.0, 600.0),
            ..Default::default()
        };
        c.fit_clip(&Aabb3::from_point(c.target).expanded(50.0));
        c
    }

    #[test]
    fn window_versus_crossing() {
        let cam = cam();
        let o = cam.target;
        let line = vec![vec![
            o + DVec3::new(-1.0, 0.0, 0.0),
            o + DVec3::new(1.0, 0.0, 0.0),
        ]];
        let (a, _) = cam
            .world_to_screen(o + DVec3::new(-0.5, -0.5, 0.0))
            .expect("visible");
        let (b, _) = cam
            .world_to_screen(o + DVec3::new(0.5, 0.5, 0.0))
            .expect("visible");
        // The box covers only the middle of the line.
        let window = SelectionBox::from_drag(a.min(b), a.max(b));
        assert_eq!(window.mode, BoxMode::Window);
        assert!(!window.selects(&cam, &line));
        let crossing = SelectionBox::from_drag(a.max(b), a.min(b));
        assert_eq!(crossing.mode, BoxMode::Crossing);
        assert!(crossing.selects(&cam, &line), "the line passes through the box");
        // A box around the whole line selects it either way.
        let (c, _) = cam
            .world_to_screen(o + DVec3::new(-2.0, -2.0, 0.0))
            .expect("visible");
        let (d, _) = cam
            .world_to_screen(o + DVec3::new(2.0, 2.0, 0.0))
            .expect("visible");
        assert!(SelectionBox::from_drag(c.min(d), c.max(d)).selects(&cam, &line));
        // A point is a single-element part.
        let point = vec![vec![o]];
        assert!(crossing.selects(&cam, &point) && window.selects(&cam, &point));
    }
}
