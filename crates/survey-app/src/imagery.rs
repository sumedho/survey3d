//! Georeferenced images in the session: each image record's tile cache
//! (built in the background, on disk for large images and in memory for
//! small ones), and for the viewport, which tiles a view needs, streamed in
//! on worker threads with coarser tiles standing in until they arrive.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use cad_render::{Camera, ImageFrame, ImagePixels, ImageQuad};
use glam::{DVec2, DVec3};
use survey_core::Aabb3;
use survey_io::imagery::{TILE, TileCache, build_cache};
use survey_model::{Document, ImageRecord};

/// Images up to this many pixels build their tile cache in memory.
const MEMORY_PIXELS: u64 = 8192 * 8192;
/// Decoded tiles kept for the viewport.
const TILE_BUDGET: usize = 700;
/// Most tiles drawn in one view.
const MAX_TILES: usize = 400;

/// What state an image's cache is in.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageStatus {
    Ready,
    /// Building, with the fraction done.
    Building(f32),
    Failed(String),
    /// The image file is not where the record says.
    Missing,
}

enum Entry {
    Building {
        progress: Arc<AtomicU32>,
        cancel: Arc<AtomicBool>,
        rx: Receiver<Result<TileCache, String>>,
    },
    Ready(Arc<Mutex<TileCache>>),
    Failed(String),
    Missing,
}

/// A stable number for an image file (tile keys and cache names).
pub fn path_key(path: &Path) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

/// Image caches by source file.
#[derive(Default)]
pub struct Imagery {
    cache_dir: Option<PathBuf>,
    entries: HashMap<PathBuf, Entry>,
}

impl Imagery {
    /// Where disk caches go (else beside each image, or the temp folder).
    pub fn set_cache_dir(&mut self, dir: Option<PathBuf>) {
        self.cache_dir = dir;
    }

    /// The cache file for an image.
    pub fn cache_path(&self, source: &Path) -> PathBuf {
        let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
        let name = format!("{stem}-{:016x}.s3dimg", path_key(source));
        let dir = self
            .cache_dir
            .clone()
            .or_else(|| source.parent().map(Path::to_path_buf))
            .filter(|d| writable(d))
            .unwrap_or_else(std::env::temp_dir);
        dir.join(name)
    }

    /// Starts or finishes caches for every image in `doc` and drops caches
    /// no image uses. Returns what finished (for the log).
    pub fn sync(&mut self, doc: &Document) -> Vec<String> {
        let wanted: HashMap<PathBuf, (u32, u32, String)> = doc
            .images()
            .iter()
            .map(|r| (r.path.clone(), (r.width, r.height, r.name.clone())))
            .collect();
        self.entries.retain(|path, e| {
            let keep = wanted.contains_key(path);
            if !keep && let Entry::Building { cancel, .. } = e {
                cancel.store(true, Ordering::Relaxed);
            }
            keep
        });
        for (path, (w, h, _)) in &wanted {
            if !self.entries.contains_key(path) {
                let entry = self.start(path, *w, *h);
                self.entries.insert(path.clone(), entry);
            }
        }
        let mut done = Vec::new();
        for (path, entry) in &mut self.entries {
            if let Entry::Building { rx, .. } = entry
                && let Ok(result) = rx.try_recv()
            {
                let name = wanted.get(path).map_or_else(String::new, |w| w.2.clone());
                *entry = match result {
                    Ok(cache) => {
                        let levels = cache.levels;
                        done.push(format!(
                            "Image {name} ready ({levels} level{})",
                            if levels == 1 { "" } else { "s" }
                        ));
                        Entry::Ready(Arc::new(Mutex::new(cache)))
                    }
                    Err(e) => {
                        done.push(format!("Image {name} could not be read: {e}"));
                        Entry::Failed(e)
                    }
                };
            }
        }
        done
    }

    fn start(&self, path: &Path, width: u32, height: u32) -> Entry {
        if !path.exists() {
            return Entry::Missing;
        }
        let memory = u64::from(width) * u64::from(height) <= MEMORY_PIXELS;
        let cache_path = (!memory).then(|| self.cache_path(path));
        if let Some(c) = &cache_path
            && let Some(cache) = TileCache::open(c, path)
        {
            return Entry::Ready(Arc::new(Mutex::new(cache)));
        }
        let (tx, rx) = channel();
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c, source) = (progress.clone(), cancel.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let result = build_cache(&source, cache_path.as_deref(), &mut |f| {
                p.store(f.to_bits(), Ordering::Relaxed);
                !c.load(Ordering::Relaxed)
            })
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        Entry::Building { progress, cancel, rx }
    }

    /// Builds again (after the file was replaced or relinked).
    pub fn rebuild(&mut self, path: &Path) {
        if let Some(Entry::Building { cancel, .. }) = self.entries.remove(path) {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn status(&self, path: &Path) -> ImageStatus {
        match self.entries.get(path) {
            Some(Entry::Ready(_)) => ImageStatus::Ready,
            Some(Entry::Building { progress, .. }) => {
                ImageStatus::Building(f32::from_bits(progress.load(Ordering::Relaxed)))
            }
            Some(Entry::Failed(e)) => ImageStatus::Failed(e.clone()),
            Some(Entry::Missing) => ImageStatus::Missing,
            None => ImageStatus::Building(0.0),
        }
    }

    pub fn cache(&self, path: &Path) -> Option<Arc<Mutex<TileCache>>> {
        match self.entries.get(path) {
            Some(Entry::Ready(c)) => Some(c.clone()),
            _ => None,
        }
    }

    /// Waits for every image's cache to finish (scripts and agents).
    pub fn wait(&mut self, doc: &Document) -> Vec<String> {
        let mut messages = self.sync(doc);
        while self.entries.values().any(|e| matches!(e, Entry::Building { .. })) {
            std::thread::sleep(std::time::Duration::from_millis(20));
            messages.extend(self.sync(doc));
        }
        messages
    }

    /// Whether any cache is still building (to keep repainting).
    pub fn busy(&self) -> bool {
        self.entries.values().any(|e| matches!(e, Entry::Building { .. }))
    }
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".survey3d-write-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

// ====================================================================== the viewport

/// A tile: image file key, level, column, row.
type TileKey = (u64, u32, u32, u32);

fn tile_id(k: TileKey) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    k.hash(&mut h);
    h.finish()
}

struct Request {
    key: TileKey,
    cache: Arc<Mutex<TileCache>>,
}

/// Streams image tiles for the viewport.
pub struct TileStreamer {
    tiles: HashMap<TileKey, (Option<Arc<ImagePixels>>, u64)>,
    pending: HashSet<TileKey>,
    requests: Sender<Request>,
    replies: Receiver<(TileKey, Option<ImagePixels>)>,
    frame: u64,
}

impl Default for TileStreamer {
    fn default() -> Self {
        let (requests, work) = channel::<Request>();
        let (done, replies) = channel();
        let work = Arc::new(Mutex::new(work));
        for _ in 0..3 {
            let (work, done) = (work.clone(), done.clone());
            std::thread::spawn(move || {
                loop {
                    let Ok(req) = work.lock().map_err(|_| ()).and_then(|w| w.recv().map_err(|_| ())) else {
                        return;
                    };
                    let (_, level, x, y) = req.key;
                    let tile = req
                        .cache
                        .lock()
                        .ok()
                        .and_then(|c| c.tile(level, x, y).ok().flatten())
                        .map(|t| ImagePixels {
                            width: t.width,
                            height: t.height,
                            rgba: t.rgba,
                        });
                    if done.send((req.key, tile)).is_err() {
                        return;
                    }
                }
            });
        }
        Self {
            tiles: HashMap::new(),
            pending: HashSet::new(),
            requests,
            replies,
            frame: 0,
        }
    }
}

/// A tile chosen for a view.
struct Wanted {
    key: TileKey,
    corners: [DVec3; 4],
}

impl TileStreamer {
    /// The tiles `camera` needs from the visible images, as a frame for the
    /// renderer; the second value is true while tiles are still loading.
    pub fn frame(
        &mut self,
        doc: &Document,
        imagery: &Imagery,
        camera: &Camera,
    ) -> (Option<Arc<ImageFrame>>, bool) {
        self.frame += 1;
        while let Ok((key, px)) = self.replies.try_recv() {
            self.pending.remove(&key);
            self.tiles.insert(key, (px.map(Arc::new), self.frame));
        }
        let vp = camera.viewport;
        let margin = vp * 0.1;
        let frustum = camera.rect_frustum(-margin, vp + margin);
        let mut quads = Vec::new();
        let mut loading = false;
        for rec in doc.images().iter().filter(|r| r.visible && r.opacity > 0.0) {
            let Some(cache) = imagery.cache(&rec.path) else {
                continue;
            };
            let file = path_key(&rec.path);
            let levels = cache.lock().map(|c| c.levels).unwrap_or(1);
            for w in wanted_tiles(rec, levels, file, camera, &frustum) {
                let (lvl, x, y) = (w.key.1, w.key.2, w.key.3);
                // This tile, else the nearest coarser one loaded, cut to this tile's area.
                for up in 0..levels - lvl {
                    let anc = (file, lvl + up, x >> up, y >> up);
                    match self.tiles.get_mut(&anc) {
                        Some((Some(px), used)) => {
                            *used = self.frame;
                            let uv = if up == 0 {
                                [0.0, 0.0, 1.0, 1.0]
                            } else {
                                // The child's full-resolution area in the ancestor's pixels.
                                let (o, cw, ch) = full_res_tile(rec, lvl, x, y);
                                let (ao, _, _) = full_res_tile(rec, anc.1, anc.2, anc.3);
                                let k = f64::from(1u32 << anc.1);
                                let (pw, ph) = (f64::from(px.width), f64::from(px.height));
                                [
                                    ((o.x - ao.x) / k / pw) as f32,
                                    ((o.y - ao.y) / k / ph) as f32,
                                    ((o.x + cw - ao.x) / k / pw).min(1.0) as f32,
                                    ((o.y + ch - ao.y) / k / ph).min(1.0) as f32,
                                ]
                            };
                            quads.push(ImageQuad {
                                key: tile_id(anc),
                                pixels: px.clone(),
                                corners: w.corners,
                                uv,
                                opacity: rec.opacity,
                            });
                            break;
                        }
                        // An empty tile (all transparent): nothing below it either.
                        Some((None, used)) => {
                            *used = self.frame;
                            break;
                        }
                        None if up == 0 => {
                            loading = true;
                            if self.pending.insert(anc) {
                                let _ = self.requests.send(Request {
                                    key: anc,
                                    cache: cache.clone(),
                                });
                            }
                        }
                        None => {}
                    }
                }
            }
            // The coarsest tile always, so there is something to stand in.
            let top = (file, levels - 1, 0, 0);
            if !self.tiles.contains_key(&top) && self.pending.insert(top) {
                let _ = self.requests.send(Request {
                    key: top,
                    cache: cache.clone(),
                });
            }
        }
        self.evict();
        loading |= !self.pending.is_empty();
        let frame = (!quads.is_empty()).then(|| Arc::new(ImageFrame { quads }));
        (frame, loading)
    }

    fn evict(&mut self) {
        if self.tiles.len() <= TILE_BUDGET {
            return;
        }
        let mut old: Vec<(u64, TileKey)> = self
            .tiles
            .iter()
            .filter(|(_, (_, used))| *used < self.frame)
            .map(|(k, (_, used))| (*used, *k))
            .collect();
        old.sort_unstable();
        let excess = self.tiles.len() - TILE_BUDGET * 3 / 4;
        for (_, k) in old.into_iter().take(excess) {
            self.tiles.remove(&k);
        }
    }
}

/// A level tile's full-resolution pixel origin and size.
fn full_res_tile(rec: &ImageRecord, level: u32, x: u32, y: u32) -> (DVec2, f64, f64) {
    let k = f64::from(1u32 << level);
    let t = f64::from(TILE) * k;
    let (x0, y0) = (f64::from(x) * t, f64::from(y) * t);
    let (w, h) = (
        (f64::from(rec.width) - x0).min(t),
        (f64::from(rec.height) - y0).min(t),
    );
    (DVec2::new(x0, y0), w, h)
}

/// The tiles of an image a view needs: from the coarsest level, a tile is
/// split while it shows bigger on screen than its own pixels and is in view.
fn wanted_tiles(
    rec: &ImageRecord,
    levels: u32,
    file: u64,
    camera: &Camera,
    frustum: &cad_render::Frustum,
) -> Vec<Wanted> {
    let corners_of = |level: u32, x: u32, y: u32| -> [DVec3; 4] {
        let (o, w, h) = full_res_tile(rec, level, x, y);
        [
            o,
            o + DVec2::new(w, 0.0),
            o + DVec2::new(w, h),
            o + DVec2::new(0.0, h),
        ]
        .map(|p| rec.transform.apply(p).extend(rec.elevation))
    };
    let mut out = Vec::new();
    let mut queue = vec![(levels - 1, 0u32, 0u32)];
    while let Some((level, x, y)) = queue.pop() {
        let corners = corners_of(level, x, y);
        let bounds = corners.iter().fold(Aabb3::EMPTY, |b, p| {
            let mut b = b;
            b.extend(*p);
            b
        });
        if !frustum.intersects_aabb(&bounds) {
            continue;
        }
        // On-screen size: the longest projected edge (a corner behind the eye counts as huge).
        let screen: Vec<Option<DVec2>> = corners
            .iter()
            .map(|p| camera.world_to_screen(*p).map(|s| s.0))
            .collect();
        let span = (0..4)
            .map(|i| match (screen[i], screen[(i + 1) % 4]) {
                (Some(a), Some(b)) => a.distance(b),
                _ => f64::INFINITY,
            })
            .fold(0.0, f64::max);
        let split = level > 0 && span > f64::from(TILE) * 1.2 && out.len() + queue.len() < MAX_TILES;
        if split {
            let (lw, lh) = {
                let k = 1u32 << (level - 1);
                (
                    rec.width.div_ceil(k).div_ceil(TILE),
                    rec.height.div_ceil(k).div_ceil(TILE),
                )
            };
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (cx, cy) = (2 * x + dx, 2 * y + dy);
                if cx < lw && cy < lh {
                    queue.push((level - 1, cx, cy));
                }
            }
        } else {
            out.push(Wanted {
                key: (file, level, x, y),
                corners,
            });
        }
    }
    out
}

// ====================================================================== plan sheets

/// The visible images on a plan sheet as rasters resampled at `dpi`,
/// cut to the map frame (drawn under the map).
pub fn sheet_images(
    doc: &Document,
    imagery: &Imagery,
    t: &crate::plan::SheetTransform,
    frame: &crate::plan::Rect,
    dpi: f64,
) -> Vec<survey_io::plot::Item> {
    let px_per_mm = dpi / 25.4;
    let mut items = Vec::new();
    for rec in doc.images().iter().filter(|r| r.visible && r.opacity > 0.0) {
        let Some(cache) = imagery.cache(&rec.path) else {
            continue;
        };
        let Ok(cache) = cache.lock() else {
            continue;
        };
        // The image's footprint on the paper, cut to the frame.
        let paper: Vec<[f64; 2]> = rec.corners().iter().map(|c| t.to_paper(*c)).collect();
        let lo = [
            paper
                .iter()
                .map(|p| p[0])
                .fold(f64::INFINITY, f64::min)
                .max(frame.min[0]),
            paper
                .iter()
                .map(|p| p[1])
                .fold(f64::INFINITY, f64::min)
                .max(frame.min[1]),
        ];
        let hi = [
            paper
                .iter()
                .map(|p| p[0])
                .fold(f64::NEG_INFINITY, f64::max)
                .min(frame.max[0]),
            paper
                .iter()
                .map(|p| p[1])
                .fold(f64::NEG_INFINITY, f64::max)
                .min(frame.max[1]),
        ];
        if hi[0] <= lo[0] || hi[1] <= lo[1] {
            continue;
        }
        let w = ((hi[0] - lo[0]) * px_per_mm).ceil().clamp(1.0, 8000.0) as u32;
        let h = ((hi[1] - lo[1]) * px_per_mm).ceil().clamp(1.0, 8000.0) as u32;
        let (sx, sy) = ((hi[0] - lo[0]) / f64::from(w), (hi[1] - lo[1]) / f64::from(h));
        // The pyramid level whose pixels are about the output's.
        let out_m = sx / t.mm_per_metre();
        let ratio = out_m / rec.transform.pixel_size().max(1e-12);
        let level = if ratio > 1.0 {
            ratio.log2().floor() as u32
        } else {
            0
        };
        let (iw, ih) = (f64::from(rec.width), f64::from(rec.height));
        let mut rgba = survey_io::imagery::resample(&cache, level, w, h, |x, y| {
            let world = t.to_world([lo[0] + x * sx, hi[1] - y * sy]);
            rec.transform
                .inverse(world)
                .filter(|p| p.x >= 0.0 && p.y >= 0.0 && p.x <= iw && p.y <= ih)
        });
        if rec.opacity < 1.0 {
            for a in rgba.iter_mut().skip(3).step_by(4) {
                *a = (f32::from(*a) * rec.opacity).round() as u8;
            }
        }
        items.push(survey_io::plot::Item::Image {
            min: lo,
            max: hi,
            width: w,
            height: h,
            rgba: Arc::new(rgba),
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use survey_core::georef::ImageTransform;
    use survey_model::ChangeSet;

    use super::*;

    /// A 3000 × 2000 PNG at 0.1 m pixels from (1000, 2200), in a document.
    fn setup(name: &str) -> (Document, PathBuf) {
        let dir = std::env::temp_dir().join(format!("survey3d-imagery-app-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("site.png");
        image::RgbaImage::from_fn(3000, 2000, |x, y| {
            image::Rgba([(x / 12) as u8, (y / 8) as u8, 90, 255])
        })
        .save(&path)
        .expect("png");
        let mut doc = Document::new();
        let mut cs = ChangeSet::new("image").insert_record(ImageRecord {
            name: "site".into(),
            path: path.clone(),
            width: 3000,
            height: 2000,
            transform: ImageTransform::north_up(DVec2::new(1000.0, 2200.0), 0.1),
            ..Default::default()
        });
        survey_model::Command::apply(&mut cs, &mut doc);
        (doc, path)
    }

    fn camera(centre: DVec2, distance: f64) -> Camera {
        Camera {
            target: centre.extend(0.0),
            distance,
            viewport: DVec2::new(800.0, 600.0),
            ..Camera::default()
        }
    }

    /// Streams until every tile has arrived.
    fn settle(s: &mut TileStreamer, doc: &Document, imagery: &Imagery, cam: &Camera) -> Arc<ImageFrame> {
        for _ in 0..500 {
            let (frame, loading) = s.frame(doc, imagery, cam);
            if !loading && let Some(f) = frame {
                return f;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("tiles never arrived");
    }

    #[test]
    fn tiles_follow_the_view() {
        let (doc, path) = setup("view");
        let mut imagery = Imagery::default();
        imagery.wait(&doc);
        assert_eq!(imagery.status(&path), ImageStatus::Ready);
        let mut s = TileStreamer::default();
        // The whole image (300 × 200 m) in an 800 px wide view: coarse tiles.
        let far = settle(&mut s, &doc, &imagery, &camera(DVec2::new(1150.0, 2100.0), 400.0));
        assert!(far.quads.len() <= 6, "{} quads", far.quads.len());
        // Close in on one spot: full resolution, only tiles near the view.
        let near_cam = camera(DVec2::new(1050.0, 2150.0), 20.0);
        let near = settle(&mut s, &doc, &imagery, &near_cam);
        let levels: Vec<u32> = near
            .quads
            .iter()
            .map(|q| (q.pixels.width, q.uv))
            .map(|(_, uv)| if uv == [0.0, 0.0, 1.0, 1.0] { 0 } else { 1 })
            .collect();
        assert!(levels.iter().all(|l| *l == 0), "all drawn from their own tiles");
        assert!(near.quads.len() <= 9, "{} quads", near.quads.len());
        // The quads cover the view's centre.
        let c = DVec2::new(1050.0, 2150.0);
        assert!(near.quads.iter().any(|q| {
            let (lo, hi) = (q.corners[3].truncate(), q.corners[1].truncate());
            c.x >= lo.x && c.x <= hi.x && c.y >= lo.y && c.y <= hi.y
        }));
        // Hidden images draw nothing.
        let mut hidden = doc;
        let mut rec = hidden.images()[0].record.clone();
        rec.visible = false;
        let id = hidden.images()[0].id;
        let mut cs = ChangeSet::new("hide").update_record(id, rec);
        survey_model::Command::apply(&mut cs, &mut hidden);
        assert!(s.frame(&hidden, &imagery, &near_cam).0.is_none());
    }

    #[test]
    fn missing_files_are_reported() {
        let (doc, path) = setup("missing");
        std::fs::remove_file(&path).expect("remove");
        let mut imagery = Imagery::default();
        imagery.sync(&doc);
        assert_eq!(imagery.status(&path), ImageStatus::Missing);
    }
}
