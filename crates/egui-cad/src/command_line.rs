//! AutoCAD-style command line: a scrolling log, a prompt and an input field
//! with history (Up/Down) and Tab completion.
//!
//! Typing while nothing else has keyboard focus goes to the command line, so
//! users can type commands or coordinates without clicking first.

use std::collections::VecDeque;

use egui::{Key, RichText, TextEdit};

use crate::fonts;
use crate::icons;
use crate::theme::CadTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Info,
    Command,
    Result,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

/// Persistent command-line state.
#[derive(Default)]
pub struct CommandLineState {
    pub input: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    log: VecDeque<LogLine>,
    focus_requested: bool,
    cursor_to_end: bool,
    /// Show a taller history (toggled by the chevron next to the input).
    expanded: bool,
}

const MAX_LOG: usize = 500;

impl CommandLineState {
    pub fn log(&mut self, kind: LogKind, text: impl Into<String>) {
        for line in text.into().lines() {
            self.log.push_back(LogLine {
                kind,
                text: line.to_string(),
            });
        }
        while self.log.len() > MAX_LOG {
            self.log.pop_front();
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.log(LogKind::Info, text);
    }

    pub fn result(&mut self, text: impl Into<String>) {
        self.log(LogKind::Result, text);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.log(LogKind::Warning, text);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.log(LogKind::Error, text);
    }

    /// Takes text typed before the input field received focus (e.g. when
    /// characters and Enter arrive in the same frame).
    pub fn take_pending_input(&mut self) -> Option<String> {
        let text = std::mem::take(&mut self.input);
        (!text.trim().is_empty()).then_some(text)
    }

    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    pub fn lines(&self) -> impl Iterator<Item = &LogLine> {
        self.log.iter()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandLineEvent {
    /// Enter was pressed. An empty string means "accept / finish".
    Submit(String),
    /// Escape was pressed while the command line had focus.
    Cancel,
}

/// A completion candidate shown while typing.
#[derive(Clone, Debug)]
pub struct Completion {
    pub name: String,
    pub description: String,
}

pub struct CommandLine<'a> {
    state: &'a mut CommandLineState,
    prompt: &'a str,
    completions: &'a [Completion],
    log_rows: usize,
}

impl<'a> CommandLine<'a> {
    pub fn new(state: &'a mut CommandLineState, prompt: &'a str) -> Self {
        Self {
            state,
            prompt,
            completions: &[],
            log_rows: 4,
        }
    }

    /// Names offered for Tab completion when the input is a prefix of them.
    pub fn completions(mut self, completions: &'a [Completion]) -> Self {
        self.completions = completions;
        self
    }

    pub fn log_rows(mut self, rows: usize) -> Self {
        self.log_rows = rows;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> Option<CommandLineEvent> {
        let theme = CadTheme::current(ui.ctx());
        let state = self.state;
        let id = ui.scope_id().with("egui_cad_command_line");

        // Log.
        let row_h = 11.5 + 5.0;
        let rows = if state.expanded {
            self.log_rows * 3
        } else {
            self.log_rows
        };
        egui::ScrollArea::vertical()
            .id_salt(id.with("log"))
            .max_height(row_h * rows as f32)
            .min_scrolled_height(row_h * rows as f32)
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for line in &state.log {
                    let color = match line.kind {
                        LogKind::Info => theme.text_secondary,
                        LogKind::Command => theme.accent,
                        LogKind::Result => theme.text,
                        LogKind::Warning => theme.warning,
                        LogKind::Error => theme.error,
                    };
                    ui.label(RichText::new(&line.text).font(fonts::mono(11.5)).color(color));
                }
            });

        // Route stray typing to the command line.
        let nothing_focused = ui.ctx().memory(|m| m.focused().is_none());
        if nothing_focused {
            let typed: String = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            if !typed.is_empty() {
                state.input.push_str(&typed);
                state.focus_requested = true;
                state.cursor_to_end = true;
            }
        }

        let input = state.input.trim().to_ascii_lowercase();
        let matches: Vec<&Completion> = if input.is_empty() || input.contains([',', ' ', '@']) {
            Vec::new()
        } else {
            self.completions
                .iter()
                .filter(|c| c.name.starts_with(&input))
                .take(8)
                .collect()
        };

        let mut event = None;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.prompt)
                    .font(fonts::medium(12.5))
                    .color(theme.accent),
            );
            let text_id = id.with("input");
            let has_focus = ui.ctx().memory(|m| m.has_focus(text_id));
            // Handle keys before the TextEdit consumes them.
            if has_focus {
                let (up, down, tab, esc) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, Key::Tab),
                        i.key_pressed(Key::Escape),
                    )
                });
                if up && !state.history.is_empty() {
                    let pos = state
                        .history_pos
                        .map_or(state.history.len() - 1, |p| p.saturating_sub(1));
                    state.history_pos = Some(pos);
                    state.input = state.history[pos].clone();
                    state.cursor_to_end = true;
                }
                if down && let Some(p) = state.history_pos {
                    if p + 1 < state.history.len() {
                        state.history_pos = Some(p + 1);
                        state.input = state.history[p + 1].clone();
                        state.cursor_to_end = true;
                    } else {
                        state.history_pos = None;
                        state.input.clear();
                    }
                }
                if tab && let Some(first) = matches.first() {
                    state.input = first.name.clone();
                    state.cursor_to_end = true;
                }
                if esc {
                    state.input.clear();
                    event = Some(CommandLineEvent::Cancel);
                }
            }
            let edit = TextEdit::singleline(&mut state.input)
                .id(text_id)
                .font(egui::TextStyle::Monospace)
                .hint_text("Type a command or coordinates — Ctrl/Cmd+K to search")
                .desired_width(ui.available_width() - 26.0)
                .show(ui);
            let chevron = if state.expanded {
                icons::CARET_DOWN
            } else {
                crate::icons::CARET_RIGHT
            };
            if crate::chrome::IconButton::new(
                chevron,
                if state.expanded {
                    "Shorter history"
                } else {
                    "Longer history"
                },
            )
            .size(20.0)
            .show(ui)
            .clicked()
            {
                state.expanded = !state.expanded;
            }
            if state.focus_requested {
                edit.response.request_focus();
                state.focus_requested = false;
            }
            if std::mem::take(&mut state.cursor_to_end) {
                // Text was added programmatically: keep typing at the end, not the start.
                let mut st = egui::text_edit::TextEditState::load(ui.ctx(), text_id).unwrap_or_default();
                let end = egui::text::CCursor::new(state.input.chars().count());
                st.cursor
                    .set_char_range(Some(egui::text_selection::CCursorRange::one(end)));
                st.store(ui.ctx(), text_id);
            }
            if edit.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let text = std::mem::take(&mut state.input);
                if !text.trim().is_empty() && state.history.last() != Some(&text) {
                    state.history.push(text.clone());
                }
                state.history_pos = None;
                event = Some(CommandLineEvent::Submit(text));
                // Keep focus for the next entry.
                edit.response.request_focus();
            }
        });
        if !matches.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Tab").small().color(theme.text_secondary));
                for m in matches {
                    ui.label(RichText::new(&m.name).small().monospace().color(theme.text))
                        .on_hover_text(&m.description);
                }
            });
        }
        event
    }
}
