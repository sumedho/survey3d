//! View-dependent streaming of out-of-core (COPC) point clouds, as Potree
//! does it.
//!
//! Each frame the app offers the streamer the view for every on-disk cloud.
//! When the camera has moved enough, a background thread chooses the octree
//! nodes for the view ([`CopcSource::select`]: the coarse base, then nodes in
//! the frustum where points are furthest apart on screen first, each drawn in
//! the share the view needs, within a point budget) and hands the
//! selection back at once with the nodes already decoded, then again once the
//! rest are decoded on every core. The renderer draws each node from its own
//! GPU buffer, so nothing is merged or re-uploaded as the view moves.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use cad_render::{CloudColoring, CloudFrame, CloudLayer, CloudNode};
use glam::DVec3;
use survey_io::copc::{CopcSource, NodeKey, ViewQuery};
use survey_model::{CloudColor, Document, PointCloudId};

/// Points drawn for one cloud's view (Potree's budgets are 1–10 million).
const BUDGET: usize = 3_000_000;
/// Wanted distance between neighbouring points on screen (logical px); the
/// renderer grows points to their spacing, so this only sets the detail.
pub const SPACING_PX: f64 = 2.0;
/// Eye-dome lighting strength when it is on: enough to outline shapes without
/// edging every point.
const EDL_STRENGTH: f32 = 0.4;
/// Least time between successive selections for one cloud. Selection is
/// cheap and decoded nodes are cached, so this only damps continuous motion.
const MIN_INTERVAL: Duration = Duration::from_millis(50);

/// The camera as far as re-selecting goes.
#[derive(Clone, Copy)]
pub struct ViewKey {
    pub target: DVec3,
    pub eye: DVec3,
    /// World size of the view at the target (its diagonal).
    pub span: f64,
    /// World units per pixel at the target.
    pub resolution: f64,
}

impl ViewKey {
    /// Whether the view changed enough since `prev` to be worth re-selecting.
    /// The frustum has a margin around the view (see
    /// `SurveyApp::stream_clouds`), so a pan or orbit of about half that margin
    /// re-selects before blank edges show.
    fn differs_from(&self, prev: &ViewKey) -> bool {
        let reach = prev.span.max(1e-3) * 0.12;
        let moved = (self.target - prev.target).length() > reach;
        let orbited = (self.eye - prev.eye).length() > reach;
        let scaled = ratio(self.resolution, prev.resolution) > 1.2;
        moved || orbited || scaled
    }
}

fn ratio(a: f64, b: f64) -> f64 {
    let (a, b) = (a.max(1e-9), b.max(1e-9));
    (a / b).max(b / a)
}

/// A selection from a background thread.
struct Streamed {
    id: PointCloudId,
    /// `None` when reading failed: keep the current selection.
    nodes: Option<Vec<CloudNode>>,
    /// Every node is decoded (the thread is finished).
    done: bool,
}

pub struct CloudStreamer {
    tx: Sender<Streamed>,
    rx: Receiver<Streamed>,
    in_flight: HashSet<PointCloudId>,
    last: HashMap<PointCloudId, (ViewKey, Instant)>,
    /// The latest selection for each cloud.
    selections: HashMap<PointCloudId, Vec<CloudNode>>,
    /// Bumped when a selection changes.
    revision: u64,
    /// The frame last built, and what it was built from.
    frame: Option<((u64, u64, bool), Arc<CloudFrame>)>,
}

impl Default for CloudStreamer {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            in_flight: HashSet::new(),
            last: HashMap::new(),
            selections: HashMap::new(),
            revision: 0,
            frame: None,
        }
    }
}

impl CloudStreamer {
    /// Takes in finished selections. Returns whether any arrived.
    pub fn drain(&mut self) -> bool {
        let mut changed = false;
        while let Ok(s) = self.rx.try_recv() {
            if s.done || s.nodes.is_none() {
                self.in_flight.remove(&s.id);
            }
            if let Some(nodes) = s.nodes {
                self.selections.insert(s.id, nodes);
                self.revision += 1;
                changed = true;
            }
        }
        changed
    }

    /// Whether a selection is still being read for any cloud (the app keeps
    /// drawing frames until streaming settles).
    pub fn is_busy(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// Requests a selection for `id`'s view, unless one is already being read
    /// or the camera (`key`) has barely changed. `path` is the resolved COPC
    /// file.
    pub fn request(&mut self, id: PointCloudId, path: PathBuf, view: ViewQuery, key: ViewKey) {
        if self.in_flight.contains(&id) {
            return;
        }
        let now = Instant::now();
        if let Some((prev, at)) = self.last.get(&id)
            && (now.duration_since(*at) < MIN_INTERVAL || !key.differs_from(prev))
        {
            return;
        }
        self.last.insert(id, (key, now));
        self.in_flight.insert(id);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let Ok(src) = CopcSource::shared(&path) else {
                let _ = tx.send(Streamed {
                    id,
                    nodes: None,
                    done: true,
                });
                return;
            };
            let selection = src.select(&view, BUDGET);
            let mut file = DefaultHasher::new();
            path.hash(&mut file);
            let file = file.finish();
            let empty: Arc<Vec<[f32; 4]>> = Arc::new(Vec::new());
            let nodes = |src: &CopcSource| -> Vec<CloudNode> {
                selection
                    .iter()
                    .map(|s| CloudNode {
                        id: node_id(file, s.key),
                        center: s.center,
                        side: s.side,
                        spacing: s.spacing as f32,
                        parent: s.parent.map(|p| p as u32),
                        octant: s.octant,
                        points: if s.count == 0 {
                            Some(empty.clone())
                        } else {
                            src.cached(s.key).map(|n| n.points.clone())
                        },
                        cells: src.cached(s.key).map(|n| n.cells.clone()),
                        fraction: s.fraction,
                    })
                    .collect()
            };
            // What is decoded already draws now; the rest follows.
            let first = nodes(&src);
            let complete = first.iter().all(|n| n.points.is_some());
            let _ = tx.send(Streamed {
                id,
                nodes: Some(first),
                done: complete,
            });
            if complete {
                return;
            }
            let keys: Vec<NodeKey> = selection.iter().map(|s| s.key).collect();
            let loaded = src.load(&keys).is_ok();
            let _ = tx.send(Streamed {
                id,
                nodes: loaded.then(|| nodes(&src)),
                done: true,
            });
        });
    }

    /// Every loaded point of the current selections, in world coordinates
    /// (the detail on screen, for picking).
    pub fn for_each_point(&self, mut f: impl FnMut(DVec3)) {
        for node in self.selections.values().flatten() {
            if let Some(points) = &node.points {
                for p in points.iter() {
                    f(node.center + DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])));
                }
            }
        }
    }

    /// The renderer's view of the visible out-of-core clouds, rebuilt only
    /// when a selection, a cloud or the lighting changes.
    pub fn frame(&mut self, doc: &Document, edl: bool) -> Option<Arc<CloudFrame>> {
        let key = (self.revision, doc.clouds_revision(), edl);
        if let Some((k, f)) = &self.frame
            && *k == key
        {
            return Some(f.clone());
        }
        let layers: Vec<CloudLayer> = doc
            .clouds()
            .filter(|(_, c)| c.style.visible)
            .filter_map(|(id, c)| {
                let source = c.source.as_ref()?;
                let nodes = self.selections.get(&id)?.clone();
                let b = source.full_bounds;
                let coloring = match c.style.color {
                    CloudColor::Rgb if source.has_color => CloudColoring::Source,
                    CloudColor::Rgb | CloudColor::Height => CloudColoring::Height {
                        min: b.min.z,
                        max: b.max.z,
                    },
                    CloudColor::Single => CloudColoring::Single(c.style.single.0),
                };
                Some(CloudLayer {
                    nodes,
                    coloring,
                    min_size_px: c.style.point_size,
                })
            })
            .collect();
        let frame = Arc::new(CloudFrame {
            layers,
            edl: if edl { EDL_STRENGTH } else { 0.0 },
        });
        self.frame = Some((key, frame.clone()));
        Some(frame)
    }
}

/// Keys a node's GPU buffer: the same file and node is the same content.
fn node_id(file: u64, key: NodeKey) -> u64 {
    let mut h = DefaultHasher::new();
    (file, key).hash(&mut h);
    h.finish()
}
