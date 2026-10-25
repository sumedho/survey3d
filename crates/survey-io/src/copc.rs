//! Cloud Optimized Point Clouds (COPC): a LAZ 1.4 file whose points are stored
//! in a clustered octree, so a viewer can read a coarse, resolution-bounded
//! subset without decompressing the whole cloud.
//!
//! This is the *reader* only, and native only (no filesystem or LAZ decoder on
//! wasm). [`CopcSource`] parses the hierarchy once, decodes octree nodes in
//! parallel straight from their LAZ chunks and caches them, so the first
//! preview ([`read_copc_coarse`]) and the viewer's node selection
//! ([`CopcSource::select`], then [`CopcSource::load`]) share the work.

use std::path::Path;

use glam::DVec3;

use crate::IoError;
use crate::pointcloud::RawCloud;

/// What a coarse read left in and left out.
#[derive(Clone, Debug, PartialEq)]
pub struct CopcSummary {
    /// Points in the whole file (from the LAS header), not what was read.
    pub total: u64,
    /// File bounds (all points), from the LAS header.
    pub min: DVec3,
    pub max: DVec3,
    /// Whether the read points carry colour.
    pub has_color: bool,
}

fn invalid(e: impl std::fmt::Display) -> IoError {
    IoError::Invalid(format!("COPC: {e}"))
}

/// The COPC info record: the octree's cube and the first hierarchy page.
struct CopcInfo {
    center: DVec3,
    halfsize: f64,
    root_hier_offset: u64,
    root_hier_size: u64,
}

/// A COPC file's LAS header and info record.
fn open_header(path: &Path) -> Result<(las::Header, CopcInfo), IoError> {
    let reader = las::Reader::from_path(path).map_err(|e| invalid(format!("opening the file: {e}")))?;
    let header = reader.header().clone();
    let data = header
        .all_vlrs()
        .find(|v| v.user_id.eq_ignore_ascii_case("copc") && v.record_id == 1)
        .map(|v| v.data.clone())
        .filter(|d| d.len() >= 56)
        .ok_or_else(|| invalid("not a COPC file (no COPC info record); convert it first"))?;
    let f64_at = |i: usize| f64::from_le_bytes(data[i..i + 8].try_into().unwrap_or_default());
    let u64_at = |i: usize| u64::from_le_bytes(data[i..i + 8].try_into().unwrap_or_default());
    let info = CopcInfo {
        center: DVec3::new(f64_at(0), f64_at(8), f64_at(16)),
        halfsize: f64_at(24),
        root_hier_offset: u64_at(40),
        root_hier_size: u64_at(48),
    };
    Ok((header, info))
}

/// A short description of a LAS point record, for diagnosing decode failures.
fn describe_format(header: &las::Header) -> String {
    let f = header.point_format();
    let id = f.to_u8().map_or_else(|_| "?".to_string(), |n| n.to_string());
    format!(
        "PDRF {id}, extra_bytes {}, extended {}, colour {}, nir {}",
        f.extra_bytes, f.is_extended, f.has_color, f.has_nir
    )
}

/// Reads a bounded, coarse subset of a `.copc.laz` file: whole octree levels
/// from the root down, stopping before the level that would push the total past
/// `max_points`. The result is a spatially balanced sample of the whole cloud,
/// not a cropped corner, so it is representative at a glance.
pub fn read_copc_coarse(path: &Path, max_points: usize) -> Result<(RawCloud, CopcSummary), IoError> {
    let (header, _) = open_header(path)?;
    // `format` describes the point record so a decode failure is diagnosable.
    let (total, min, max, format) = {
        let h = &header;
        let b = h.bounds();
        (
            h.number_of_points(),
            DVec3::new(b.min.x, b.min.y, b.min.z),
            DVec3::new(b.max.x, b.max.y, b.max.z),
            describe_format(h),
        )
    };
    let raw = CopcSource::shared(path)
        .and_then(|src| src.read_levels(max_points))
        .map_err(|e| {
            IoError::Invalid(format!(
                "{e} ({format}). This build reads LAS point formats 6–8; re-export the COPC without \
                 extra dimensions."
            ))
        })?;
    if raw.points.is_empty() {
        return Err(IoError::NoPoints(" — the COPC file has no points".into()));
    }
    let has_color = raw.colors.is_some();
    Ok((
        raw,
        CopcSummary {
            total,
            min,
            max,
            has_color,
        },
    ))
}

/// What a view needs from [`CopcSource::select`].
#[derive(Clone, Debug)]
pub struct ViewQuery {
    /// The view's frustum (with a margin) as `(point_on_plane, inward_normal)`
    /// planes; nodes outside it are not refined.
    pub frustum: Vec<(DVec3, DVec3)>,
    /// World units per pixel at the camera's target.
    pub world_per_pixel: f64,
    /// Perspective views: the eye and its distance to the target, so a node's
    /// size on screen is taken at its own distance. `None` for orthographic.
    pub eye: Option<(DVec3, f64)>,
    /// Wanted distance between neighbouring points on screen (pixels): a
    /// node is drawn (in part) only where the coarser levels leave points
    /// further apart than this.
    pub spacing_px: f64,
}

/// An octree node's address: level, then x, y, z at that level.
pub type NodeKey = (i32, i32, i32, i32);

/// Sub-cells per side a node's points are binned into (see [`NodePoints`]).
pub const CELLS_PER_SIDE: usize = 4;
/// Sub-cells per node.
pub const CELLS: usize = CELLS_PER_SIDE * CELLS_PER_SIDE * CELLS_PER_SIDE;

/// A decoded node, ready to draw: positions as `f32` offsets from the node's
/// centre (small, so precise) and the RGBA8 colour's bits in `w` (grey
/// without colour). The alpha byte is not opacity: it is the plan density
/// of the point's column of cells relative to the node's average, as
/// `128 + 16·log2(ratio)` (see [`density_byte`]), so a viewer can size points
/// to their local spacing where a node mixes, say, woodland and open field. Points are grouped by sub-cell of the node's cube
/// (x + 4y + 16z, 4 per side), so a viewer can skip cells outside the view,
/// and shuffled within each cell, so any prefix of a cell is an even sample:
/// COPC's coarse levels are dense, and a view often needs only part of one.
pub struct NodePoints {
    pub points: std::sync::Arc<Vec<[f32; 4]>>,
    /// Cell `i` holds `points[cells[i]..cells[i + 1]]`.
    pub cells: std::sync::Arc<[u32; CELLS + 1]>,
}

/// A node chosen for a view by [`CopcSource::select`]. Parents always come
/// before their children.
#[derive(Clone, Debug)]
pub struct Selected {
    pub key: NodeKey,
    pub center: DVec3,
    /// Edge length of the node's cube.
    pub side: f64,
    /// Plan spacing between points where this is the deepest node drawn: from
    /// the summed density of it and its ancestors (COPC levels add up).
    pub spacing: f64,
    /// Index of the parent in the selection (`None` for the root).
    pub parent: Option<usize>,
    /// Which child of its parent: x | y << 1 | z << 2 (1 = the high half).
    pub octant: u8,
    /// Points in the node (0 for a node that only leads to its children).
    pub count: u32,
    /// Share of the node's points to draw (a prefix of each cell): coarse
    /// levels are often denser than the view needs. Nodes drawn in part are
    /// not refined.
    pub fraction: f32,
}

/// A COPC file opened for repeated view queries. The octree hierarchy is
/// parsed once and decoded nodes are kept in a bounded cache, so moving the
/// view decodes only nodes that have not been seen (or have been evicted), on
/// every core.
pub struct CopcSource {
    path: std::path::PathBuf,
    laz: laz::LazVlr,
    record_len: usize,
    /// Byte offset of RGB inside a point record, when the format has colour.
    color_at: Option<usize>,
    /// Colours use the full 16 bits (else they are 8-bit values in 16-bit
    /// fields), decided once for the file from its root node.
    wide_color: bool,
    scale: [f64; 3],
    offset: [f64; 3],
    root_min: DVec3,
    root_size: f64,
    entries: std::collections::HashMap<NodeKey, Entry>,
    /// The coarsest whole levels (about [`BASE_POINTS`]), in every selection
    /// so areas just outside the view are never blank while it moves.
    base_levels: i32,
    cache: std::sync::Mutex<NodeCache>,
}

#[derive(Clone, Copy)]
struct Entry {
    offset: u64,
    count: u32,
}

#[derive(Default)]
struct NodeCache {
    nodes: std::collections::HashMap<NodeKey, (std::sync::Arc<NodePoints>, u64)>,
    points: usize,
    clock: u64,
}

/// Points in the coarse whole-cloud base under every selection.
const BASE_POINTS: usize = 1_000_000;

/// Decoded points kept per file (16 bytes each).
const CACHE_POINTS: usize = 24_000_000;

/// Grey for points of a file without colour (RGBA8, little-endian bits).
const NO_COLOR: u32 = u32::from_le_bytes([200, 200, 200, 255]);

/// Whole octree levels from the root down, stopping before the level that
/// would push the total past `max_points` (counts come from the hierarchy, so
/// nothing is decoded to decide). Returns the nodes with points and the number
/// of levels.
fn whole_levels(
    entries: &std::collections::HashMap<NodeKey, Entry>,
    max_points: usize,
) -> (Vec<NodeKey>, i32) {
    let mut by_level: Vec<Vec<NodeKey>> = Vec::new();
    for k in entries.keys() {
        let l = k.0.max(0) as usize;
        if by_level.len() <= l {
            by_level.resize(l + 1, Vec::new());
        }
        by_level[l].push(*k);
    }
    let mut keys = Vec::new();
    let mut total = 0usize;
    let mut levels = 0;
    for level in by_level {
        let n: usize = level.iter().map(|k| entries[k].count as usize).sum();
        if !keys.is_empty() && total + n > max_points {
            break;
        }
        total += n;
        keys.extend(level.into_iter().filter(|k| entries[k].count > 0));
        levels += 1;
        if total >= max_points {
            break;
        }
    }
    (keys, levels)
}

/// The share of cell `c` a node drawn at `fraction` shows: all of it when
/// the node is drawn in full; else the fraction scaled by the inverse of the
/// cell's column density relative to the node's (so sparse columns give more
/// and dense ones less, evening the node out), at most all of it. The
/// renderer draws the same shares (`column_share` in `cad_render::clouds`,
/// which cannot depend on this crate): change both together.
pub fn drawn_share(cells: &[u32; CELLS + 1], c: usize, fraction: f64) -> f64 {
    let total = cells[CELLS];
    if fraction >= 1.0 || total == 0 {
        return fraction.min(1.0);
    }
    let per = CELLS_PER_SIDE;
    let column = c % (per * per);
    let n: u32 = (0..per)
        .map(|z| {
            let i = column + z * per * per;
            cells[i + 1] - cells[i]
        })
        .sum();
    let ratio = f64::from(n) * (per * per) as f64 / f64::from(total);
    (fraction / ratio.max(1e-6)).min(1.0)
}

/// A density ratio (local over node average) as a byte: `128 + 16·log2`,
/// clamped (1/256 to 128 times the average).
pub fn density_byte(ratio: f64) -> u8 {
    (128.0 + 16.0 * ratio.max(1e-9).log2()).round().clamp(0.0, 255.0) as u8
}

/// A poisoned lock still guards a usable cache (a decode panicked mid-way).
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn le_i32(b: &[u8], i: usize) -> i32 {
    i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn parent_of(k: NodeKey) -> NodeKey {
    (k.0 - 1, k.1 >> 1, k.2 >> 1, k.3 >> 1)
}

fn children_of(k: NodeKey) -> impl Iterator<Item = NodeKey> {
    (0..8).map(move |i| {
        (
            k.0 + 1,
            k.1 * 2 + (i & 1),
            k.2 * 2 + ((i >> 1) & 1),
            k.3 * 2 + ((i >> 2) & 1),
        )
    })
}

impl CopcSource {
    /// The process-wide source for `path`, opened on first use, so the initial
    /// preview and later view reads share one hierarchy and node cache.
    pub fn shared(path: &Path) -> Result<std::sync::Arc<Self>, IoError> {
        use std::sync::{Arc, Mutex, OnceLock};
        static OPEN: OnceLock<Mutex<std::collections::HashMap<std::path::PathBuf, Arc<CopcSource>>>> =
            OnceLock::new();
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut open = lock(OPEN.get_or_init(Default::default));
        if let Some(s) = open.get(&key) {
            return Ok(s.clone());
        }
        let s = Arc::new(Self::open(path)?);
        open.insert(key, s.clone());
        Ok(s)
    }

    pub fn open(path: &Path) -> Result<Self, IoError> {
        use std::io::{Read, Seek, SeekFrom};
        let (header, info) = open_header(path)?;
        let format = *header.point_format();
        let fmt = format.to_u8().map_err(invalid)?;
        let t = header.transforms();
        let laz = header
            .all_vlrs()
            .find(|v| v.user_id.eq_ignore_ascii_case("laszip encoded") && v.record_id == 22204)
            .ok_or_else(|| invalid("no LASzip VLR"))
            .and_then(|v| laz::LazVlr::read_from(v.data.as_slice()).map_err(invalid))?;
        let record_len = (format.len() + format.extra_bytes) as usize;
        let color_at = matches!(fmt, 7 | 8 | 10).then_some(30);
        let (scale, offset) = (
            [t.x.scale, t.y.scale, t.z.scale],
            [t.x.offset, t.y.offset, t.z.offset],
        );

        // Walk the hierarchy pages (32-byte entries; count -1 points at a child page).
        let mut file = std::io::BufReader::new(std::fs::File::open(path).map_err(invalid)?);
        let mut entries = std::collections::HashMap::new();
        let mut pages = vec![(info.root_hier_offset, info.root_hier_size)];
        while let Some((off, size)) = pages.pop() {
            file.seek(SeekFrom::Start(off)).map_err(invalid)?;
            let mut buf = vec![0u8; size as usize];
            file.read_exact(&mut buf).map_err(invalid)?;
            for e in buf.as_chunks::<32>().0 {
                let key = (le_i32(e, 0), le_i32(e, 4), le_i32(e, 8), le_i32(e, 12));
                let offset = u64::from(le_i32(e, 16) as u32) | (u64::from(le_i32(e, 20) as u32) << 32);
                let bytes = le_i32(e, 24);
                let count = le_i32(e, 28);
                if count == -1 {
                    pages.push((offset, bytes as u64));
                } else if count >= 0 {
                    // Empty nodes are kept: refinement passes through them to
                    // descendants that have points.
                    entries.insert(
                        key,
                        Entry {
                            offset,
                            count: count as u32,
                        },
                    );
                }
            }
        }
        let (_, base_levels) = whole_levels(&entries, BASE_POINTS);
        let mut source = Self {
            path: path.to_path_buf(),
            laz,
            record_len,
            color_at,
            wide_color: false,
            scale,
            offset,
            root_min: info.center - DVec3::splat(info.halfsize),
            root_size: info.halfsize * 2.0,
            entries,
            base_levels,
            cache: Default::default(),
        };
        // 8-bit colours stored in 16-bit fields are common: judge by the root.
        if source.color_at.is_some() && source.entries.contains_key(&(0, 0, 0, 0)) {
            let (_, rgb) = source.decode_raw((0, 0, 0, 0))?;
            source.wide_color = rgb.iter().flatten().any(|c| c.iter().any(|v| *v > 255));
        }
        Ok(source)
    }

    /// Whether the file's points carry colour.
    pub fn has_color(&self) -> bool {
        self.color_at.is_some()
    }

    fn node_bounds(&self, k: NodeKey) -> (DVec3, DVec3) {
        let side = self.root_size / (1u64 << k.0) as f64;
        let min = self.root_min + DVec3::new(k.1 as f64, k.2 as f64, k.3 as f64) * side;
        (min, min + DVec3::splat(side))
    }

    /// Quantised positions and (16-bit) colours as stored in the file.
    #[allow(clippy::type_complexity)]
    fn decode_raw(&self, key: NodeKey) -> Result<(Vec<[i32; 3]>, Option<Vec<[u16; 3]>>), IoError> {
        use laz::record::{LayeredPointRecordDecompressor, RecordDecompressor};
        use std::io::{Seek, SeekFrom};
        let e = self.entries[&key];
        let mut file =
            std::io::BufReader::with_capacity(1 << 16, std::fs::File::open(&self.path).map_err(invalid)?);
        file.seek(SeekFrom::Start(e.offset)).map_err(invalid)?;
        let mut dec = LayeredPointRecordDecompressor::new(file);
        dec.set_fields_from(self.laz.items()).map_err(invalid)?;
        let mut rec = vec![0u8; self.record_len];
        let n = e.count as usize;
        let mut xyz = Vec::with_capacity(n);
        let mut rgb = self.color_at.map(|_| Vec::with_capacity(n));
        for _ in 0..n {
            dec.decompress_next(&mut rec).map_err(invalid)?;
            xyz.push([le_i32(&rec, 0), le_i32(&rec, 4), le_i32(&rec, 8)]);
            if let (Some(at), Some(rgb)) = (self.color_at, rgb.as_mut()) {
                let u = |i: usize| u16::from_le_bytes([rec[at + i], rec[at + i + 1]]);
                rgb.push([u(0), u(2), u(4)]);
            }
        }
        Ok((xyz, rgb))
    }

    fn decode(&self, key: NodeKey) -> Result<NodePoints, IoError> {
        let (xyz, rgb) = self.decode_raw(key)?;
        let (min, max) = self.node_bounds(key);
        let c = (min + max) * 0.5;
        let wide = self.wide_color;
        let side = max.x - min.x;
        let points: Vec<[f32; 4]> = xyz
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let color = rgb.as_ref().map_or(NO_COLOR, |rgb| {
                    let [r, g, b] = rgb[i].map(|v| if wide { (v >> 8) as u8 } else { v as u8 });
                    u32::from_le_bytes([r, g, b, 255])
                });
                [
                    (p[0] as f64 * self.scale[0] + self.offset[0] - c.x) as f32,
                    (p[1] as f64 * self.scale[1] + self.offset[1] - c.y) as f32,
                    (p[2] as f64 * self.scale[2] + self.offset[2] - c.z) as f32,
                    f32::from_bits(color),
                ]
            })
            .collect();
        // Group by cell (a counting sort), then shuffle within each cell.
        let cell_of = |p: &[f32; 4]| {
            let at = |v: f32| {
                ((f64::from(v) / side + 0.5) * CELLS_PER_SIDE as f64).clamp(0.0, (CELLS_PER_SIDE - 1) as f64)
                    as usize
            };
            at(p[0]) + CELLS_PER_SIDE * (at(p[1]) + CELLS_PER_SIDE * at(p[2]))
        };
        let mut cells = [0u32; CELLS + 1];
        for p in &points {
            cells[cell_of(p) + 1] += 1;
        }
        for i in 0..CELLS {
            cells[i + 1] += cells[i];
        }
        let mut next = cells;
        let mut sorted = vec![[0f32; 4]; points.len()];
        for p in &points {
            let c = cell_of(p);
            sorted[next[c] as usize] = *p;
            next[c] += 1;
        }
        let mut state = 0x9E37_79B9_7F4A_7C15u64
            ^ (key.0 as u64) << 48
            ^ (key.1 as u64) << 32
            ^ (key.2 as u64) << 16
            ^ key.3 as u64;
        for c in 0..CELLS {
            let run = &mut sorted[cells[c] as usize..cells[c + 1] as usize];
            for i in (1..run.len()).rev() {
                // xorshift64*
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                let j = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) % (i as u64 + 1)) as usize;
                run.swap(i, j);
            }
        }
        // Each column of cells' density relative to the node's: the column
        // holds `n` of the node's points on 1/16 of its footprint.
        let total = sorted.len().max(1) as f64;
        let per_side = CELLS_PER_SIDE;
        for c in 0..CELLS {
            let column = c % (per_side * per_side);
            let n: u32 = (0..per_side)
                .map(|z| {
                    let i = column + z * per_side * per_side;
                    cells[i + 1] - cells[i]
                })
                .sum();
            let ratio = f64::from(n) * (per_side * per_side) as f64 / total;
            let byte = density_byte(ratio);
            for p in &mut sorted[cells[c] as usize..cells[c + 1] as usize] {
                let [r, g, b, _] = p[3].to_bits().to_le_bytes();
                p[3] = f32::from_bits(u32::from_le_bytes([r, g, b, byte]));
            }
        }
        Ok(NodePoints {
            points: std::sync::Arc::new(sorted),
            cells: std::sync::Arc::new(cells),
        })
    }

    /// The node if it is decoded, without decoding it.
    pub fn cached(&self, key: NodeKey) -> Option<std::sync::Arc<NodePoints>> {
        let mut c = lock(&self.cache);
        c.clock += 1;
        let clock = c.clock;
        c.nodes.get_mut(&key).map(|(n, used)| {
            *used = clock;
            n.clone()
        })
    }

    /// The node, decoding it first when absent.
    pub fn node(&self, key: NodeKey) -> Result<std::sync::Arc<NodePoints>, IoError> {
        if let Some(n) = self.cached(key) {
            return Ok(n);
        }
        let node = std::sync::Arc::new(self.decode(key)?);
        let mut c = lock(&self.cache);
        c.points += node.points.len();
        let clock = c.clock;
        c.nodes.insert(key, (node.clone(), clock));
        // Evict the least recently used nodes beyond the budget.
        if c.points > CACHE_POINTS {
            let mut by_age: Vec<(u64, NodeKey)> = c.nodes.iter().map(|(k, (_, t))| (*t, *k)).collect();
            by_age.sort_unstable();
            for (_, k) in by_age {
                if c.points <= CACHE_POINTS * 9 / 10 {
                    break;
                }
                if k != key
                    && let Some((n, _)) = c.nodes.remove(&k)
                {
                    c.points -= n.points.len();
                }
            }
        }
        Ok(node)
    }

    /// Decodes the uncached `keys` on every core.
    pub fn load(&self, keys: &[NodeKey]) -> Result<(), IoError> {
        let missing: Vec<NodeKey> = {
            let c = lock(&self.cache);
            keys.iter()
                .copied()
                .filter(|k| self.entries.get(k).is_some_and(|e| e.count > 0) && !c.nodes.contains_key(k))
                .collect()
        };
        let next = std::sync::atomic::AtomicUsize::new(0);
        let failed = std::sync::Mutex::new(None);
        let workers = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(missing.len());
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&k) = missing.get(i) else { break };
                        if let Err(e) = self.node(k) {
                            *lock(&failed) = Some(e);
                            break;
                        }
                    }
                });
            }
        });
        match failed
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// The nodes to draw for a view, as Potree chooses them, adapted to
    /// COPC: the coarse base of the whole cloud, then nodes inside the frustum
    /// from a priority queue, the place where points are furthest apart on
    /// screen first, until `max_points` are drawn (counting only a node's
    /// cells inside the frustum). COPC's coarse levels are dense, so a node is
    /// drawn only in the share the view needs to bring points to
    /// `spacing_px` apart (see [`NodePoints`] and [`drawn_share`]); its
    /// children are still refined where that share leaves gaps. Parents come
    /// before children; levels add to their ancestors.
    pub fn select(&self, view: &ViewQuery, max_points: usize) -> Vec<Selected> {
        let spacing_wanted = view.spacing_px;
        use std::collections::{BinaryHeap, HashMap};
        let box_in_frustum = |min: DVec3, max: DVec3| {
            view.frustum.iter().all(|(p, n)| {
                let far = DVec3::new(
                    if n.x >= 0.0 { max.x } else { min.x },
                    if n.y >= 0.0 { max.y } else { min.y },
                    if n.z >= 0.0 { max.z } else { min.z },
                );
                n.dot(far - *p) >= 0.0
            })
        };
        let in_frustum = |k: NodeKey| {
            let (min, max) = self.node_bounds(k);
            box_in_frustum(min, max)
        };
        // Share of the node's points in cells inside the frustum: real
        // counts once decoded, else assumed even.
        let visible_share = |k: NodeKey| {
            let (min, _) = self.node_bounds(k);
            let cell = self.root_size / (1u64 << k.0) as f64 / CELLS_PER_SIDE as f64;
            let cached = self.cached(k);
            let mut seen = 0.0;
            let mut all = 0.0;
            for c in 0..CELLS {
                let (x, y, z) = (
                    c % CELLS_PER_SIDE,
                    c / CELLS_PER_SIDE % CELLS_PER_SIDE,
                    c / (CELLS_PER_SIDE * CELLS_PER_SIDE),
                );
                let weight = cached
                    .as_ref()
                    .map_or(1.0, |n| f64::from(n.cells[c + 1] - n.cells[c]));
                all += weight;
                let lo = min + DVec3::new(x as f64, y as f64, z as f64) * cell;
                if weight > 0.0 && box_in_frustum(lo, lo + DVec3::splat(cell)) {
                    seen += weight;
                }
            }
            if all > 0.0 { seen / all } else { 0.0 }
        };
        // World units per pixel at the node (at its nearest point in perspective).
        let wpp_at = |k: NodeKey| {
            let wpp = view.world_per_pixel.max(1e-12);
            match view.eye {
                None => wpp,
                Some((eye, distance)) => {
                    let (min, max) = self.node_bounds(k);
                    let d = (eye - eye.clamp(min, max)).length().max(1e-3);
                    wpp * d / distance.max(1e-9)
                }
            }
        };
        // The node's plan density over its footprint (points per square unit).
        // Its own points only: summing the nodes stacked over the same ground
        // would count a canopy against the ground beneath it.
        let density = |k: NodeKey| {
            let side = self.root_size / (1u64 << k.0) as f64;
            self.entries.get(&k).map_or(0, |e| e.count) as f64 / (side * side)
        };
        // Point spacing on screen (pixels) at `summed` density there.
        let spacing_px = |k: NodeKey, summed: f64| 1.0 / summed.max(1e-12).sqrt() / wpp_at(k);

        // The density a node draws over one child's region (points per square
        // unit): from its cells once decoded, counting each cell's drawn share
        // as the renderer draws it (see `drawn_share`), else its average.
        let drawn_in = |k: NodeKey, fraction: f64, octant: usize| {
            let half = self.root_size / (1u64 << k.0) as f64 * 0.5;
            let Some(n) = self.cached(k) else {
                return density(k) * fraction;
            };
            let per = CELLS_PER_SIDE;
            let (ox, oy, oz) = (octant & 1, (octant >> 1) & 1, (octant >> 2) & 1);
            let mut drawn = 0.0;
            for z in oz * per / 2..(oz + 1) * per / 2 {
                for y in oy * per / 2..(oy + 1) * per / 2 {
                    for x in ox * per / 2..(ox + 1) * per / 2 {
                        let c = x + per * (y + per * z);
                        let len = n.cells[c + 1] - n.cells[c];
                        drawn += (f64::from(len) * drawn_share(&n.cells, c, fraction)).ceil();
                    }
                }
            }
            drawn / (half * half)
        };

        let mut out: Vec<Selected> = Vec::new();
        // Each selected node's index, share drawn and the density below it
        // (its ancestors' drawn points over its region).
        let mut index: HashMap<NodeKey, (usize, f64, f64)> = HashMap::new();
        // The density the ancestors already draw over `k`'s region.
        let below_of = |k: NodeKey, index: &HashMap<NodeKey, (usize, f64, f64)>| {
            let p = parent_of(k);
            index.get(&p).map_or(0.0, |&(_, fraction, below)| {
                let octant = ((k.1 & 1) | ((k.2 & 1) << 1) | ((k.3 & 1) << 2)) as usize;
                below + drawn_in(p, fraction, octant)
            })
        };
        let add = |k: NodeKey,
                   fraction: f64,
                   below: f64,
                   out: &mut Vec<Selected>,
                   index: &mut HashMap<NodeKey, (usize, f64, f64)>| {
            let count = self.entries.get(&k).map_or(0, |e| e.count);
            let parent = (k.0 > 0).then(|| index.get(&parent_of(k)).map(|e| e.0)).flatten();
            let summed = below + density(k) * fraction;
            let (min, max) = self.node_bounds(k);
            index.insert(k, (out.len(), fraction, below));
            out.push(Selected {
                key: k,
                center: (min + max) * 0.5,
                side: max.x - min.x,
                spacing: 1.0 / summed.max(1e-12).sqrt(),
                parent,
                octant: ((k.1 & 1) | ((k.2 & 1) << 1) | ((k.3 & 1) << 2)) as u8,
                count,
                fraction: fraction as f32,
            });
        };

        let root = (0, 0, 0, 0);
        if !self.entries.contains_key(&root) {
            return out;
        }
        // The base, level by level (parents first), whatever the view.
        let mut total = 0usize;
        let mut level = vec![root];
        while let Some(&first) = level.first()
            && first.0 < self.base_levels
        {
            let mut next = Vec::new();
            for k in level {
                let below = below_of(k, &index);
                add(k, 1.0, below, &mut out, &mut index);
                total += self.entries.get(&k).map_or(0, |e| e.count as usize);
                next.extend(children_of(k).filter(|c| self.entries.contains_key(c)));
            }
            level = next;
        }
        // Then refine into the view where points are furthest apart on screen
        // first: (spacing there, node, density below it). Values are positive,
        // so their bit patterns order like the values.
        let mut queue: BinaryHeap<(u64, NodeKey, u64)> = level
            .into_iter()
            .filter(|k| in_frustum(*k))
            .map(|k| {
                let below = below_of(k, &index);
                (spacing_px(k, below).to_bits(), k, below.to_bits())
            })
            .collect();
        while let Some((gap, k, below)) = queue.pop() {
            if f64::from_bits(gap) <= spacing_wanted {
                break; // the rest of the view is dense enough
            }
            let below = f64::from_bits(below);
            let wanted = 1.0 / (spacing_wanted * wpp_at(k)).powi(2);
            let d = density(k);
            let fraction = if d > 0.0 {
                ((wanted - below) / d).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let count = self.entries.get(&k).map_or(0, |e| e.count as usize);
            let cost = (count as f64 * fraction * visible_share(k)).ceil() as usize;
            if total + cost > max_points {
                continue; // too dear: leave this place at its coarser levels
            }
            total += cost;
            add(k, fraction, below, &mut out, &mut index);
            // Children too where this node, drawn in part, leaves gaps: its
            // sparse columns may be all drawn and still short of the target.
            for c in children_of(k) {
                if self.entries.contains_key(&c) && in_frustum(c) {
                    let below = below_of(c, &index);
                    queue.push((spacing_px(c, below).to_bits(), c, below.to_bits()));
                }
            }
        }
        out
    }

    /// Whole octree levels from the root down, at most `max_points` (see
    /// [`whole_levels`]).
    pub fn read_levels(&self, max_points: usize) -> Result<RawCloud, IoError> {
        let (keys, _) = whole_levels(&self.entries, max_points);
        self.load(&keys)?;
        let mut points = Vec::new();
        let mut colors = Vec::new();
        for k in keys {
            let node = self.node(k)?;
            let (min, max) = self.node_bounds(k);
            let c = (min + max) * 0.5;
            for p in node.points.iter() {
                points.push(c + DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])));
                let [r, g, b, _] = p[3].to_bits().to_le_bytes();
                colors.push([r, g, b]);
            }
        }
        let colors = self.has_color().then_some(colors);
        Ok(RawCloud { points, colors })
    }
}
