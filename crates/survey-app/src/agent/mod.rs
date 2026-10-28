//! The agent interface: the Model Context Protocol server through which an
//! agent opens, edits, queries and renders projects. The headless
//! `survey3d-mcp` binary serves it on stdio over a session of its own; the
//! desktop app serves it on a local port over the session on screen (see
//! [`link`]), so a chat and the drawing share one project.

mod base64;
pub mod link;
pub mod protocol;
mod render;
pub mod server;
mod tools;

pub use server::{Options, Server};
