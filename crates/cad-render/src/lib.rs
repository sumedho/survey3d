//! wgpu renderer for CAD viewports embedded in egui.
//!
//! * [`camera`] — `f64` camera maths, relative-to-center (RTC) matrices, picking rays.
//! * [`scene`] — CPU-side scene description, chunked by spatial origin.
//! * [`renderer`] — GPU resources and the `egui_wgpu` paint callback.
//!
//! The 3D scene is drawn into an offscreen multisampled colour + depth target
//! during `prepare`, then composited into egui's render pass during `paint`.

pub mod camera;
pub mod renderer;
pub mod scene;

/// Re-exported so widgets can build paint callbacks against the same version.
pub use egui_wgpu;

pub use camera::{Camera, Frustum, Projection, Ray};
pub use renderer::{GridSettings, ViewportCallback, ViewportFrame, install};
pub use scene::{PointShape, Scene, SceneBuilder};
