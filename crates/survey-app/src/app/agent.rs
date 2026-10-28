//! The agent link in the window: started and stopped from the settings,
//! answering agents' requests on the session on screen between frames, and
//! a status-bar chip showing it is on and what the agent last did.

use std::sync::Arc;

use egui::RichText;
use egui_cad::CadTheme;
use web_time::Instant;

use super::SurveyApp;
use crate::agent::Server;
use crate::agent::link::{self, Link, LinkInfo, Served};

/// The link's state in the app.
#[derive(Default)]
pub struct AgentState {
    link: Option<Link>,
    /// Lends the window's session to each request.
    server: Option<Server>,
    /// Why the link could not start (for the port it was asked for).
    error: Option<(u16, String)>,
    /// The last thing an agent did, and when.
    last: Option<(Served, Instant)>,
    calls: u64,
}

impl AgentState {
    /// The link's address while it is on.
    pub fn url(&self) -> Option<String> {
        self.link.as_ref().map(Link::url)
    }

    pub fn token(&self) -> Option<&str> {
        self.link.as_ref().map(|l| l.token.as_str())
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_ref().map(|(_, e)| e.as_str())
    }
}

impl SurveyApp {
    /// Starts or stops the link to match the settings, then answers what
    /// agents have asked since the last frame.
    pub(super) fn run_agent(&mut self, ctx: &egui::Context) {
        let (on, port) = (self.settings.agent_link, self.settings.agent_port);
        let running = self.agent.link.as_ref().map(|l| l.port);
        if (!on || running.is_some_and(|p| p != port)) && self.agent.link.take().is_some() {
            self.session.log.info("Agent link off");
        }
        if on && self.agent.link.is_none() && self.agent.error.as_ref().is_none_or(|(p, _)| *p != port) {
            let wake = {
                let ctx = ctx.clone();
                Arc::new(move || ctx.request_repaint()) as Arc<dyn Fn() + Send + Sync>
            };
            match Link::start(port, link::token(), wake) {
                Ok(l) => {
                    let info = LinkInfo {
                        port: l.port,
                        token: l.token.clone(),
                        url: l.url(),
                    };
                    match link::write_info(&info) {
                        Ok(path) => self.session.log.info(format!(
                            "Agent link on at {} (connection details in {})",
                            info.url,
                            path.display()
                        )),
                        Err(e) => self.session.log.warn(format!(
                            "Agent link on at {}, but its details could not be saved for survey3d-mcp --attach: {e}",
                            info.url
                        )),
                    }
                    self.agent.error = None;
                    self.agent.link = Some(l);
                }
                Err(e) => {
                    let msg = format!(
                        "Agent link could not listen on port {port}: {e} (choose another port in Settings)"
                    );
                    self.session.log.error(msg.clone());
                    self.toasts.error(msg.clone());
                    self.agent.error = Some((port, msg));
                }
            }
        }
        if !on {
            self.agent.error = None;
        }
        let Some(l) = &self.agent.link else {
            return;
        };
        let requests: Vec<link::Request> = l.requests().collect();
        for req in requests {
            if self.agent.server.is_none() {
                match Server::for_app(&self.settings) {
                    Ok(s) => self.agent.server = Some(s),
                    Err(e) => {
                        self.session.log.error(format!("Agent link: {e}"));
                        req.answer(None);
                        continue;
                    }
                }
            }
            let Some(server) = self.agent.server.as_mut() else {
                continue;
            };
            let message = req.message.clone();
            // The audit trail records the agent's edits as the user's, via the agent.
            let user = self.settings.user_name.clone();
            self.session
                .project
                .history
                .set_user(format!("{user} (agent via MCP)"));
            let (reply, served) = link::serve_on(&mut self.session, server, message);
            self.session.project.history.set_user(user);
            req.answer(reply);
            if served.replaced_project {
                self.reset_views();
            }
            if served.tool.is_some() {
                self.agent.calls += 1;
                self.agent.last = Some((served, Instant::now()));
            }
            ctx.request_repaint();
        }
        // Fade the chip's highlight.
        if self
            .agent
            .last
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed().as_secs_f32() < 3.0)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    /// The status-bar chip: hidden while the link is off.
    pub(super) fn agent_chip(&mut self, ui: &mut egui::Ui, theme: &CadTheme) {
        if !self.settings.agent_link {
            return;
        }
        let (text, color, tip) = match (&self.agent.link, &self.agent.error) {
            (Some(l), _) => {
                let recent = self
                    .agent
                    .last
                    .as_ref()
                    .filter(|(_, t)| t.elapsed().as_secs_f32() < 3.0);
                let tip = match &self.agent.last {
                    Some((s, t)) => format!(
                        "Agents can drive this window at {}\n{} calls; last {} {:.0} s ago: {}",
                        l.url(),
                        self.agent.calls,
                        s.tool.as_deref().unwrap_or("message"),
                        t.elapsed().as_secs_f32(),
                        s.summary
                    ),
                    None => format!("Agents can drive this window at {}\nNo calls yet", l.url()),
                };
                match recent {
                    Some((s, _)) => (
                        format!("● Agent: {}", s.tool.as_deref().unwrap_or("…")),
                        if s.error { theme.warning } else { theme.accent },
                        tip,
                    ),
                    None => ("● Agent link".to_string(), theme.text_secondary, tip),
                }
            }
            (None, Some((_, e))) => ("● Agent link: not listening".to_string(), theme.error, e.clone()),
            (None, None) => (
                "● Agent link: starting".to_string(),
                theme.text_secondary,
                String::new(),
            ),
        };
        let r = ui
            .add(egui::Label::new(RichText::new(text).small().color(color)).sense(egui::Sense::click()))
            .on_hover_text(format!("{tip}\nClick for the agent link settings"));
        if r.clicked() {
            self.settings_window.open_at_general();
        }
    }
}
