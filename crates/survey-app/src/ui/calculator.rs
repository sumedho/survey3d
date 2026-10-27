//! COGO calculator: circular curve (plain, with spirals, compound and
//! reverse) and triangle solutions from any sufficient subset of their
//! elements. Pure calculation; nothing is added to the drawing.

use std::fmt::Write as _;

use egui::RichText;
use egui_cad::chrome::segmented;
use egui_cad::{CadTheme, fonts};
use survey_core::solvers::{CurveInput, Triangle, TriangleInput, solve_curve, solve_triangle};

use crate::curves::{CurveRequest, curve_text, solve_request};
use survey_core::curve_sets::{CompoundInput, ReverseInput, SpiralCurveInput};
use survey_core::{Angle, Conventions};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CalcTab {
    Curve,
    Triangle,
}

/// Which curve problem the curve tab solves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CurveKind {
    Simple,
    Spiral,
    Compound,
    Reverse,
}

pub struct CalculatorWindow {
    pub open: bool,
    pub tab: CalcTab,
    /// Radius, Δ, arc length, chord, tangent, external, mid-ordinate.
    curve: [String; 7],
    /// a, b, c, α, β, γ.
    triangle: [String; 6],
    solution: usize,
    kind: CurveKind,
    /// Radius, spiral length, Δ, tangent, total length.
    spiral: [String; 5],
    /// R1, R2, Δ1, Δ2, Δ, T1, T2.
    compound: [String; 7],
    /// Equal radius, R1, R2, Δ, offset, along.
    reverse: [String; 6],
}

impl Default for CalculatorWindow {
    fn default() -> Self {
        Self {
            open: false,
            tab: CalcTab::Curve,
            curve: Default::default(),
            triangle: Default::default(),
            solution: 0,
            kind: CurveKind::Simple,
            spiral: Default::default(),
            compound: Default::default(),
            reverse: Default::default(),
        }
    }
}

const CURVE_LABELS: [&str; 7] = [
    "Radius",
    "Delta (central angle)",
    "Arc length",
    "Chord",
    "Tangent",
    "External",
    "Mid-ordinate",
];
const TRIANGLE_LABELS: [&str; 6] = ["Side a", "Side b", "Side c", "Angle A", "Angle B", "Angle C"];

fn parse(conv: &Conventions, label: &str, text: &str, angle: bool) -> Result<Option<f64>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let v = if angle {
        conv.parse_angle(t).map(|a| a.radians())
    } else {
        conv.parse_length(t)
    };
    v.map(Some).map_err(|e| format!("{label}: {e}"))
}

impl CalculatorWindow {
    pub fn open_at(&mut self, tab: CalcTab) {
        self.open = true;
        self.tab = tab;
    }

    /// Shows the window; returns a (title, text) report when one is requested.
    pub fn show(&mut self, ctx: &egui::Context, conv: &Conventions) -> Option<(String, String)> {
        if !self.open {
            return None;
        }
        let theme = CadTheme::current(ctx);
        let mut report = None;
        let mut open = self.open;
        egui::Window::new("COGO calculator")
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if segmented(
                        ui,
                        &mut self.tab,
                        &[(CalcTab::Curve, "Curve"), (CalcTab::Triangle, "Triangle")],
                    ) {
                        self.solution = 0;
                    }
                    if ui.button("Clear").clicked() {
                        self.curve = Default::default();
                        self.triangle = Default::default();
                        self.spiral = Default::default();
                        self.compound = Default::default();
                        self.reverse = Default::default();
                        self.solution = 0;
                    }
                });
                ui.add_space(6.0);
                match self.tab {
                    CalcTab::Curve => report = self.curve_ui(ui, &theme, conv),
                    CalcTab::Triangle => report = self.triangle_ui(ui, &theme, conv),
                }
            });
        self.open = open;
        report
    }

    fn inputs(ui: &mut egui::Ui, id: &str, labels: &[&str], values: &mut [String], hint: &str) {
        egui::Grid::new(id)
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                for (label, value) in labels.iter().zip(values.iter_mut()) {
                    ui.label(*label);
                    ui.add_sized(
                        [160.0, 22.0],
                        egui::TextEdit::singleline(value)
                            .hint_text(hint)
                            .font(fonts::mono(12.0)),
                    );
                    ui.end_row();
                }
            });
    }

    fn pick_solution(&mut self, ui: &mut egui::Ui, theme: &CadTheme, count: usize) -> usize {
        if count > 1 {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{count} solutions")).color(theme.text_secondary));
                self.solution = self.solution.min(count - 1);
                let labels: Vec<String> = (1..=count).map(|i| i.to_string()).collect();
                let options: Vec<(usize, &str)> =
                    labels.iter().enumerate().map(|(i, l)| (i, l.as_str())).collect();
                segmented(ui, &mut self.solution, &options);
            });
        }
        self.solution.min(count - 1)
    }

    fn curve_ui(
        &mut self,
        ui: &mut egui::Ui,
        theme: &CadTheme,
        conv: &Conventions,
    ) -> Option<(String, String)> {
        if segmented(
            ui,
            &mut self.kind,
            &[
                (CurveKind::Simple, "Circular"),
                (CurveKind::Spiral, "With spirals"),
                (CurveKind::Compound, "Compound"),
                (CurveKind::Reverse, "Reverse"),
            ],
        ) {
            self.solution = 0;
        }
        ui.add_space(4.0);
        if self.kind != CurveKind::Simple {
            return self.curve_set_ui(ui, theme, conv);
        }
        ui.label(
            RichText::new("Enter any two elements.")
                .small()
                .color(theme.text_secondary),
        );
        Self::inputs(ui, "calc_curve", &CURVE_LABELS, &mut self.curve, "");
        ui.separator();
        let input = (|| {
            let v: Vec<Option<f64>> = self
                .curve
                .iter()
                .zip(CURVE_LABELS)
                .enumerate()
                .map(|(i, (t, l))| parse(conv, l, t, i == 1))
                .collect::<Result<_, _>>()?;
            Ok::<_, String>(CurveInput {
                radius: v[0],
                delta: v[1],
                length: v[2],
                chord: v[3],
                tangent: v[4],
                external: v[5],
                mid_ordinate: v[6],
            })
        })();
        match input.and_then(|i| solve_curve(i).map_err(|e| e.to_string())) {
            Ok(sol) => {
                let c = sol[self.pick_solution(ui, theme, sol.len())];
                let text = curve_text(conv, &c);
                ui.label(RichText::new(&text).font(fonts::mono(12.0)));
                if ui.button("Add to reports").clicked() {
                    return Some(("Curve solution".into(), text));
                }
            }
            Err(e) => {
                ui.label(RichText::new(e).color(theme.text_secondary));
            }
        }
        None
    }

    /// Spiral, compound and reverse curves.
    fn curve_set_ui(
        &mut self,
        ui: &mut egui::Ui,
        theme: &CadTheme,
        conv: &Conventions,
    ) -> Option<(String, String)> {
        let (help, labels, angles, values): (&str, &[&str], &[usize], &mut [String]) = match self.kind {
            CurveKind::Spiral => (
                "Equal spirals each side: the radius and spiral length, and one of Δ, the tangent (PI to TS) or the total length.",
                &[
                    "Radius",
                    "Spiral length Ls",
                    "Delta (total)",
                    "Tangent Ts",
                    "Total length",
                ],
                &[2],
                &mut self.spiral,
            ),
            CurveKind::Compound => (
                "Two arcs turning the same way: both radii, and two of Δ1, Δ2, Δ, T1 (PC to PI) and T2 (PI to PT), one of them an angle.",
                &[
                    "Radius R1",
                    "Radius R2",
                    "Delta 1",
                    "Delta 2",
                    "Delta (total)",
                    "T1 (PC–PI)",
                    "T2 (PI–PT)",
                ],
                &[2, 3, 4],
                &mut self.compound,
            ),
            _ => (
                "Two arcs turning opposite ways between parallel tangents: the radius (or R1 and R2) and Δ, the offset or the distance along; or the offset and the distance along.",
                &[
                    "Radius (both)",
                    "Radius R1",
                    "Radius R2",
                    "Delta (each)",
                    "Offset",
                    "Along",
                ],
                &[3],
                &mut self.reverse,
            ),
        };
        ui.label(RichText::new(help).small().color(theme.text_secondary));
        Self::inputs(ui, "calc_curve_set", labels, values, "");
        ui.separator();
        let v: Result<Vec<Option<f64>>, String> = values
            .iter()
            .zip(labels)
            .enumerate()
            .map(|(i, (t, l))| parse(conv, l, t, angles.contains(&i)))
            .collect();
        let request = v.map(|v| match self.kind {
            CurveKind::Spiral => CurveRequest::Spiral(SpiralCurveInput {
                radius: v[0],
                spiral_length: v[1],
                delta: v[2],
                tangent: v[3],
                total_length: v[4],
            }),
            CurveKind::Compound => CurveRequest::Compound(CompoundInput {
                r1: v[0],
                r2: v[1],
                delta1: v[2],
                delta2: v[3],
                delta: v[4],
                t1: v[5],
                t2: v[6],
            }),
            _ => CurveRequest::Reverse(ReverseInput {
                radius: v[0],
                r1: v[1],
                r2: v[2],
                delta: v[3],
                offset: v[4],
                along: v[5],
            }),
        });
        match request.and_then(|r| solve_request(conv, r)) {
            Ok(sol) => {
                let text = sol[self.pick_solution(ui, theme, sol.len())].clone();
                ui.label(RichText::new(&text).font(fonts::mono(12.0)));
                if ui.button("Add to reports").clicked() {
                    return Some(("Curve solution".into(), text));
                }
            }
            Err(e) => {
                ui.label(RichText::new(e).color(theme.text_secondary));
            }
        }
        None
    }

    fn triangle_ui(
        &mut self,
        ui: &mut egui::Ui,
        theme: &CadTheme,
        conv: &Conventions,
    ) -> Option<(String, String)> {
        ui.label(
            RichText::new("Enter any three elements including a side. Side a is opposite angle A.")
                .small()
                .color(theme.text_secondary),
        );
        Self::inputs(ui, "calc_triangle", &TRIANGLE_LABELS, &mut self.triangle, "");
        ui.separator();
        let input = (|| {
            let v: Vec<Option<f64>> = self
                .triangle
                .iter()
                .zip(TRIANGLE_LABELS)
                .enumerate()
                .map(|(i, (t, l))| parse(conv, l, t, i >= 3))
                .collect::<Result<_, _>>()?;
            Ok::<_, String>(TriangleInput {
                a: v[0],
                b: v[1],
                c: v[2],
                alpha: v[3],
                beta: v[4],
                gamma: v[5],
            })
        })();
        match input.and_then(|i| solve_triangle(i).map_err(|e| e.to_string())) {
            Ok(sol) => {
                let t = sol[self.pick_solution(ui, theme, sol.len())];
                let text = triangle_text(conv, &t);
                ui.label(RichText::new(&text).font(fonts::mono(12.0)));
                if ui.button("Add to reports").clicked() {
                    return Some(("Triangle solution".into(), text));
                }
            }
            Err(e) => {
                ui.label(RichText::new(e).color(theme.text_secondary));
            }
        }
        None
    }
}

fn triangle_text(conv: &Conventions, t: &Triangle) -> String {
    let len = |v: f64| conv.format_length_with_unit(v);
    let ang = |v: f64| conv.format_angle(Angle::from_radians(v));
    let mut s = String::new();
    let _ = writeln!(s, "a  {:>14}    A  {}", len(t.a), ang(t.alpha));
    let _ = writeln!(s, "b  {:>14}    B  {}", len(t.b), ang(t.beta));
    let _ = writeln!(s, "c  {:>14}    C  {}", len(t.c), ang(t.gamma));
    let _ = write!(s, "Area  {}", conv.format_area(t.area()));
    s
}
