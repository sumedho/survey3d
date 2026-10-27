//! Survey computations shared by the windows and by agents (the MCP server):
//! each builds its inputs from the document, computes with survey-core, and
//! returns the result with a plain-text report and, where it draws or moves
//! things, the undoable change.

pub mod closure;
pub mod network;
pub mod projection;
pub mod resection;
pub mod sections;
pub mod setout;
pub mod subdivide;
pub mod traverse;
pub mod volumes;
