//! Georeferenced imagery: reading GeoTIFF (tiled or stripped, BigTIFF, LZW,
//! Deflate or JPEG; with an alpha band or GDAL no-data) and JPEG or PNG with
//! a world file, and building a tile pyramid cache so any size of image can
//! be drawn by reading only the tiles a view needs.
//!
//! The cache is an SQLite file of 256-pixel tiles at every power-of-two
//! level (level 0 is full resolution); opaque tiles are JPEG, partly
//! transparent ones PNG, and empty ones are not stored. Small images build
//! the same cache in memory.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};

use glam::DVec2;
use rusqlite::{Connection, OptionalExtension, params};
use survey_core::georef::ImageTransform;
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

use crate::IoError;

/// Tile size in pixels.
pub const TILE: u32 = 256;

/// Image files read (with a world file for JPEG and PNG).
pub const EXTENSIONS: [&str; 5] = ["tif", "tiff", "jpg", "jpeg", "png"];

/// Largest JPEG or PNG read: they are decoded whole (GeoTIFF is read in pieces).
const MAX_DECODED_PIXELS: u64 = 400_000_000;

/// Cache layout version: a cache written by another version is rebuilt.
const CACHE_VERSION: i64 = 1;

fn invalid(e: impl std::fmt::Display) -> IoError {
    IoError::Invalid(e.to_string())
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// What an image file holds and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub width: u32,
    pub height: u32,
    /// From GeoTIFF tags or a world file, when either is present.
    pub transform: Option<ImageTransform>,
    /// "GeoTIFF" or the world file's name.
    pub placed_by: Option<String>,
    /// The GeoTIFF's projected (or geographic) EPSG code.
    pub epsg: Option<u32>,
    /// The GeoTIFF is in latitude and longitude (degrees), not a grid.
    pub geographic: bool,
}

/// The world file beside an image, if there is one (`photo.jgw`,
/// `photo.jpgw`, `photo.wld` and the like, any case).
pub fn world_file_for(path: &Path) -> Option<PathBuf> {
    let ext = extension(path);
    let short = match ext.as_str() {
        "tif" | "tiff" => "tfw",
        "jpg" | "jpeg" => "jgw",
        "png" => "pgw",
        _ => "wld",
    };
    let candidates = [short.to_string(), format!("{ext}w"), "wld".into()];
    let dir = path.parent()?;
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.to_ascii_lowercase() == stem)
                && candidates.contains(&extension(p))
        })
}

/// Reads an image's size and placement without decoding it.
pub fn inspect(path: &Path) -> Result<SourceInfo, IoError> {
    let ext = extension(path);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    if !path.is_file() && EXTENSIONS.contains(&ext.as_str()) {
        return Err(IoError::Invalid(format!("{} was not found", path.display())));
    }
    let mut info = match ext.as_str() {
        "tif" | "tiff" => {
            let mut d = tiff_decoder(path)
                .map_err(|e| IoError::Invalid(format!("{name} is not a readable TIFF ({e})")))?;
            let (width, height) = d.dimensions().map_err(invalid)?;
            // Imagery is 8- or 16-bit unsigned: floats and wider samples are
            // elevation models or other data rasters.
            let format = d
                .find_tag_unsigned::<u16>(Tag::SampleFormat)
                .ok()
                .flatten()
                .unwrap_or(1);
            let bits = d.colortype().map(|c| c.bit_depth()).unwrap_or(8);
            if format != 1 || bits > 16 {
                return Err(IoError::Invalid(format!(
                    "{name} holds {} data, not an image (an elevation model?): imagery needs 8- or 16-bit RGB or grey",
                    match format {
                        3 => "floating-point",
                        2 => "signed",
                        _ => "32-bit or wider",
                    }
                )));
            }
            let geo = geotiff_placement(&mut d);
            SourceInfo {
                width,
                height,
                transform: geo.0,
                placed_by: geo.0.map(|_| "GeoTIFF".to_string()),
                epsg: geo.1,
                geographic: geo.2,
            }
        }
        "jpg" | "jpeg" | "png" => {
            let (width, height) = image::image_dimensions(path)
                .map_err(|e| IoError::Invalid(format!("{name} is not a readable image ({e})")))?;
            SourceInfo {
                width,
                height,
                transform: None,
                placed_by: None,
                epsg: None,
                geographic: false,
            }
        }
        _ => {
            return Err(IoError::Invalid(format!(
                "'.{ext}' images are not read (GeoTIFF, JPEG or PNG; convert ECW or JPEG 2000 with GDAL)"
            )));
        }
    };
    // A world file places images without tags, and a GeoTIFF in latitude and
    // longitude (likely written from a grid placement, as by save world
    // file); it never overrides a grid GeoTIFF's own tags.
    if (info.transform.is_none() || info.geographic)
        && let Some(wf) = world_file_for(path)
    {
        let text = std::fs::read_to_string(&wf).map_err(invalid)?;
        let name = wf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        info.transform = Some(
            ImageTransform::from_world_file(&text).map_err(|e| IoError::Invalid(format!("{name}: {e}")))?,
        );
        info.placed_by = Some(name);
        // The placement is the world file's, which says nothing of a coordinate system.
        if info.geographic {
            info.epsg = None;
        }
        info.geographic = false;
    }
    Ok(info)
}

fn tiff_decoder(path: &Path) -> Result<Decoder<BufReader<File>>, IoError> {
    let file = File::open(path).map_err(invalid)?;
    Ok(Decoder::new(BufReader::new(file))
        .map_err(invalid)?
        .with_limits(Limits::unlimited()))
}

/// The transform, EPSG code and whether it is geographic, from GeoTIFF tags.
fn geotiff_placement<R: std::io::Read + std::io::Seek>(
    d: &mut Decoder<R>,
) -> (Option<ImageTransform>, Option<u32>, bool) {
    let keys = d
        .find_tag_unsigned_vec::<u16>(Tag::Unknown(34735))
        .ok()
        .flatten()
        .unwrap_or_default();
    let key = |id: u16| {
        keys.as_chunks::<4>()
            .0
            .iter()
            .skip(1)
            .find(|k| k[0] == id)
            .filter(|k| k[1] == 0)
            .map(|k| k[3])
    };
    // GTRasterTypeGeoKey: 1 PixelIsArea (the default), 2 PixelIsPoint.
    let area = key(1025) != Some(2);
    // GTModelTypeGeoKey 2 is geographic.
    let geographic = key(1024) == Some(2);
    let epsg = key(3072)
        .or(key(2048))
        .filter(|c| *c > 1 && *c < 32767)
        .map(u32::from);
    let matrix = d.get_tag_f64_vec(Tag::Unknown(34264)).ok();
    let transform = match matrix {
        Some(m) => ImageTransform::from_matrix(&m, area),
        None => match (
            d.get_tag_f64_vec(Tag::Unknown(33922)),
            d.get_tag_f64_vec(Tag::Unknown(33550)),
        ) {
            (Ok(tie), Ok(scale)) if tie.len() >= 6 && scale.len() >= 2 => Some(
                ImageTransform::from_tie_point([tie[0], tie[1], tie[3], tie[4]], [scale[0], scale[1]], area),
            ),
            _ => None,
        },
    };
    (transform, epsg, geographic)
}

// ====================================================================== reading rows

/// Full-width RGBA rows of the source, top to bottom.
trait Rows {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    /// Rows `y0..y0 + n` as RGBA.
    fn read(&mut self, y0: u32, n: u32) -> Result<Vec<u8>, IoError>;
}

/// A JPEG or PNG decoded whole.
struct Decoded(image::RgbaImage);

impl Rows for Decoded {
    fn width(&self) -> u32 {
        self.0.width()
    }

    fn height(&self) -> u32 {
        self.0.height()
    }

    fn read(&mut self, y0: u32, n: u32) -> Result<Vec<u8>, IoError> {
        let row = self.0.width() as usize * 4;
        let start = y0 as usize * row;
        Ok(self.0.as_raw()[start..start + n as usize * row].to_vec())
    }
}

/// A decoded chunk: RGBA and its width and height.
type Chunk = (Vec<u8>, u32, u32);

/// A GeoTIFF read chunk by chunk (strips or tiles), one row of chunks held.
struct TiffRows {
    d: Decoder<BufReader<File>>,
    width: u32,
    height: u32,
    chunk: (u32, u32),
    across: u32,
    color: tiff::ColorType,
    nodata: Option<f64>,
    /// The chunk row held: its index and each chunk.
    held: Option<(u32, Vec<Chunk>)>,
}

impl TiffRows {
    fn open(path: &Path) -> Result<Self, IoError> {
        let mut d = tiff_decoder(path)?;
        let (width, height) = d.dimensions().map_err(invalid)?;
        let color = d.colortype().map_err(invalid)?;
        let planar = d
            .find_tag_unsigned::<u16>(Tag::PlanarConfiguration)
            .ok()
            .flatten()
            .unwrap_or(1);
        if planar != 1 {
            return Err(IoError::Invalid(
                "band-separate (planar) TIFFs are not read: convert with gdal_translate -co INTERLEAVE=PIXEL"
                    .into(),
            ));
        }
        let nodata = d
            .get_tag_ascii_string(Tag::Unknown(42113))
            .ok()
            .and_then(|s| s.trim_matches(char::from(0)).trim().parse::<f64>().ok());
        let chunk = d.chunk_dimensions();
        Ok(Self {
            d,
            width,
            height,
            chunk,
            across: width.div_ceil(chunk.0),
            color,
            nodata,
            held: None,
        })
    }

    /// One chunk as RGBA, with its data size.
    fn chunk_rgba(&mut self, index: u32) -> Result<(Vec<u8>, u32, u32), IoError> {
        let (w, h) = self.d.chunk_data_dimensions(index);
        let data = self.d.read_chunk(index).map_err(invalid)?;
        let samples: Vec<f64> = match data {
            DecodingResult::U8(v) => v.into_iter().map(f64::from).collect(),
            DecodingResult::U16(v) => v.into_iter().map(|x| f64::from(x >> 8)).collect(),
            _ => {
                return Err(IoError::Invalid(
                    "the TIFF is not an 8- or 16-bit image (an elevation model?): imagery needs RGB or grey"
                        .into(),
                ));
            }
        };
        let raw_nodata = self.nodata.map(|n| {
            // No-data is in the file's own units; 16-bit samples were scaled down.
            if self.color.bit_depth() == 16 {
                n / 256.0
            } else {
                n
            }
        });
        let spp = match self.color {
            tiff::ColorType::Gray(_) => 1,
            tiff::ColorType::GrayA(_) => 2,
            tiff::ColorType::RGB(_) | tiff::ColorType::YCbCr(_) => 3,
            tiff::ColorType::RGBA(_) => 4,
            tiff::ColorType::Multiband { num_samples, .. } => usize::from(num_samples),
            other => {
                return Err(IoError::Invalid(format!(
                    "{other:?} TIFFs are not read: convert to RGB with GDAL"
                )));
            }
        };
        // Samples per row: a padded tile's rows are the tile width long.
        let total = samples.len() / spp.max(1);
        let row_len = if h > 0 { total / h as usize } else { 0 };
        let mut out = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let s = &samples[(y * row_len + x) * spp..][..spp];
                let (rgb, alpha) = match spp {
                    1 => ([s[0]; 3], 255.0),
                    2 => ([s[0]; 3], s[1]),
                    3 => ([s[0], s[1], s[2]], 255.0),
                    // RGBA; four or more bands with no alpha (RGB + near infrared) read as RGB.
                    _ if matches!(self.color, tiff::ColorType::RGBA(_)) => ([s[0], s[1], s[2]], s[3]),
                    _ => ([s[0], s[1], s[2]], 255.0),
                };
                let blank = raw_nodata.is_some_and(|n| rgb.iter().all(|v| (*v - n).abs() < 0.5));
                // JPEG-compressed TIFFs (most COGs) decode to YCbCr (JFIF, full range).
                let rgb = if matches!(self.color, tiff::ColorType::YCbCr(_)) {
                    let [y, cb, cr] = rgb;
                    [
                        y + 1.402 * (cr - 128.0),
                        y - 0.344_136 * (cb - 128.0) - 0.714_136 * (cr - 128.0),
                        y + 1.772 * (cb - 128.0),
                    ]
                } else {
                    rgb
                };
                let o = &mut out[(y * w as usize + x) * 4..][..4];
                o[0] = rgb[0].round().clamp(0.0, 255.0) as u8;
                o[1] = rgb[1].round().clamp(0.0, 255.0) as u8;
                o[2] = rgb[2].round().clamp(0.0, 255.0) as u8;
                o[3] = if blank { 0 } else { alpha.clamp(0.0, 255.0) as u8 };
            }
        }
        Ok((out, w, h))
    }
}

impl Rows for TiffRows {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn read(&mut self, y0: u32, n: u32) -> Result<Vec<u8>, IoError> {
        let row_bytes = self.width as usize * 4;
        let mut out = vec![0u8; row_bytes * n as usize];
        let mut y = y0;
        while y < y0 + n {
            let cy = y / self.chunk.1;
            if self.held.as_ref().is_none_or(|(held, _)| *held != cy) {
                let mut chunks = Vec::with_capacity(self.across as usize);
                for cx in 0..self.across {
                    chunks.push(self.chunk_rgba(cy * self.across + cx)?);
                }
                self.held = Some((cy, chunks));
            }
            let (_, chunks) = self.held.as_ref().expect("held");
            let row_in_chunk = y - cy * self.chunk.1;
            let rows_here = (self.chunk.1 - row_in_chunk).min(y0 + n - y);
            for (cx, (rgba, w, h)) in chunks.iter().enumerate() {
                let x0 = cx as u32 * self.chunk.0;
                let w = (*w).min(self.width - x0);
                for r in 0..rows_here {
                    let src_row = row_in_chunk + r;
                    if src_row >= *h {
                        continue;
                    }
                    let src = &rgba[(src_row as usize * w as usize) * 4..][..w as usize * 4];
                    let dst = ((y - y0 + r) as usize) * row_bytes + x0 as usize * 4;
                    out[dst..dst + w as usize * 4].copy_from_slice(src);
                }
            }
            y += rows_here;
        }
        Ok(out)
    }
}

fn open_rows(path: &Path) -> Result<Box<dyn Rows>, IoError> {
    match extension(path).as_str() {
        "tif" | "tiff" => Ok(Box::new(TiffRows::open(path)?)),
        _ => {
            let (w, h) = image::image_dimensions(path).map_err(invalid)?;
            if u64::from(w) * u64::from(h) > MAX_DECODED_PIXELS {
                return Err(IoError::Invalid(format!(
                    "a {w} × {h} JPEG or PNG is too large to read: convert it to a tiled GeoTIFF \
                     (gdal_translate -of COG)"
                )));
            }
            Ok(Box::new(Decoded(image::open(path).map_err(invalid)?.to_rgba8())))
        }
    }
}

// ====================================================================== the cache

/// Levels in the pyramid of a `width` × `height` image: down to one tile.
pub fn level_count(width: u32, height: u32) -> u32 {
    let mut levels = 1;
    let mut size = width.max(height);
    while size > TILE {
        size = size.div_ceil(2);
        levels += 1;
    }
    levels
}

/// A decoded tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A tile pyramid, on disk or in memory.
pub struct TileCache {
    conn: Connection,
    pub width: u32,
    pub height: u32,
    pub levels: u32,
}

impl std::fmt::Debug for TileCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TileCache({} × {}, {} levels)",
            self.width, self.height, self.levels
        )
    }
}

/// The source file's size and modification time, to tell a stale cache.
fn stamp(source: &Path) -> String {
    std::fs::metadata(source)
        .map(|m| {
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            format!("{}:{modified}", m.len())
        })
        .unwrap_or_default()
}

fn schema(conn: &Connection) -> Result<(), IoError> {
    conn.execute_batch(
        "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
         CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS tiles (level INTEGER, x INTEGER, y INTEGER, data BLOB NOT NULL,
                                           PRIMARY KEY (level, x, y));",
    )
    .map_err(invalid)
}

impl TileCache {
    /// Opens a finished cache built from `source` (as it is now); `None`
    /// when there is none or it is stale or incomplete.
    pub fn open(cache: &Path, source: &Path) -> Option<Self> {
        let conn = Connection::open(cache).ok()?;
        let meta = |key: &str| -> Option<String> {
            conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
                .optional()
                .ok()
                .flatten()
        };
        if meta("complete")? != "1"
            || meta("version")?.parse::<i64>().ok()? != CACHE_VERSION
            || meta("source_stamp")? != stamp(source)
        {
            return None;
        }
        let width = meta("width")?.parse().ok()?;
        let height = meta("height")?.parse().ok()?;
        Some(Self {
            width,
            height,
            levels: level_count(width, height),
            conn,
        })
    }

    /// Tiles across and down at `level`.
    pub fn tiles(&self, level: u32) -> (u32, u32) {
        let (w, h) = self.level_size(level);
        (w.div_ceil(TILE), h.div_ceil(TILE))
    }

    /// The image's size in pixels at `level`.
    pub fn level_size(&self, level: u32) -> (u32, u32) {
        let k = 1u32 << level.min(31);
        (self.width.div_ceil(k), self.height.div_ceil(k))
    }

    /// The tile at `level`, column `x`, row `y`; `None` where it is empty.
    pub fn tile(&self, level: u32, x: u32, y: u32) -> Result<Option<Tile>, IoError> {
        let data: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT data FROM tiles WHERE level = ?1 AND x = ?2 AND y = ?3",
                params![level, x, y],
                |r| r.get(0),
            )
            .optional()
            .map_err(invalid)?;
        let Some(data) = data else {
            return Ok(None);
        };
        let img = image::load_from_memory(&data).map_err(invalid)?.to_rgba8();
        Ok(Some(Tile {
            width: img.width(),
            height: img.height(),
            rgba: img.into_raw(),
        }))
    }
}

/// Encodes a tile: nothing when it is empty, JPEG when opaque, else PNG.
fn encode(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let alphas = rgba.iter().skip(3).step_by(4);
    if alphas.clone().all(|a| *a == 0) {
        return None;
    }
    let mut out = Cursor::new(Vec::new());
    if alphas.clone().all(|a| *a == 255) {
        let rgb: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
        image::ImageEncoder::write_image(enc, &rgb, w, h, image::ExtendedColorType::Rgb8).ok()?;
    } else {
        let enc = image::codecs::png::PngEncoder::new(&mut out);
        image::ImageEncoder::write_image(enc, rgba, w, h, image::ExtendedColorType::Rgba8).ok()?;
    }
    Some(out.into_inner())
}

/// Halves a band of RGBA rows (`w` × `h`), weighting colours by alpha.
fn halve(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (ow, oh) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = vec![0u8; ow as usize * oh as usize * 4];
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut c, mut a, mut n) = ([0u32; 3], 0u32, 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (x, y) = (2 * ox + dx, 2 * oy + dy);
                if x >= w || y >= h {
                    continue;
                }
                let p = &src[(y as usize * w as usize + x as usize) * 4..][..4];
                let pa = u32::from(p[3]);
                for k in 0..3 {
                    c[k] += u32::from(p[k]) * pa;
                }
                a += pa;
                n += 1;
            }
            let o = &mut out[(oy as usize * ow as usize + ox as usize) * 4..][..4];
            for k in 0..3 {
                o[k] = c[k].checked_div(a).unwrap_or(0) as u8;
            }
            o[3] = (a / n.max(1)) as u8;
        }
    }
    (out, ow, oh)
}

/// One level of the pyramid being built: rows waiting to make a row of tiles.
struct Level {
    width: u32,
    height: u32,
    buf: Vec<u8>,
    rows: u32,
    received: u32,
    tile_row: u32,
}

struct Builder<'a> {
    conn: &'a Connection,
    levels: Vec<Level>,
}

impl Builder<'_> {
    /// Adds `n` rows to level `k`, writing tile rows and passing halved bands up.
    fn push(&mut self, k: usize, rows: &[u8], n: u32) -> Result<(), IoError> {
        let lv = &mut self.levels[k];
        lv.buf.extend_from_slice(rows);
        lv.rows += n;
        lv.received += n;
        loop {
            let lv = &mut self.levels[k];
            let done = lv.received >= lv.height;
            if lv.rows < TILE && !(done && lv.rows > 0) {
                return Ok(());
            }
            let band_rows = lv.rows.min(TILE);
            let band_bytes = (band_rows * lv.width * 4) as usize;
            let band: Vec<u8> = lv.buf.drain(..band_bytes).collect();
            lv.rows -= band_rows;
            let (width, ty) = (lv.width, lv.tile_row);
            lv.tile_row += 1;
            self.write_row(k as u32, ty, &band, width, band_rows)?;
            if k + 1 < self.levels.len() {
                let (half, _, hh) = halve(&band, width, band_rows);
                self.push(k + 1, &half, hh)?;
            }
        }
    }

    /// Cuts a band into tiles, encodes them in parallel and stores them.
    fn write_row(&self, level: u32, ty: u32, band: &[u8], width: u32, rows: u32) -> Result<(), IoError> {
        let across = width.div_ceil(TILE);
        let tiles: Vec<(u32, Option<Vec<u8>>)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..across)
                .map(|tx| {
                    s.spawn(move || {
                        let x0 = tx * TILE;
                        let tw = TILE.min(width - x0);
                        let mut px = Vec::with_capacity((tw * rows * 4) as usize);
                        for r in 0..rows {
                            let start = ((r * width + x0) * 4) as usize;
                            px.extend_from_slice(&band[start..start + (tw * 4) as usize]);
                        }
                        (tx, encode(&px, tw, rows))
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut insert = self
            .conn
            .prepare_cached("INSERT OR REPLACE INTO tiles (level, x, y, data) VALUES (?1, ?2, ?3, ?4)")
            .map_err(invalid)?;
        for (tx, data) in tiles {
            if let Some(data) = data {
                insert.execute(params![level, tx, ty, data]).map_err(invalid)?;
            }
        }
        Ok(())
    }
}

/// Builds the tile pyramid of `source` into `cache` (`None` in memory).
/// `progress` is told the fraction done and cancels by returning false.
pub fn build_cache(
    source: &Path,
    cache: Option<&Path>,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<TileCache, IoError> {
    let mut rows = open_rows(source)?;
    let (width, height) = (rows.width(), rows.height());
    if width == 0 || height == 0 {
        return Err(IoError::Invalid("the image is empty".into()));
    }
    let partial = cache.map(|c| {
        let mut p = c.as_os_str().to_owned();
        p.push(".partial");
        PathBuf::from(p)
    });
    let conn = match &partial {
        Some(p) => {
            let _ = std::fs::remove_file(p);
            Connection::open(p).map_err(invalid)?
        }
        None => Connection::open_in_memory().map_err(invalid)?,
    };
    schema(&conn)?;
    let levels = level_count(width, height);
    let mut b = Builder {
        conn: &conn,
        levels: (0..levels)
            .map(|k| Level {
                width: width.div_ceil(1 << k),
                height: height.div_ceil(1 << k),
                buf: Vec::new(),
                rows: 0,
                received: 0,
                tile_row: 0,
            })
            .collect(),
    };
    let result = (|| {
        conn.execute_batch("BEGIN").map_err(invalid)?;
        let mut y = 0;
        while y < height {
            let n = TILE.min(height - y);
            let band = rows.read(y, n)?;
            b.push(0, &band, n)?;
            y += n;
            if !progress(y as f32 / height as f32) {
                return Err(IoError::Invalid("image cache cancelled".into()));
            }
        }
        for (k, v) in [
            ("version", CACHE_VERSION.to_string()),
            ("width", width.to_string()),
            ("height", height.to_string()),
            ("source", source.display().to_string()),
            ("source_stamp", stamp(source)),
            ("complete", "1".into()),
        ] {
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                params![k, v],
            )
            .map_err(invalid)?;
        }
        conn.execute_batch("COMMIT").map_err(invalid)
    })();
    if let Err(e) = result {
        drop(conn);
        if let Some(p) = &partial {
            let _ = std::fs::remove_file(p);
        }
        return Err(e);
    }
    let conn = match (cache, partial) {
        (Some(c), Some(p)) => {
            drop(conn);
            std::fs::rename(&p, c).map_err(invalid)?;
            Connection::open(c).map_err(invalid)?
        }
        _ => conn,
    };
    Ok(TileCache {
        conn,
        width,
        height,
        levels,
    })
}

/// Resamples the image into an `out_w` × `out_h` RGBA raster: `pixel_of`
/// gives the full-resolution image pixel under each output pixel's centre
/// (or `None` outside), read at `level` with bilinear interpolation.
pub fn resample(
    cache: &TileCache,
    level: u32,
    out_w: u32,
    out_h: u32,
    pixel_of: impl Fn(f64, f64) -> Option<DVec2>,
) -> Vec<u8> {
    let level = level.min(cache.levels - 1);
    let k = f64::from(1u32 << level);
    let (lw, lh) = cache.level_size(level);
    let mut tiles: HashMap<(u32, u32), Option<Tile>> = HashMap::new();
    let mut fetch = |x: i64, y: i64| -> [f64; 4] {
        if x < 0 || y < 0 || x >= i64::from(lw) || y >= i64::from(lh) {
            return [0.0; 4];
        }
        let (x, y) = (x as u32, y as u32);
        let key = (x / TILE, y / TILE);
        let tile = tiles
            .entry(key)
            .or_insert_with(|| cache.tile(level, key.0, key.1).ok().flatten());
        match tile {
            Some(t) => {
                let (tx, ty) = (x % TILE, y % TILE);
                if tx >= t.width || ty >= t.height {
                    return [0.0; 4];
                }
                let p = &t.rgba[((ty * t.width + tx) * 4) as usize..][..4];
                [f64::from(p[0]), f64::from(p[1]), f64::from(p[2]), f64::from(p[3])]
            }
            None => [0.0; 4],
        }
    };
    let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
    for oy in 0..out_h {
        for ox in 0..out_w {
            let Some(p) = pixel_of(f64::from(ox) + 0.5, f64::from(oy) + 0.5) else {
                continue;
            };
            // Level pixel centres are at +0.5.
            let (fx, fy) = (p.x / k - 0.5, p.y / k - 0.5);
            let (x0, y0) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - x0, fy - y0);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let s = [
                fetch(x0, y0),
                fetch(x0 + 1, y0),
                fetch(x0, y0 + 1),
                fetch(x0 + 1, y0 + 1),
            ];
            let w = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
            let a: f64 = s.iter().zip(&w).map(|(c, w)| c[3] * w).sum();
            let o = &mut out[((oy * out_w + ox) * 4) as usize..][..4];
            if a > 0.0 {
                for ch in 0..3 {
                    let v: f64 = s.iter().zip(&w).map(|(c, w)| c[ch] * c[3] * w).sum::<f64>() / a;
                    o[ch] = v.round().clamp(0.0, 255.0) as u8;
                }
            }
            o[3] = a.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("survey3d-imagery-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    /// A 700 × 300 gradient PNG with a transparent corner, and its world file.
    fn sample(dir: &Path) -> PathBuf {
        let img = image::RgbaImage::from_fn(700, 300, |x, y| {
            let a = if x < 50 && y < 50 { 0 } else { 255 };
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 100, a])
        });
        let path = dir.join("ortho.png");
        img.save(&path).expect("png");
        std::fs::write(dir.join("ortho.pgw"), "0.1\n0\n0\n-0.1\n1000.05\n2000.95\n").expect("pgw");
        path
    }

    #[test]
    fn png_with_a_world_file() {
        let dir = temp("png");
        let path = sample(&dir);
        let info = inspect(&path).expect("inspect");
        assert_eq!((info.width, info.height), (700, 300));
        assert_eq!(info.placed_by.as_deref(), Some("ortho.pgw"));
        let t = info.transform.expect("placed");
        assert!((t.apply(DVec2::ZERO) - DVec2::new(1000.0, 2001.0)).length() < 1e-9);
        assert!(world_file_for(&dir.join("other.png")).is_none());
        assert!(inspect(&dir.join("x.ecw")).is_err());
        // Missing and unreadable files say which file and why.
        let e = inspect(&dir.join("gone.tif")).expect_err("missing").to_string();
        assert!(e.contains("gone.tif was not found"), "{e}");
        std::fs::write(dir.join("fake.tif"), b"not a tiff").expect("fake");
        let e = inspect(&dir.join("fake.tif")).expect_err("fake").to_string();
        assert!(e.contains("fake.tif is not a readable TIFF"), "{e}");
    }

    #[test]
    fn pyramid_levels_and_tiles() {
        assert_eq!(level_count(256, 100), 1);
        assert_eq!(level_count(257, 100), 2);
        assert_eq!(level_count(700, 300), 3);
        let dir = temp("pyramid");
        let path = sample(&dir);
        let cache_path = dir.join("ortho.s3dimg");
        let mut seen = 0.0;
        let cache = build_cache(&path, Some(&cache_path), &mut |f| {
            seen = f;
            true
        })
        .expect("build");
        assert_eq!(seen, 1.0);
        assert_eq!(
            (cache.levels, cache.tiles(0), cache.tiles(2)),
            (3, (3, 2), (1, 1))
        );
        // Level 0 tile (1, 0) is opaque: pixel (10, 20) of it is image pixel (266, 20).
        let t = cache.tile(0, 1, 0).expect("read").expect("tile");
        assert_eq!((t.width, t.height), (256, 256));
        let p = &t.rgba[((20 * 256 + 10) * 4)..][..4];
        assert!(
            (i32::from(p[0]) - 266 % 256).abs() <= 3 && (i32::from(p[1]) - 20).abs() <= 3,
            "{p:?}"
        );
        // The edge tile is cut to the image.
        let edge = cache.tile(0, 2, 1).expect("read").expect("tile");
        assert_eq!((edge.width, edge.height), (700 - 512, 300 - 256));
        // The top-left corner is transparent (PNG, alpha kept).
        let corner = cache.tile(0, 0, 0).expect("read").expect("tile");
        assert_eq!(corner.rgba[3], 0);
        assert_eq!(corner.rgba[(100 * 256 + 100) * 4 + 3], 255);
        // Reopened from disk; stale once the source changes.
        drop(cache);
        let again = TileCache::open(&cache_path, &path).expect("reopen");
        assert_eq!(again.levels, 3);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        image::RgbaImage::new(10, 10).save(&path).expect("rewrite");
        assert!(TileCache::open(&cache_path, &path).is_none());
    }

    #[test]
    fn resampling_follows_the_mapping() {
        let dir = temp("resample");
        let path = sample(&dir);
        let cache = build_cache(&path, None, &mut |_| true).expect("memory");
        // A 70 × 30 copy at a tenth of the size, from level 3 (clamped to 2).
        let out = resample(&cache, 3, 70, 30, |x, y| Some(DVec2::new(x * 10.0, y * 10.0)));
        assert_eq!(out.len(), 70 * 30 * 4);
        let p = &out[((15 * 70 + 40) * 4)..][..4];
        assert_eq!(p[3], 255);
        assert!((i32::from(p[2]) - 100).abs() <= 2, "{p:?}");
        // Outside the mapping stays transparent.
        let none = resample(&cache, 0, 4, 4, |_, _| None);
        assert!(none.iter().all(|v| *v == 0));
    }

    /// GDAL-made GeoTIFFs in `SURVEY3D_IMAGERY_DIR` (see the test's list):
    /// each a 3000 × 2000 image of colour (100, 150, 200) placed in MGA 55
    /// at 0.05 m pixels from (300000, 6250100).
    #[test]
    fn gdal_files() {
        let Ok(dir) = std::env::var("SURVEY3D_IMAGERY_DIR") else {
            return;
        };
        let dir = Path::new(&dir);
        for (name, grey) in [
            ("cog_jpeg.tif", false),
            ("ycbcr.tif", false),
            ("lzw_alpha_big.tif", false),
            ("nodata.tif", false),
            ("rgb16.tif", false),
            ("gray.tif", true),
            ("photo.jpg", false),
        ] {
            let path = dir.join(name);
            let info = inspect(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!((info.width, info.height), (3000, 2000), "{name}");
            let t = info.transform.unwrap_or_else(|| panic!("{name}: not placed"));
            assert!(
                (t.apply(DVec2::ZERO) - DVec2::new(300_000.0, 6_250_100.0)).length() < 1e-6,
                "{name}: {t:?}"
            );
            assert!((t.pixel_size() - 0.05).abs() < 1e-9, "{name}");
            if name.ends_with(".tif") {
                assert_eq!(info.epsg, Some(7855), "{name}");
            }
            let cache = build_cache(&path, None, &mut |_| true).unwrap_or_else(|e| panic!("{name}: {e}"));
            let tile = cache
                .tile(0, 5, 3)
                .expect("read")
                .unwrap_or_else(|| panic!("{name}: empty tile"));
            let p = &tile.rgba[((100 * tile.width + 100) * 4) as usize..][..4];
            let want = if grey { [100, 100, 100] } else { [100, 150, 200] };
            for k in 0..3 {
                assert!((i32::from(p[k]) - want[k]).abs() <= 3, "{name}: {p:?}");
            }
            assert_eq!(p[3], 255, "{name}");
        }
        // An elevation model is refused before anything is built.
        let e = inspect(&dir.join("dem.tif")).expect_err("dem").to_string();
        assert!(e.contains("floating-point data, not an image"), "{e}");
        // A latitude/longitude GeoTIFF with a world file beside it takes the world file.
        if dir.join("geo4326.tfw").exists() {
            let g = inspect(&dir.join("geo4326.tif")).expect("geo");
            assert_eq!(g.placed_by.as_deref(), Some("geo4326.tfw"));
            assert!(!g.geographic && g.epsg.is_none(), "{g:?}");
        }
        let e = build_cache(&dir.join("planar.tif"), None, &mut |_| true).expect_err("planar");
        assert!(e.to_string().contains("INTERLEAVE=PIXEL"), "{e}");
    }

    #[test]
    fn halving_weights_by_alpha() {
        // A 2 × 1 band: red opaque and green transparent halve to red, half alpha.
        let (out, w, h) = halve(&[255, 0, 0, 255, 0, 255, 0, 0], 2, 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![255, 0, 0, 127]);
    }
}
