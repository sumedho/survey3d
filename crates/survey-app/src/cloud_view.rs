//! Drawing point clouds: one level-of-detail scene for all visible clouds,
//! rebuilt only when a cloud (or the theme) changes. On native the rebuild runs
//! on a background thread (millions of points take a while to bin), and the
//! previous scene keeps drawing until the new one is ready.

use std::sync::Arc;

use cad_render::{Scene, SceneBuilder};
use egui_cad::{CadTheme, ThemeMode};
use survey_core::style::Symbol;
use survey_model::{CloudColor, Document, PointCloud};

use crate::surface_view::ramp;

/// Target points per chunk: small enough for fine level of detail, large
/// enough to keep draw calls few.
const POINTS_PER_CHUNK: f64 = 20_000.0;

#[derive(Default)]
pub struct CloudSceneCache {
    key: Option<(u64, ThemeMode)>,
    revision: u64,
    scene: Option<Arc<Scene>>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<std::sync::mpsc::Receiver<Option<Arc<Scene>>>>,
}

impl CloudSceneCache {
    pub fn get(&mut self, doc: &Document, theme: &CadTheme, ctx: &egui::Context) -> Option<Arc<Scene>> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(scene) => {
                    self.scene = scene;
                    self.pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return self.scene.clone(),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pending = None,
            }
        }
        let key = (doc.clouds_revision(), theme.mode);
        if self.key != Some(key) {
            self.key = Some(key);
            self.revision += 1;
            let clouds: Vec<PointCloud> = doc
                .clouds()
                .map(|(_, c)| c)
                // Out-of-core clouds are drawn node by node (see `cloud_stream`).
                .filter(|c| c.style.visible && !c.is_empty() && !c.is_out_of_core())
                .cloned()
                .collect();
            let revision = self.revision;
            #[cfg(not(target_arch = "wasm32"))]
            {
                let (tx, rx) = std::sync::mpsc::channel();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(build(&clouds, revision));
                    ctx.request_repaint();
                });
                self.pending = Some(rx);
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = ctx;
                self.scene = build(&clouds, revision);
            }
        }
        self.scene.clone()
    }

    pub fn invalidate(&mut self) {
        self.key = None;
        self.scene = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pending = None;
        }
    }
}

fn build(clouds: &[PointCloud], revision: u64) -> Option<Arc<Scene>> {
    if clouds.is_empty() {
        return None;
    }
    // Chunk size from the densest cloud's point spacing.
    let chunk = clouds
        .iter()
        .map(|c| {
            let b = c.bounds();
            let area = ((b.max.x - b.min.x) * (b.max.y - b.min.y)).max(1.0);
            (area * POINTS_PER_CHUNK / c.len() as f64).sqrt()
        })
        .fold(f64::MAX, f64::min)
        .clamp(2.0, 2048.0);
    let mut b = SceneBuilder::new(chunk);
    for c in clouds {
        let bounds = c.bounds();
        let (zmin, span) = (bounds.min.z, (bounds.max.z - bounds.min.z).max(1e-9));
        let mode = match (c.style.color, &c.colors) {
            (CloudColor::Rgb, None) => CloudColor::Height,
            (m, _) => m,
        };
        let single = c.style.single.0;
        for i in 0..c.len() {
            let p = c.point(i);
            let color = match mode {
                CloudColor::Rgb => {
                    let [r, g, bl] = c.colors.as_ref().map_or([200; 3], |cs| cs[i]);
                    [r, g, bl, 255]
                }
                CloudColor::Height => {
                    let [r, g, bl] = ramp((p.z - zmin) / span);
                    [r as u8, g as u8, bl as u8, 255]
                }
                CloudColor::Single => single,
            };
            b.add_point(p, color, c.style.point_size, Symbol::Square);
        }
    }
    // Revisions offset from the drawing and surface scenes'.
    Some(Arc::new(b.build_lod(revision | (1 << 61))))
}
