//! Point symbols and line styles in the UI: painters that match the viewport
//! (both draw [`survey_core::style`]'s primitives) and compact pickers.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use survey_core::style::{LineStyle, Prim, Symbol, outline};

use crate::CadTheme;

/// Stroke width for a symbol of `radius` (matches the viewport shader).
fn stroke_width(radius: f32) -> f32 {
    (radius * 0.17).max(1.0)
}

/// Draws `symbol` centred at `center` with the given radius (points).
pub fn paint_symbol(painter: &Painter, center: Pos2, radius: f32, color: Color32, symbol: Symbol) {
    let stroke = Stroke::new(stroke_width(radius), color);
    // Unit coordinates are y-up; the screen is y-down.
    let at = |p: [f32; 2]| center + vec2(p[0], -p[1]) * radius;
    for prim in symbol.prims() {
        match *prim {
            Prim::Circle { c, r, fill } => {
                if fill {
                    painter.circle_filled(at(c), r * radius, color);
                } else {
                    painter.circle_stroke(at(c), r * radius, stroke);
                }
            }
            Prim::Square { c, h, fill } => {
                let rect = Rect::from_center_size(at(c), Vec2::splat(2.0 * h * radius));
                if fill {
                    painter.rect_filled(rect, 0.0, color);
                } else {
                    painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Middle);
                }
            }
            Prim::Polygon { fill, .. } | Prim::Star { fill, .. } => {
                let pts: Vec<Pos2> = outline(prim).into_iter().map(at).collect();
                if fill {
                    // A fan from the centre fills stars (not convex) as well as polygons.
                    let c = match *prim {
                        Prim::Polygon { c, .. } | Prim::Star { c, .. } => at(c),
                        _ => center,
                    };
                    let mut mesh = Mesh::default();
                    mesh.colored_vertex(c, color);
                    for p in &pts {
                        mesh.colored_vertex(*p, color);
                    }
                    let n = pts.len() as u32;
                    for i in 0..n {
                        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
                    }
                    painter.add(Shape::mesh(mesh));
                } else {
                    painter.add(Shape::closed_line(pts, stroke));
                }
            }
            Prim::Segment { a, b } => {
                painter.line_segment([at(a), at(b)], stroke);
            }
        }
    }
}

/// Draws a horizontal sample of a line style across `rect`.
pub fn paint_line_sample(painter: &Painter, rect: Rect, color: Color32, width: f32, style: LineStyle) {
    let y = rect.center().y;
    let stroke = Stroke::new(width, color);
    let pattern = style.pattern();
    if pattern.is_empty() {
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
        return;
    }
    let unit = width.max(1.5);
    let mut x = rect.left();
    let mut i = 0;
    while x < rect.right() {
        let len = pattern[i % pattern.len()] * unit;
        if i % 2 == 0 {
            let end = (x + len).min(rect.right());
            painter.line_segment([pos2(x, y), pos2(end.max(x + 0.5), y)], stroke);
        }
        x += len;
        i += 1;
    }
}

/// A button showing the current symbol that opens a grid of all symbols.
pub fn symbol_picker(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut Symbol,
    color: Color32,
) -> Response {
    let theme = CadTheme::current(ui.ctx());
    let size = vec2(30.0, ui.spacing().interact_size.y);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() {
        theme.raised
    } else {
        theme.bg
    };
    ui.painter().rect_filled(rect, theme.radius, fill);
    ui.painter().rect_stroke(
        rect,
        theme.radius,
        Stroke::new(1.0, theme.border),
        egui::StrokeKind::Inside,
    );
    paint_symbol(ui.painter(), rect.center(), 6.0, color, *value);
    let response_hover = response.clone();
    response = response.on_hover_text(value.label());
    let popup_id = ui.make_persistent_id(id_salt);
    egui::Popup::menu(&response_hover)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_max_width(8.0 * 34.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for s in Symbol::ALL {
                    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
                    let selected = *s == *value;
                    if selected || resp.hovered() {
                        ui.painter().rect_filled(
                            r,
                            theme.radius,
                            if selected {
                                theme.accent_subtle
                            } else {
                                theme.raised
                            },
                        );
                    }
                    paint_symbol(ui.painter(), r.center(), 9.0, color, *s);
                    if resp.on_hover_text(s.label()).clicked() {
                        *value = *s;
                        response.mark_changed();
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
            });
        });
    response
}

/// A combo box of line styles drawn as samples.
pub fn line_style_picker(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut LineStyle,
    color: Color32,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id_salt)
        .width(96.0)
        .selected_text(value.label())
        .show_ui(ui, |ui| {
            for style in LineStyle::ALL {
                let (rect, resp) = ui.allocate_exact_size(vec2(110.0, 20.0), Sense::click());
                if style == *value || resp.hovered() {
                    let theme = CadTheme::current(ui.ctx());
                    ui.painter().rect_filled(rect, theme.radius, theme.raised);
                }
                paint_line_sample(ui.painter(), rect.shrink2(vec2(8.0, 0.0)), color, 1.5, style);
                if resp.on_hover_text(style.label()).clicked() {
                    *value = style;
                    changed = true;
                }
            }
        });
    changed
}
