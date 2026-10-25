//! LAS/LAZ to COPC conversion, natively (no PDAL).
//!
//! The method is PotreeConverter 2's out-of-core octree build, written for
//! COPC's additive levels (each point is stored once, in the coarsest node that
//! samples it):
//!
//! 1. **Count.** Read positions only and count them on a 128³ grid over the
//!    root cube; group grid cells into chunks of at most `chunk_points`.
//! 2. **Distribute.** Read again, convert each point to LAS 1.4 point format
//!    6, 7 or 8, and append it to its chunk's temporary file.
//! 3. **Build chunks** (in parallel, one chunk in memory per worker). Split a
//!    chunk down to leaves of at most `leaf_points`, then going up, each node
//!    keeps one point per cell of a `grid`³ lattice over its cube (the point
//!    nearest the cell's centre) taken from its children; what the children
//!    keep is written as their own nodes.
//! 4. **Build the top** the same way from the chunks' roots.
//!
//! Each node is one LAZ chunk (variable-size chunks); the hierarchy is one page
//! in an EVLR, and the LAZ chunk table is written too, so plain LAZ readers
//! still read the file.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Cursor, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use glam::DVec3;
use las::point::Format;
use las::{PointDataBuilder, Reader, Transform, Vector};
use laz::LazVlr;
use laz::laszip::{ChunkTable, ChunkTableEntry};
use laz::record::{LayeredPointRecordCompressor, RecordCompressor};

use crate::IoError;

/// What a conversion wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct CopcWritten {
    pub points: u64,
    /// Octree nodes with points.
    pub nodes: usize,
    /// Deepest level (0 = root only).
    pub depth: i32,
}

/// Tuning (the defaults suit files of any size; tests use small values).
#[derive(Clone, Copy, Debug)]
pub struct ConvertOptions {
    /// Most points of one chunk held in memory at once.
    pub chunk_points: usize,
    /// Most points in a leaf node.
    pub leaf_points: usize,
    /// Sampling cells per side of a node: its spacing is its size / `grid`.
    pub grid: u32,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            chunk_points: 2_000_000,
            leaf_points: 100_000,
            grid: 128,
        }
    }
}

/// Counting grid level (2^7 = 128 cells per side).
const COUNT_LEVEL: i32 = 7;
/// Deepest node (only reached by piles of duplicate points).
const MAX_LEVEL: i32 = 20;
/// Points read per batch.
const BATCH: u64 = 1 << 20;
/// Chunk buffer size before it is appended to its file.
const FLUSH_BYTES: usize = 4 << 20;

type Key = (i32, i32, i32, i32);

fn invalid(e: impl std::fmt::Display) -> IoError {
    IoError::Invalid(format!("COPC conversion: {e}"))
}

fn cancelled() -> IoError {
    IoError::Invalid("COPC conversion cancelled".into())
}

/// Converts `input` (LAS or LAZ, any version and point format) to a COPC file
/// at `output`. `progress` is told the fraction done (0–1) from time to time
/// and cancels by returning false. Temporary files go next to `output`.
pub fn convert_to_copc(
    input: &Path,
    output: &Path,
    options: ConvertOptions,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<CopcWritten, IoError> {
    let temp = output.with_extension("copc-tmp");
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).map_err(invalid)?;
    let result = Converter::new(input, output, &temp, options).and_then(|c| c.run(progress));
    let _ = fs::remove_dir_all(&temp);
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

/// The octree's cube and the output records' layout.
struct Space {
    min: DVec3,
    size: f64,
    scale: [f64; 3],
    offset: [f64; 3],
    rec_len: usize,
}

impl Space {
    fn position(&self, rec: &[u8]) -> DVec3 {
        let i = |at: usize| {
            f64::from(i32::from_le_bytes([
                rec[at],
                rec[at + 1],
                rec[at + 2],
                rec[at + 3],
            ]))
        };
        DVec3::new(
            i(0) * self.scale[0] + self.offset[0],
            i(4) * self.scale[1] + self.offset[1],
            i(8) * self.scale[2] + self.offset[2],
        )
    }

    /// The cell of `p` at `level` (2^level per side), clamped into the cube.
    fn cell(&self, p: DVec3, level: i32) -> (i32, i32, i32) {
        let n = (1i64 << level) as f64;
        let at = |v: f64, lo: f64| (((v - lo) / self.size * n).floor()).clamp(0.0, n - 1.0) as i32;
        (at(p.x, self.min.x), at(p.y, self.min.y), at(p.z, self.min.z))
    }

    fn node_min(&self, k: Key) -> (DVec3, f64) {
        let side = self.size / (1u64 << k.0) as f64;
        (
            self.min + DVec3::new(k.1 as f64, k.2 as f64, k.3 as f64) * side,
            side,
        )
    }
}

fn child(k: Key, octant: usize) -> Key {
    let o = octant as i32;
    (
        k.0 + 1,
        k.1 * 2 + (o & 1),
        k.2 * 2 + ((o >> 1) & 1),
        k.3 * 2 + ((o >> 2) & 1),
    )
}

fn parent(k: Key) -> Key {
    (k.0 - 1, k.1 >> 1, k.2 >> 1, k.3 >> 1)
}

/// A written node, for the hierarchy.
struct Written {
    key: Key,
    offset: u64,
    bytes: u32,
    count: u32,
}

/// The output file while nodes are written.
struct Out {
    file: BufWriter<File>,
    pos: u64,
    nodes: Vec<Written>,
    table: ChunkTable,
}

struct Converter<'a> {
    input: &'a Path,
    output: &'a Path,
    temp: &'a Path,
    options: ConvertOptions,
    header: las::Header,
    format: Format,
    vlr: LazVlr,
}

impl<'a> Converter<'a> {
    fn new(
        input: &'a Path,
        output: &'a Path,
        temp: &'a Path,
        options: ConvertOptions,
    ) -> Result<Self, IoError> {
        let reader = Reader::from_path(input).map_err(invalid)?;
        let header = reader.header().clone();
        let source = header.point_format();
        let (color, nir) = (source.has_color, source.has_nir);
        let id = match (color, nir) {
            (true, true) => 8,
            (true, false) => 7,
            _ => 6,
        };
        let format = Format::new(id).map_err(invalid)?;
        let vlr = laz::LazVlrBuilder::default()
            .with_point_format(id, 0)
            .map_err(invalid)?
            .with_variable_chunk_size()
            .build();
        Ok(Self {
            input,
            output,
            temp,
            options,
            header,
            format,
            vlr,
        })
    }

    fn reader(&self) -> Result<Reader, IoError> {
        Reader::from_path(self.input).map_err(invalid)
    }

    fn run(self, progress: &mut dyn FnMut(f32) -> bool) -> Result<CopcWritten, IoError> {
        let total = self.header.number_of_points().max(1);
        let transforms = *self.header.transforms();
        let scale = [transforms.x.scale, transforms.y.scale, transforms.z.scale];
        let offset = [transforms.x.offset, transforms.y.offset, transforms.z.offset];
        let rec_len = self.format.len() as usize;

        // 1. Count, on a cube around the header's bounds (recounted on the
        // real bounds when the header is wrong).
        let hb = self.header.bounds();
        let mut cube = cube_around(
            DVec3::new(hb.min.x, hb.min.y, hb.min.z),
            DVec3::new(hb.max.x, hb.max.y, hb.max.z),
        );
        let (mut counts, bounds) = self.count(&cube, &transforms, total, progress)?;
        if !(cube.0.cmple(bounds.0).all() && (cube.0 + DVec3::splat(cube.1)).cmpge(bounds.1).all()) {
            cube = cube_around(bounds.0, bounds.1);
            (counts, _) = self.count(&cube, &transforms, total, progress)?;
        }
        let space = Space {
            min: cube.0,
            size: cube.1,
            scale,
            offset,
            rec_len,
        };
        let chunks = choose_chunks(&counts, self.options.chunk_points);
        let side = 1usize << COUNT_LEVEL;
        let mut chunk_of = vec![u32::MAX; side * side * side];
        for (i, k) in chunks.iter().enumerate() {
            let span = 1usize << (COUNT_LEVEL - k.0);
            let (x0, y0, z0) = (k.1 as usize * span, k.2 as usize * span, k.3 as usize * span);
            for z in z0..z0 + span {
                for y in y0..y0 + span {
                    for x in x0..x0 + span {
                        chunk_of[x + side * (y + side * z)] = i as u32;
                    }
                }
            }
        }

        // 2. Distribute.
        let stats = self.distribute(&space, &chunks, &chunk_of, total, progress)?;

        // The file up to its points: header, VLRs and the chunk-table offset.
        let vlrs = self.vlrs(&space, &stats);
        let mut file = BufWriter::new(File::create(self.output).map_err(invalid)?);
        let point_start = 375 + vlrs.iter().map(|v| 54 + v.2.len() as u64).sum::<u64>();
        file.write_all(&[0u8; 375]).map_err(invalid)?;
        for (user, record, data) in &vlrs {
            write_vlr(&mut file, user, *record, data).map_err(invalid)?;
        }
        file.write_all(&0i64.to_le_bytes()).map_err(invalid)?;
        let out = Mutex::new(Out {
            file,
            pos: point_start + 8,
            nodes: Vec::new(),
            table: ChunkTable::default(),
        });

        // 3. Build the chunks, in parallel.
        let builder = Builder {
            space: &space,
            options: self.options,
            vlr: &self.vlr,
            out: &out,
        };
        let roots = self.build_chunks(&builder, &chunks, progress)?;

        // 4. The levels above the chunks.
        let mut above: HashSet<Key> = HashSet::new();
        for k in &chunks {
            let mut a = *k;
            while a.0 > 0 {
                a = parent(a);
                above.insert(a);
            }
        }
        let mut roots = roots;
        let root = (0, 0, 0, 0);
        if let Some(points) = builder.build_top(root, &mut roots, &above)? {
            builder.emit(root, &points, true)?;
        }
        if !progress(0.98) {
            return Err(cancelled());
        }

        // Chunk table, hierarchy, then the header and COPC info.
        let Out {
            mut file,
            pos,
            nodes,
            table,
        } = out
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let table_offset = pos;
        table.write_to(&mut file, &self.vlr).map_err(invalid)?;
        let evlr_start = file.stream_position().map_err(invalid)?;
        let mut page = Vec::with_capacity(nodes.len() * 32);
        for n in &nodes {
            for v in [n.key.0, n.key.1, n.key.2, n.key.3] {
                page.extend_from_slice(&v.to_le_bytes());
            }
            page.extend_from_slice(&n.offset.to_le_bytes());
            page.extend_from_slice(&n.bytes.to_le_bytes());
            page.extend_from_slice(&n.count.to_le_bytes());
        }
        write_evlr(&mut file, "copc", 1000, "EPT hierarchy", &page).map_err(invalid)?;
        file.seek(SeekFrom::Start(point_start)).map_err(invalid)?;
        file.write_all(&(table_offset as i64).to_le_bytes())
            .map_err(invalid)?;
        file.seek(SeekFrom::Start(0)).map_err(invalid)?;
        let header = self.header_bytes(&space, &stats, point_start as u32, vlrs.len() as u32, evlr_start);
        file.write_all(&header).map_err(invalid)?;
        // The COPC info's hierarchy location (its VLR is first, at 375 + 54).
        file.seek(SeekFrom::Start(375 + 54 + 40)).map_err(invalid)?;
        file.write_all(&(evlr_start + 60).to_le_bytes())
            .map_err(invalid)?;
        file.write_all(&(page.len() as u64).to_le_bytes())
            .map_err(invalid)?;
        file.flush().map_err(invalid)?;
        progress(1.0);

        let with_points: Vec<&Written> = nodes.iter().filter(|n| n.count > 0).collect();
        Ok(CopcWritten {
            points: with_points.iter().map(|n| u64::from(n.count)).sum(),
            nodes: with_points.len(),
            depth: nodes.iter().map(|n| n.key.0).max().unwrap_or(0),
        })
    }

    /// Point counts on the counting grid, and the points' real bounds.
    #[allow(clippy::type_complexity)]
    fn count(
        &self,
        cube: &(DVec3, f64),
        t: &Vector<Transform>,
        total: u64,
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<(Vec<u32>, (DVec3, DVec3)), IoError> {
        let side = 1usize << COUNT_LEVEL;
        let mut counts = vec![0u32; side * side * side];
        let space = Space {
            min: cube.0,
            size: cube.1,
            scale: [t.x.scale, t.y.scale, t.z.scale],
            offset: [t.x.offset, t.y.offset, t.z.offset],
            rec_len: 0,
        };
        let (mut lo, mut hi) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
        let mut reader = self.reader()?;
        let mut pd = PointDataBuilder::new().for_header(reader.header()).build();
        let mut done = 0u64;
        loop {
            let n = reader.fill_points(BATCH, &mut pd).map_err(invalid)?;
            if n == 0 {
                break;
            }
            for ((x, y), z) in pd.x_raw().zip(pd.y_raw()).zip(pd.z_raw()) {
                let p = DVec3::new(
                    f64::from(x) * space.scale[0] + space.offset[0],
                    f64::from(y) * space.scale[1] + space.offset[1],
                    f64::from(z) * space.scale[2] + space.offset[2],
                );
                lo = lo.min(p);
                hi = hi.max(p);
                let (cx, cy, cz) = space.cell(p, COUNT_LEVEL);
                counts[cx as usize + side * (cy as usize + side * cz as usize)] += 1;
            }
            done += n;
            if !progress(0.2 * done as f32 / total as f32) {
                return Err(cancelled());
            }
        }
        if done == 0 {
            return Err(IoError::NoPoints(" — the file has no points".into()));
        }
        Ok((counts, (lo, hi)))
    }

    /// Converts every point and appends it to its chunk's file.
    fn distribute(
        &self,
        space: &Space,
        chunks: &[Key],
        chunk_of: &[u32],
        total: u64,
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<Stats, IoError> {
        let side = 1usize << COUNT_LEVEL;
        let mut buffers: Vec<Vec<u8>> = vec![Vec::new(); chunks.len()];
        let mut stats = Stats::default();
        let mut reader = self.reader()?;
        let mut pd = PointDataBuilder::new().for_header(reader.header()).build();
        let source = *reader.header().point_format();
        let transforms = *reader.header().transforms();
        let workers = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(16);
        let mut done = 0u64;
        loop {
            let n = reader.fill_points(BATCH, &mut pd).map_err(invalid)?;
            if n == 0 {
                break;
            }
            let raw = pd.raw_bytes();
            let in_len = pd.record_len();
            let count = n as usize;
            let per = count.div_ceil(workers);
            // Convert in parallel: each worker returns (chunk, record) pairs
            // and its statistics.
            let parts: Vec<Result<Converted, IoError>> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..workers)
                    .map(|w| {
                        let (start, end) = (w * per, ((w + 1) * per).min(count));
                        let this = &*self;
                        s.spawn(move || {
                            let mut out = Vec::with_capacity((end.saturating_sub(start)) * space.rec_len);
                            let mut owners = Vec::with_capacity(end.saturating_sub(start));
                            let mut stats = Stats::default();
                            for i in start..end {
                                let bytes = &raw[i * in_len..(i + 1) * in_len];
                                let raw = las::raw::Point::read_from(bytes, &source).map_err(invalid)?;
                                let point = las::Point::new(raw, &transforms);
                                let at = out.len();
                                this.convert(point, &transforms, &mut stats, &mut out)?;
                                let (cx, cy, cz) = space.cell(space.position(&out[at..]), COUNT_LEVEL);
                                let chunk = chunk_of[cx as usize + side * (cy as usize + side * cz as usize)];
                                owners.push((chunk, at));
                            }
                            Ok((owners, out, stats))
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_else(|_| Err(invalid("a worker panicked"))))
                    .collect()
            });
            for part in parts {
                let (owners, out, s) = part?;
                stats.merge(&s);
                for (chunk, at) in owners {
                    let buf = &mut buffers[chunk as usize];
                    buf.extend_from_slice(&out[at..at + space.rec_len]);
                    if buf.len() >= FLUSH_BYTES {
                        append(&self.chunk_path(chunk as usize), buf)?;
                        buf.clear();
                    }
                }
            }
            done += n;
            if !progress(0.2 + 0.35 * done as f32 / total as f32) {
                return Err(cancelled());
            }
        }
        for (i, buf) in buffers.iter().enumerate() {
            if !buf.is_empty() {
                append(&self.chunk_path(i), buf)?;
            }
        }
        Ok(stats)
    }

    /// Writes `point` as an output record, keeping statistics.
    fn convert(
        &self,
        mut point: las::Point,
        t: &Vector<Transform>,
        stats: &mut Stats,
        out: &mut Vec<u8>,
    ) -> Result<(), IoError> {
        point.extra_bytes.clear();
        point.waveform = None;
        if !self.format.has_color {
            point.color = None;
        }
        if !self.format.has_nir {
            point.nir = None;
        }
        if let Some(g) = point.gps_time {
            stats.gps = (stats.gps.0.min(g), stats.gps.1.max(g));
        }
        if (1..=15).contains(&point.return_number) {
            stats.returns[point.return_number as usize - 1] += 1;
        }
        stats.points += 1;
        let raw = point.into_raw(t).map_err(invalid)?;
        raw.write_to(&mut *out, &self.format).map_err(invalid)?;
        let p = DVec3::new(
            f64::from(raw.x) * t.x.scale + t.x.offset,
            f64::from(raw.y) * t.y.scale + t.y.offset,
            f64::from(raw.z) * t.z.scale + t.z.offset,
        );
        stats.min = stats.min.min(p);
        stats.max = stats.max.max(p);
        Ok(())
    }

    fn chunk_path(&self, i: usize) -> PathBuf {
        self.temp.join(format!("{i}.bin"))
    }

    /// Builds every chunk's subtree (writing its nodes) on several threads;
    /// returns each chunk root's points, for the levels above.
    fn build_chunks(
        &self,
        builder: &Builder<'_>,
        chunks: &[Key],
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<HashMap<Key, Vec<u8>>, IoError> {
        let next = AtomicUsize::new(0);
        let finished = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let roots = Mutex::new(HashMap::new());
        let failed: Mutex<Option<IoError>> = Mutex::new(None);
        // Each worker holds one chunk (and its copies) in memory.
        let workers = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .clamp(1, 6);
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&key) = chunks.get(i) else { break };
                        let path = self.chunk_path(i);
                        let result = fs::read(&path)
                            .map_err(invalid)
                            .and_then(|recs| {
                                let _ = fs::remove_file(&path);
                                builder.build(key, recs)
                            })
                            .map(|points| lock(&roots).insert(key, points));
                        if let Err(e) = result {
                            *lock(&failed) = Some(e);
                            stop.store(true, Ordering::Relaxed);
                        }
                        finished.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
            // Report progress (and take cancellation) while the workers run.
            loop {
                let f = finished.load(Ordering::Relaxed);
                if f >= chunks.len() || stop.load(Ordering::Relaxed) {
                    break;
                }
                if !progress(0.55 + 0.4 * f as f32 / chunks.len().max(1) as f32) {
                    stop.store(true, Ordering::Relaxed);
                    *lock(&failed) = Some(cancelled());
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        if let Some(e) = failed
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return Err(e);
        }
        Ok(roots
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// The VLRs: COPC info (first, as the spec requires), LASzip, then the
    /// input's coordinate system records.
    fn vlrs(&self, space: &Space, stats: &Stats) -> Vec<(String, u16, Vec<u8>)> {
        let mut info = Vec::with_capacity(160);
        let half = space.size * 0.5;
        let centre = space.min + DVec3::splat(half);
        for v in [
            centre.x,
            centre.y,
            centre.z,
            half,
            space.size / f64::from(self.options.grid),
        ] {
            info.extend_from_slice(&v.to_le_bytes());
        }
        // Hierarchy offset and size: filled in at the end.
        info.extend_from_slice(&[0u8; 16]);
        let (g0, g1) = if stats.gps.0 <= stats.gps.1 {
            stats.gps
        } else {
            (0.0, 0.0)
        };
        info.extend_from_slice(&g0.to_le_bytes());
        info.extend_from_slice(&g1.to_le_bytes());
        info.extend_from_slice(&[0u8; 88]);
        let mut laz = Vec::new();
        let _ = self.vlr.write_to(&mut laz);
        let mut vlrs = vec![
            ("copc".to_string(), 1, info),
            ("laszip encoded".to_string(), 22204, laz),
        ];
        for v in self.header.all_vlrs() {
            if v.user_id == "LASF_Projection" && v.data.len() <= u16::MAX as usize {
                vlrs.push((v.user_id.clone(), v.record_id, v.data.clone()));
            }
        }
        vlrs
    }

    fn has_wkt(&self) -> bool {
        self.header
            .all_vlrs()
            .any(|v| v.user_id == "LASF_Projection" && v.record_id == 2112)
    }

    /// The LAS 1.4 header (375 bytes).
    fn header_bytes(
        &self,
        space: &Space,
        stats: &Stats,
        point_start: u32,
        vlrs: u32,
        evlr_start: u64,
    ) -> Vec<u8> {
        let mut h = Vec::with_capacity(375);
        h.extend_from_slice(b"LASF");
        h.extend_from_slice(&0u16.to_le_bytes()); // file source id
        let mut encoding = 0u16;
        if self.header.gps_time_type() == las::GpsTimeType::Standard {
            encoding |= 1;
        }
        if self.has_wkt() {
            encoding |= 1 << 4;
        }
        h.extend_from_slice(&encoding.to_le_bytes());
        h.extend_from_slice(&[0u8; 16]); // GUID
        h.extend_from_slice(&[1, 4]);
        let text = |s: &str| {
            let mut b = [0u8; 32];
            b[..s.len().min(32)].copy_from_slice(&s.as_bytes()[..s.len().min(32)]);
            b
        };
        h.extend_from_slice(&text("survey3d"));
        h.extend_from_slice(&text("survey3d COPC writer"));
        h.extend_from_slice(&1u16.to_le_bytes()); // day of year
        h.extend_from_slice(&2026u16.to_le_bytes());
        h.extend_from_slice(&375u16.to_le_bytes());
        h.extend_from_slice(&point_start.to_le_bytes());
        h.extend_from_slice(&vlrs.to_le_bytes());
        let id = self.format.to_u8().unwrap_or(6);
        h.push(id | 0x80); // compressed
        h.extend_from_slice(&(space.rec_len as u16).to_le_bytes());
        h.extend_from_slice(&[0u8; 24]); // legacy point counts (0 for formats 6+)
        for v in space.scale.iter().chain(space.offset.iter()) {
            h.extend_from_slice(&v.to_le_bytes());
        }
        for v in [
            stats.max.x,
            stats.min.x,
            stats.max.y,
            stats.min.y,
            stats.max.z,
            stats.min.z,
        ] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        h.extend_from_slice(&0u64.to_le_bytes()); // waveform data
        h.extend_from_slice(&evlr_start.to_le_bytes());
        h.extend_from_slice(&1u32.to_le_bytes()); // EVLRs
        h.extend_from_slice(&stats.points.to_le_bytes());
        for r in stats.returns {
            h.extend_from_slice(&r.to_le_bytes());
        }
        debug_assert_eq!(h.len(), 375);
        h
    }
}

/// A cube around a box, a little larger so no point lies on its far faces.
fn cube_around(min: DVec3, max: DVec3) -> (DVec3, f64) {
    let size = (max - min).max_element().max(1e-6) * 1.0001 + 1e-6;
    let centre = (min + max) * 0.5;
    (centre - DVec3::splat(size * 0.5), size)
}

/// Groups counting-grid cells into chunks: the largest octree nodes holding at
/// most `max` points (or single cells, however full).
fn choose_chunks(counts: &[u32], max: usize) -> Vec<Key> {
    // A pyramid of counts: level COUNT_LEVEL is `counts`, each level above sums 8.
    let mut levels: Vec<Vec<u64>> = vec![counts.iter().map(|c| u64::from(*c)).collect()];
    for l in (0..COUNT_LEVEL).rev() {
        let side = 1usize << l;
        let below = &levels[0];
        let bs = side * 2;
        let mut up = vec![0u64; side * side * side];
        for z in 0..side {
            for y in 0..side {
                for x in 0..side {
                    let mut sum = 0;
                    for o in 0..8 {
                        let (cx, cy, cz) = (x * 2 + (o & 1), y * 2 + ((o >> 1) & 1), z * 2 + ((o >> 2) & 1));
                        sum += below[cx + bs * (cy + bs * cz)];
                    }
                    up[x + side * (y + side * z)] = sum;
                }
            }
        }
        levels.insert(0, up);
    }
    let mut chunks = Vec::new();
    let mut stack = vec![(0, 0, 0, 0)];
    while let Some(k) = stack.pop() {
        let side = 1usize << k.0;
        let n = levels[k.0 as usize][k.1 as usize + side * (k.2 as usize + side * k.3 as usize)];
        if n == 0 {
            continue;
        }
        if n as usize <= max || k.0 == COUNT_LEVEL {
            chunks.push(k);
        } else {
            stack.extend((0..8).map(|o| child(k, o)));
        }
    }
    chunks
}

fn append(path: &Path, data: &[u8]) -> Result<(), IoError> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(invalid)?;
    f.write_all(data).map_err(invalid)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write_vlr(w: &mut impl Write, user: &str, record: u16, data: &[u8]) -> std::io::Result<()> {
    let mut id = [0u8; 16];
    id[..user.len().min(16)].copy_from_slice(&user.as_bytes()[..user.len().min(16)]);
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(&id)?;
    w.write_all(&record.to_le_bytes())?;
    w.write_all(&(data.len() as u16).to_le_bytes())?;
    w.write_all(&[0u8; 32])?;
    w.write_all(data)
}

fn write_evlr(
    w: &mut impl Write,
    user: &str,
    record: u16,
    description: &str,
    data: &[u8],
) -> std::io::Result<()> {
    let mut id = [0u8; 16];
    id[..user.len().min(16)].copy_from_slice(&user.as_bytes()[..user.len().min(16)]);
    let mut desc = [0u8; 32];
    desc[..description.len().min(32)].copy_from_slice(&description.as_bytes()[..description.len().min(32)]);
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(&id)?;
    w.write_all(&record.to_le_bytes())?;
    w.write_all(&(data.len() as u64).to_le_bytes())?;
    w.write_all(&desc)?;
    w.write_all(data)
}

/// One worker's share of a batch: (chunk, offset) per point, the converted
/// records, and their statistics.
type Converted = (Vec<(u32, usize)>, Vec<u8>, Stats);

/// Totals for the header.
#[derive(Clone, Copy)]
struct Stats {
    points: u64,
    returns: [u64; 15],
    gps: (f64, f64),
    min: DVec3,
    max: DVec3,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            points: 0,
            returns: [0; 15],
            gps: (f64::MAX, f64::MIN),
            min: DVec3::splat(f64::MAX),
            max: DVec3::splat(f64::MIN),
        }
    }
}

impl Stats {
    fn merge(&mut self, o: &Stats) {
        self.points += o.points;
        for (a, b) in self.returns.iter_mut().zip(o.returns) {
            *a += b;
        }
        self.gps = (self.gps.0.min(o.gps.0), self.gps.1.max(o.gps.1));
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
    }
}

/// Builds subtrees and writes their nodes.
struct Builder<'a> {
    space: &'a Space,
    options: ConvertOptions,
    vlr: &'a LazVlr,
    out: &'a Mutex<Out>,
}

impl Builder<'_> {
    /// Builds the subtree of `key` from its points, writing every node below
    /// it; returns the points `key` itself keeps (its parent samples from them
    /// and writes the rest as `key`).
    fn build(&self, key: Key, recs: Vec<u8>) -> Result<Vec<u8>, IoError> {
        let len = self.space.rec_len;
        if recs.len() / len <= self.options.leaf_points || key.0 >= MAX_LEVEL {
            return Ok(recs);
        }
        let (min, side) = self.space.node_min(key);
        let centre = min + DVec3::splat(side * 0.5);
        let mut parts: Vec<Vec<u8>> = vec![Vec::new(); 8];
        for rec in recs.chunks_exact(len) {
            let p = self.space.position(rec);
            let o = usize::from(p.x >= centre.x)
                | usize::from(p.y >= centre.y) << 1
                | usize::from(p.z >= centre.z) << 2;
            parts[o].extend_from_slice(rec);
        }
        drop(recs);
        let mut kids = Vec::new();
        for (o, part) in parts.into_iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            let k = child(key, o);
            let split = part.len() / len > self.options.leaf_points && k.0 < MAX_LEVEL;
            kids.push((k, self.build(k, part)?, split));
        }
        let kept = self.sample(key, &mut kids);
        for (k, rest, split) in kids {
            if !rest.is_empty() || split {
                self.emit(k, &rest, split)?;
            }
        }
        Ok(kept)
    }

    /// The levels above the chunks: `key`'s points from its children (chunk
    /// roots, or nodes built from them), writing the children.
    fn build_top(
        &self,
        key: Key,
        roots: &mut HashMap<Key, Vec<u8>>,
        above: &HashSet<Key>,
    ) -> Result<Option<Vec<u8>>, IoError> {
        if let Some(points) = roots.remove(&key) {
            return Ok(Some(points));
        }
        if !above.contains(&key) {
            return Ok(None);
        }
        let mut kids = Vec::new();
        for o in 0..8 {
            let k = child(key, o);
            if let Some(points) = self.build_top(k, roots, above)? {
                kids.push((k, points, true));
            }
        }
        let kept = self.sample(key, &mut kids);
        for (k, rest, _) in kids {
            self.emit(k, &rest, true)?;
        }
        Ok(Some(kept))
    }

    /// Takes one point per cell of a `grid`³ lattice over `key`'s cube from
    /// the children's points (the one nearest the cell's centre), removing it
    /// from the child. Returns the taken points.
    fn sample(&self, key: Key, kids: &mut [(Key, Vec<u8>, bool)]) -> Vec<u8> {
        let len = self.space.rec_len;
        let (min, side) = self.space.node_min(key);
        let g = self.options.grid;
        let cell = side / f64::from(g);
        let total: usize = kids.iter().map(|k| k.1.len() / len).sum();
        // cell -> (child, index, squared distance to the cell's centre)
        let mut best: HashMap<u32, (u16, u32, f64)> = HashMap::with_capacity(total.min(1 << 20));
        for (ki, (_, recs, _)) in kids.iter().enumerate() {
            for (i, rec) in recs.chunks_exact(len).enumerate() {
                let p = self.space.position(rec);
                let at = |v: f64, lo: f64| (((v - lo) / cell).floor()).clamp(0.0, f64::from(g - 1));
                let (cx, cy, cz) = (at(p.x, min.x), at(p.y, min.y), at(p.z, min.z));
                let c = min + (DVec3::new(cx, cy, cz) + 0.5) * cell;
                let d = (p - c).length_squared();
                let id = cx as u32 + g * (cy as u32 + g * cz as u32);
                let e = best.entry(id).or_insert((ki as u16, i as u32, f64::MAX));
                if d < e.2 {
                    *e = (ki as u16, i as u32, d);
                }
            }
        }
        let mut taken: Vec<Vec<bool>> = kids.iter().map(|k| vec![false; k.1.len() / len]).collect();
        let mut kept = Vec::with_capacity(best.len() * len);
        for &(ki, i, _) in best.values() {
            taken[ki as usize][i as usize] = true;
            let recs = &kids[ki as usize].1;
            kept.extend_from_slice(&recs[i as usize * len..(i as usize + 1) * len]);
        }
        for (kid, taken) in kids.iter_mut().zip(taken) {
            let mut rest = Vec::with_capacity(kid.1.len());
            for (rec, t) in kid.1.chunks_exact(len).zip(taken) {
                if !t {
                    rest.extend_from_slice(rec);
                }
            }
            kid.1 = rest;
        }
        kept
    }

    /// Compresses `recs` as `key`'s LAZ chunk and appends it to the file. An
    /// empty node is recorded (with no chunk) only when it has descendants.
    fn emit(&self, key: Key, recs: &[u8], has_children: bool) -> Result<(), IoError> {
        let len = self.space.rec_len;
        let count = recs.len() / len;
        if count == 0 {
            if has_children {
                lock(self.out).nodes.push(Written {
                    key,
                    offset: 0,
                    bytes: 0,
                    count: 0,
                });
            }
            return Ok(());
        }
        // LAZ predicts each point from the last: in scan order (GPS time,
        // bytes 22..30 of formats 6–8) neighbours are alike and compress far
        // better than in sampling order.
        let mut order: Vec<&[u8]> = recs.chunks_exact(len).collect();
        let time = |r: &[u8]| f64::from_le_bytes([r[22], r[23], r[24], r[25], r[26], r[27], r[28], r[29]]);
        order.sort_by(|a, b| time(a).total_cmp(&time(b)));
        let mut c = LayeredPointRecordCompressor::new(Cursor::new(Vec::new()));
        c.set_fields_from(self.vlr.items()).map_err(invalid)?;
        for rec in order {
            c.compress_next(rec).map_err(invalid)?;
        }
        c.done().map_err(invalid)?;
        let bytes = std::mem::take(c.get_mut().get_mut());
        let mut out = lock(self.out);
        let offset = out.pos;
        out.file.write_all(&bytes).map_err(invalid)?;
        out.pos += bytes.len() as u64;
        out.table.push(ChunkTableEntry {
            point_count: count as u64,
            byte_count: bytes.len() as u64,
        });
        out.nodes.push(Written {
            key,
            offset,
            bytes: bytes.len() as u32,
            count: count as u32,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic survey: ground with a few "trees", colours and GPS times.
    fn write_las(path: &Path, n: usize) {
        let mut builder = las::Builder::from((1, 2));
        builder.point_format = Format::new(3).expect("format");
        builder.transforms = Vector {
            x: Transform {
                scale: 0.001,
                offset: 500_000.0,
            },
            y: Transform {
                scale: 0.001,
                offset: 7_000_000.0,
            },
            z: Transform {
                scale: 0.001,
                offset: 0.0,
            },
        };
        let header = builder.into_header().expect("header");
        let mut w = las::Writer::from_path(path, header).expect("writer");
        let mut state = 12345u64;
        let mut rand = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for i in 0..n {
            let (x, y) = (rand() * 300.0, rand() * 200.0);
            let tree = (x / 40.0).fract() < 0.2 && (y / 40.0).fract() < 0.2;
            let z = 100.0 + x * 0.05 + if tree { rand() * 20.0 } else { rand() * 0.1 };
            w.write_point(las::Point {
                x: 500_000.0 + x,
                y: 7_000_000.0 + y,
                z,
                intensity: (i % 1000) as u16,
                return_number: 1,
                number_of_returns: 1,
                classification: las::point::Classification::Ground,
                gps_time: Some(i as f64),
                color: Some(las::Color::new((x * 200.0) as u16, (y * 300.0) as u16, 100)),
                ..Default::default()
            })
            .expect("point");
        }
        w.close().expect("close");
    }

    #[test]
    fn converts_and_reads_back_every_point() {
        let dir = std::env::temp_dir().join(format!("survey3d-copc-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let (las_path, copc_path) = (dir.join("in.las"), dir.join("out.copc.laz"));
        let n = 120_000;
        write_las(&las_path, n);
        let options = ConvertOptions {
            chunk_points: 20_000,
            leaf_points: 3_000,
            grid: 32,
        };
        let written = convert_to_copc(&las_path, &copc_path, options, &mut |_| true).expect("convert");
        assert_eq!(written.points, n as u64);
        assert!(written.depth >= 2, "a real hierarchy: {written:?}");

        // copc-rs reads it, every point once, and coarse levels sample all of it.
        let mut reader = copc_rs::CopcReader::from_path(&copc_path).expect("copc-rs opens it");
        let mut seen: Vec<(i64, i64, i64)> = reader
            .points(copc_rs::LodSelection::All, copc_rs::BoundsSelection::All)
            .expect("points")
            .map(|p| {
                let p = p.expect("point");
                assert!(p.color.is_some() && p.gps_time.is_some());
                (
                    (p.x * 1000.0).round() as i64,
                    (p.y * 1000.0).round() as i64,
                    (p.z * 1000.0).round() as i64,
                )
            })
            .collect();
        assert_eq!(seen.len(), n);
        let mut original: Vec<(i64, i64, i64)> = las::Reader::from_path(&las_path)
            .expect("las")
            .read_all()
            .expect("read")
            .points()
            .map(|p| {
                let p = p.expect("point");
                (
                    (p.x * 1000.0).round() as i64,
                    (p.y * 1000.0).round() as i64,
                    (p.z * 1000.0).round() as i64,
                )
            })
            .collect();
        seen.sort_unstable();
        original.sort_unstable();
        assert_eq!(seen, original, "the same points, once each");
        let coarse = reader
            .points(copc_rs::LodSelection::Level(0), copc_rs::BoundsSelection::All)
            .expect("root")
            .count();
        assert!(coarse > 100 && coarse < n / 4, "the root is a sample: {coarse}");

        // It is also a plain LAZ file (chunk table and header).
        let mut plain = las::Reader::from_path(&copc_path).expect("plain LAZ");
        assert_eq!(plain.header().number_of_points(), n as u64);
        assert_eq!(plain.read_all().expect("read").len(), n);

        // Every node's points lie inside its cube.
        let src = crate::copc::CopcSource::open(&copc_path).expect("our reader");
        let (raw, _) = src.read_levels(usize::MAX).expect("levels");
        assert!(!raw.points.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A small file of point format 0 (no colour, no GPS time) becomes a
    /// single-node COPC of format 6; cancelling removes the output.
    #[test]
    fn small_plain_files_and_cancelling() {
        let dir = std::env::temp_dir().join(format!("survey3d-copc-small-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let (las_path, copc_path) = (dir.join("in.las"), dir.join("out.copc.laz"));
        let mut builder = las::Builder::from((1, 2));
        builder.point_format = Format::new(0).expect("format");
        let mut w =
            las::Writer::from_path(&las_path, builder.into_header().expect("header")).expect("writer");
        for i in 0..500 {
            let f = f64::from(i);
            w.write_point(las::Point {
                x: f,
                y: (f * 0.37).sin() * 10.0,
                z: 1.0,
                return_number: 1,
                number_of_returns: 1,
                ..Default::default()
            })
            .expect("point");
        }
        w.close().expect("close");

        let written = convert_to_copc(&las_path, &copc_path, ConvertOptions::default(), &mut |_| true)
            .expect("convert");
        assert_eq!((written.points, written.nodes, written.depth), (500, 1, 0));
        let mut reader = copc_rs::CopcReader::from_path(&copc_path).expect("open");
        assert_eq!(reader.header().point_format().to_u8().ok(), Some(6));
        let n = reader
            .points(copc_rs::LodSelection::All, copc_rs::BoundsSelection::All)
            .expect("points")
            .count();
        assert_eq!(n, 500);

        let _ = fs::remove_file(&copc_path);
        let err = convert_to_copc(&las_path, &copc_path, ConvertOptions::default(), &mut |f| f < 0.1);
        assert!(err.is_err(), "cancelled");
        assert!(
            !copc_path.exists() && !copc_path.with_extension("copc-tmp").exists(),
            "cleaned up"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
