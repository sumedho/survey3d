//! Geoid model files: PROJ GeoTIFF grids (`.tif`, as on cdn.proj.org), NOAA
//! VDatum `.gtx`, NGS `.bin` (GEOID18 and earlier), NTv2 `.gsb` (as
//! Geoscience Australia publishes AUSGeoid2020, the separation in the first
//! field) and GeographicLib `.pgm` (EGM96, EGM2008).

use std::io::Cursor;

use survey_core::geoid::{GeoidGrid, GeoidModel};
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

use crate::IoError;

/// File extensions read as geoid models.
pub const EXTENSIONS: [&str; 6] = ["tif", "tiff", "gtx", "bin", "gsb", "pgm"];

/// Whether a file name looks like a geoid model.
pub fn is_geoid_file(name: &str) -> bool {
    extension(name).is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}

fn extension(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    Some(ext.to_ascii_lowercase())
}

fn invalid(what: &str, e: impl std::fmt::Display) -> IoError {
    IoError::Invalid(format!("{what}: {e}"))
}

/// Reads a geoid model; the format is chosen by the file's extension.
pub fn read_geoid(name: &str, bytes: &[u8]) -> Result<GeoidModel, IoError> {
    let grids = match extension(name).as_deref() {
        Some("tif" | "tiff") => read_geotiff(bytes)?,
        Some("gtx") => vec![read_gtx(bytes)?],
        Some("bin") => vec![read_ngs_bin(bytes)?],
        Some("gsb") => read_ntv2(bytes)?,
        Some("pgm") => vec![read_pgm(bytes)?],
        _ => {
            return Err(IoError::Invalid(format!(
                "'{name}' is not a geoid file (.{})",
                EXTENSIONS.join(", .")
            )));
        }
    };
    GeoidModel::new(name, grids).map_err(IoError::Invalid)
}

/// Rows stored from the north, flipped to the south-first order of [`GeoidGrid`].
fn flip_rows(values: &mut [f32], cols: usize) {
    let rows = values.len() / cols;
    for r in 0..rows / 2 {
        let (top, bottom) = values.split_at_mut((rows - 1 - r) * cols);
        top[r * cols..(r + 1) * cols].swap_with_slice(&mut bottom[..cols]);
    }
}

/// Little- or big-endian readers over a byte slice.
struct Bytes<'a> {
    data: &'a [u8],
    big: bool,
}

impl Bytes<'_> {
    fn take<const N: usize>(&self, at: usize) -> Result<[u8; N], IoError> {
        self.data
            .get(at..at + N)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| IoError::Invalid("the file is cut short".into()))
    }

    fn f64(&self, at: usize) -> Result<f64, IoError> {
        let b = self.take::<8>(at)?;
        Ok(if self.big {
            f64::from_be_bytes(b)
        } else {
            f64::from_le_bytes(b)
        })
    }

    fn f32(&self, at: usize) -> Result<f32, IoError> {
        let b = self.take::<4>(at)?;
        Ok(if self.big {
            f32::from_be_bytes(b)
        } else {
            f32::from_le_bytes(b)
        })
    }

    fn i32(&self, at: usize) -> Result<i32, IoError> {
        let b = self.take::<4>(at)?;
        Ok(if self.big {
            i32::from_be_bytes(b)
        } else {
            i32::from_le_bytes(b)
        })
    }

    /// `n` f32 values from `at`, with `nodata(v)` values made NaN.
    fn f32s(&self, at: usize, n: usize, nodata: impl Fn(f32) -> bool) -> Result<Vec<f32>, IoError> {
        if self.data.len() < at + n * 4 {
            return Err(IoError::Invalid("the file is cut short".into()));
        }
        (0..n)
            .map(|i| self.f32(at + i * 4).map(|v| if nodata(v) { f32::NAN } else { v }))
            .collect()
    }
}

fn size(v: i32, what: &str) -> Result<usize, IoError> {
    usize::try_from(v)
        .ok()
        .filter(|n| *n >= 2 && *n < 1_000_000)
        .ok_or_else(|| IoError::Invalid(format!("bad {what} count {v}")))
}

/// NOAA VDatum GTX: big-endian header (south, west, dlat, dlon, rows, cols)
/// then rows of f32 from the south; −88.8888 marks no data.
pub fn read_gtx(bytes: &[u8]) -> Result<GeoidGrid, IoError> {
    let b = Bytes {
        data: bytes,
        big: true,
    };
    let (south, west, dlat, dlon) = (b.f64(0)?, b.f64(8)?, b.f64(16)?, b.f64(24)?);
    let (rows, cols) = (size(b.i32(32)?, "row")?, size(b.i32(36)?, "column")?);
    let values = b.f32s(40, rows * cols, |v| (v + 88.8888).abs() < 1e-3)?;
    GeoidGrid::new(south, west, dlat, dlon, rows, cols, values).map_err(IoError::Invalid)
}

/// NGS binary geoid (GEOID99 to GEOID18): header (south, west, dlat, dlon,
/// rows, cols, kind) in either byte order, then rows of f32 from the south.
pub fn read_ngs_bin(bytes: &[u8]) -> Result<GeoidGrid, IoError> {
    // The kind is 1 (f32 values); it shows the byte order.
    let big = [false, true]
        .into_iter()
        .find(|big| {
            Bytes {
                data: bytes,
                big: *big,
            }
            .i32(40)
            .is_ok_and(|k| k == 1)
        })
        .ok_or_else(|| IoError::Invalid("not an NGS geoid file".into()))?;
    let b = Bytes { data: bytes, big };
    let (south, west, dlat, dlon) = (b.f64(0)?, b.f64(8)?, b.f64(16)?, b.f64(24)?);
    let (rows, cols) = (size(b.i32(32)?, "row")?, size(b.i32(36)?, "column")?);
    let values = b.f32s(44, rows * cols, |v| v <= -9000.0)?;
    GeoidGrid::new(south, west, dlat, dlon, rows, cols, values).map_err(IoError::Invalid)
}

/// NTv2: an overview header, then each sub-grid's header and nodes (four
/// f32 each, rows from the south, east to west in each row, longitudes
/// positive west). Geoid files put the separation in the first field.
pub fn read_ntv2(bytes: &[u8]) -> Result<Vec<GeoidGrid>, IoError> {
    let label = |at: usize| String::from_utf8_lossy(bytes.get(at..at + 8).unwrap_or_default()).to_string();
    if !label(0).starts_with("NUM_OREC") {
        return Err(IoError::Invalid("not an NTv2 file".into()));
    }
    let big = !(Bytes {
        data: bytes,
        big: false,
    }
    .i32(8)
    .is_ok_and(|n| n == 11));
    let b = Bytes { data: bytes, big };
    let count = b.i32(40)?;
    let unit = match label(56).trim_matches(|c: char| c == '\0' || c.is_whitespace()) {
        "SECONDS" => 1.0 / 3600.0,
        "MINUTES" => 1.0 / 60.0,
        "DEGREES" => 1.0,
        other => return Err(IoError::Invalid(format!("unknown NTv2 unit '{other}'"))),
    };
    let mut at = 11 * 16;
    let mut grids = Vec::new();
    for _ in 0..count.max(0) {
        if !label(at).starts_with("SUB_NAME") {
            return Err(IoError::Invalid("bad NTv2 sub-grid header".into()));
        }
        let field = |i: usize| b.f64(at + i * 16 + 8);
        let (s_lat, n_lat, e_long, w_long) = (field(4)?, field(5)?, field(6)?, field(7)?);
        let (lat_inc, long_inc) = (field(8)?, field(9)?);
        let nodes = usize::try_from(b.i32(at + 10 * 16 + 8)?).unwrap_or(0);
        if !(lat_inc > 0.0 && long_inc > 0.0) {
            return Err(IoError::Invalid("bad NTv2 grid spacing".into()));
        }
        let rows = ((n_lat - s_lat) / lat_inc).round() as usize + 1;
        let cols = ((w_long - e_long) / long_inc).round() as usize + 1;
        if rows * cols != nodes {
            return Err(IoError::Invalid(format!(
                "NTv2 sub-grid of {rows}×{cols} nodes says it has {nodes}"
            )));
        }
        at += 11 * 16;
        if bytes.len() < at + nodes * 16 {
            return Err(IoError::Invalid("the file is cut short".into()));
        }
        let mut values = Vec::with_capacity(nodes);
        for r in 0..rows {
            // Stored east to west: read each row backwards.
            for c in (0..cols).rev() {
                // −999 and the like mark nodes without data (separations are within ±110 m).
                let v = b.f32(at + (r * cols + c) * 16)?;
                values.push(if v < -500.0 { f32::NAN } else { v });
            }
        }
        at += nodes * 16;
        grids.push(
            GeoidGrid::new(
                s_lat * unit,
                -w_long * unit,
                lat_inc * unit,
                long_inc * unit,
                rows,
                cols,
                values,
            )
            .map_err(IoError::Invalid)?,
        );
    }
    Ok(grids)
}

/// GeographicLib PGM: 16-bit values from the north pole and Greenwich, with
/// the offset and scale in the header comments.
pub fn read_pgm(bytes: &[u8]) -> Result<GeoidGrid, IoError> {
    if !bytes.starts_with(b"P5") {
        return Err(IoError::Invalid("not a GeographicLib geoid (.pgm) file".into()));
    }
    let (mut offset, mut scale) = (None, None);
    let mut numbers = Vec::new();
    let mut at = 2;
    // Header: whitespace-separated width, height and maximum, with # comments.
    while numbers.len() < 3 {
        let rest = bytes
            .get(at..)
            .ok_or_else(|| IoError::Invalid("bad PGM header".into()))?;
        match rest.first() {
            None => return Err(IoError::Invalid("bad PGM header".into())),
            Some(b'#') => {
                let end = rest.iter().position(|c| *c == b'\n').unwrap_or(rest.len());
                let line = String::from_utf8_lossy(&rest[1..end]);
                let mut words = line.split_whitespace();
                match (words.next(), words.next().and_then(|v| v.parse::<f64>().ok())) {
                    (Some("Offset"), Some(v)) => offset = Some(v),
                    (Some("Scale"), Some(v)) => scale = Some(v),
                    _ => {}
                }
                at += end + 1;
            }
            Some(c) if c.is_ascii_whitespace() => at += 1,
            Some(_) => {
                let end = rest
                    .iter()
                    .position(u8::is_ascii_whitespace)
                    .unwrap_or(rest.len());
                let n: usize = String::from_utf8_lossy(&rest[..end])
                    .parse()
                    .map_err(|e| invalid("PGM header", e))?;
                numbers.push(n);
                at += end;
            }
        }
    }
    at += 1; // the single whitespace after the maximum
    let (cols, rows) = (numbers[0], numbers[1]);
    let (Some(offset), Some(scale)) = (offset, scale) else {
        return Err(IoError::Invalid("the PGM file gives no Offset and Scale".into()));
    };
    if rows < 2 || cols < 2 || bytes.len() < at + rows * cols * 2 {
        return Err(IoError::Invalid("the file is cut short".into()));
    }
    let mut values: Vec<f32> = bytes[at..at + rows * cols * 2]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| (offset + scale * f64::from(u16::from_be_bytes(*v))) as f32)
        .collect();
    flip_rows(&mut values, cols);
    let step = 360.0 / cols as f64;
    GeoidGrid::new(
        90.0 - step * (rows - 1) as f64,
        0.0,
        step,
        step,
        rows,
        cols,
        values,
    )
    .map_err(IoError::Invalid)
}

/// PROJ GeoTIFF grids: every image (sub-grid) placed by its model tie point
/// and pixel scale, rows from the north. GDAL's no-data value and any
/// scale and offset in its metadata are applied.
pub fn read_geotiff(bytes: &[u8]) -> Result<Vec<GeoidGrid>, IoError> {
    let tiff_err = |e| invalid("GeoTIFF", e);
    let mut d = Decoder::new(Cursor::new(bytes))
        .map_err(tiff_err)?
        .with_limits(Limits::unlimited());
    let mut grids = Vec::new();
    loop {
        let (cols, rows) = d.dimensions().map_err(tiff_err)?;
        let (cols, rows) = (cols as usize, rows as usize);
        let scale = d.get_tag_f64_vec(Tag::Unknown(33550)).map_err(tiff_err)?;
        let tie = d.get_tag_f64_vec(Tag::Unknown(33922)).map_err(tiff_err)?;
        if scale.len() < 2 || tie.len() < 6 {
            return Err(IoError::Invalid("the GeoTIFF is not georeferenced".into()));
        }
        // Raster type 1 (PixelIsArea): the tie point is a pixel's corner, its node at the centre.
        let keys = d
            .find_tag_unsigned_vec::<u16>(Tag::Unknown(34735))
            .ok()
            .flatten()
            .unwrap_or_default();
        let area = keys
            .as_chunks::<4>()
            .0
            .iter()
            .skip(1)
            .find(|k| k[0] == 1025)
            .is_none_or(|k| k[3] == 1);
        let half = if area { 0.5 } else { 0.0 };
        let (dlon, dlat) = (scale[0], scale[1]);
        let west = tie[3] + (half - tie[0]) * dlon;
        let north = tie[4] - (half - tie[1]) * dlat;
        let metadata = d.get_tag_ascii_string(Tag::Unknown(42112)).unwrap_or_default();
        let item = |name: &str| -> Option<f64> {
            let start = metadata.find(&format!("name=\"{name}\""))?;
            let rest = &metadata[start..];
            let value = &rest[rest.find('>')? + 1..rest.find("</Item>")?];
            value.trim().parse().ok()
        };
        let (k, offset) = (item("SCALE").unwrap_or(1.0), item("OFFSET").unwrap_or(0.0));
        let nodata = d
            .get_tag_ascii_string(Tag::Unknown(42113))
            .ok()
            .and_then(|s| s.trim_matches(char::from(0)).trim().parse::<f64>().ok());
        let samples = d
            .find_tag_unsigned::<u16>(Tag::SamplesPerPixel)
            .ok()
            .flatten()
            .unwrap_or(1) as usize;
        let raw: Vec<f64> = match d.read_image().map_err(tiff_err)? {
            DecodingResult::F32(v) => v.into_iter().map(f64::from).collect(),
            DecodingResult::F64(v) => v,
            DecodingResult::I16(v) => v.into_iter().map(f64::from).collect(),
            DecodingResult::U16(v) => v.into_iter().map(f64::from).collect(),
            DecodingResult::I32(v) => v.into_iter().map(f64::from).collect(),
            _ => return Err(IoError::Invalid("unsupported GeoTIFF sample type".into())),
        };
        // The separation is the first band.
        let mut values: Vec<f32> = raw
            .iter()
            .step_by(samples.max(1))
            .map(|v| {
                if nodata.is_some_and(|n| (v - n).abs() < 1e-6) || !v.is_finite() {
                    f32::NAN
                } else {
                    (v * k + offset) as f32
                }
            })
            .collect();
        flip_rows(&mut values, cols);
        grids.push(
            GeoidGrid::new(
                north - dlat * (rows - 1) as f64,
                west,
                dlat,
                dlon,
                rows,
                cols,
                values,
            )
            .map_err(IoError::Invalid)?,
        );
        if !d.more_images() {
            break;
        }
        d.next_image().map_err(tiff_err)?;
    }
    Ok(grids)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N = lat + 2·lon at a node.
    fn n(lat: f64, lon: f64) -> f32 {
        (lat + 2.0 * lon) as f32
    }

    fn check(model: &GeoidModel, lat: f64, lon: f64) {
        let got = model
            .separation(lat, lon)
            .unwrap_or_else(|| panic!("no value at {lat} {lon}"));
        assert!(
            (got - f64::from(n(lat, lon))).abs() < 1e-3,
            "{got} at {lat} {lon}"
        );
    }

    #[test]
    fn gtx_and_ngs_bin() {
        // 3 rows from 10°S by 4 columns from 140°E, 0.5° apart.
        let mut gtx = Vec::new();
        for v in [-10.0f64, 140.0, 0.5, 0.5] {
            gtx.extend_from_slice(&v.to_be_bytes());
        }
        gtx.extend_from_slice(&3i32.to_be_bytes());
        gtx.extend_from_slice(&4i32.to_be_bytes());
        for r in 0..3 {
            for c in 0..4 {
                let v = if (r, c) == (0, 0) {
                    -88.8888
                } else {
                    n(-10.0 + 0.5 * r as f64, 140.0 + 0.5 * c as f64)
                };
                gtx.extend_from_slice(&v.to_be_bytes());
            }
        }
        let m = read_geoid("test.gtx", &gtx).expect("value");
        check(&m, -9.3, 141.2);
        assert!(m.separation(-9.9, 140.1).is_none(), "no-data node");

        let mut bin = Vec::new();
        for v in [-10.0f64, 140.0, 0.5, 0.5] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        for v in [3i32, 4, 1] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        for r in 0..3 {
            for c in 0..4 {
                bin.extend_from_slice(&n(-10.0 + 0.5 * r as f64, 140.0 + 0.5 * c as f64).to_le_bytes());
            }
        }
        check(&read_geoid("g2018.BIN", &bin).expect("value"), -9.1, 140.6);
    }

    #[test]
    fn ntv2_rows_run_east_to_west() {
        let mut f = Vec::new();
        let mut rec = |label: &str, value: &[u8]| {
            let mut l = [b' '; 8];
            l[..label.len()].copy_from_slice(label.as_bytes());
            f.extend_from_slice(&l);
            let mut v = [0u8; 8];
            v[..value.len()].copy_from_slice(value);
            f.extend_from_slice(&v);
        };
        rec("NUM_OREC", &11i32.to_le_bytes());
        rec("NUM_SREC", &11i32.to_le_bytes());
        rec("NUM_FILE", &1i32.to_le_bytes());
        rec("GS_TYPE", b"SECONDS ");
        for l in [
            "VERSION", "SYSTEM_F", "SYSTEM_T", "MAJOR_F", "MINOR_F", "MAJOR_T", "MINOR_T",
        ] {
            rec(l, &[0; 8]);
        }
        rec("SUB_NAME", b"AUSGEOID");
        for l in ["PARENT", "CREATED", "UPDATED"] {
            rec(l, b"NONE    ");
        }
        // 20°S to 19°S, 150°E to 151.5°E (west longitudes negative), 0.5° steps.
        for (l, v) in [
            ("S_LAT", -20.0 * 3600.0),
            ("N_LAT", -19.0 * 3600.0),
            ("E_LONG", -151.5 * 3600.0),
            ("W_LONG", -150.0 * 3600.0),
            ("LAT_INC", 1800.0),
            ("LONG_INC", 1800.0),
        ] {
            rec(l, &f64::to_le_bytes(v));
        }
        rec("GS_COUNT", &12i32.to_le_bytes());
        for r in 0..3 {
            for c in 0..4 {
                let (lat, lon) = (-20.0 + 0.5 * r as f64, 151.5 - 0.5 * c as f64);
                for v in [n(lat, lon), 1.0, 2.0, 3.0] {
                    f.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        let m = read_geoid("AUSGeoid.gsb", &f).expect("value");
        assert_eq!(m.extent(), (-20.0, 150.0, -19.0, 151.5));
        check(&m, -19.7, 150.2);
        check(&m, -19.0, 151.5);
    }

    #[test]
    fn pgm_from_the_north_pole() {
        // A 4 × 3 global grid (90° steps): rows at 90, 0 and −90.
        let (offset, scale) = (-100.0, 0.01);
        let mut f = b"P5\n# Geoid file\n# Offset -100\n# Scale 0.01\n4 3\n65535\n".to_vec();
        for lat in [90.0, 0.0, -90.0] {
            for c in 0..4 {
                let v = ((f64::from(n(lat, 90.0 * c as f64)) - offset) / scale).round() as u16;
                f.extend_from_slice(&v.to_be_bytes());
            }
        }
        let m = read_geoid("egm.pgm", &f).expect("value");
        check(&m, 0.0, 90.0);
        check(&m, -45.0, 45.0);
        // Between 270° and 360° (= 0°), wrapping round.
        let v = m.separation(0.0, 315.0).expect("value");
        assert!(
            (v - f64::from(n(0.0, 270.0) + n(0.0, 0.0)) / 2.0).abs() < 1e-2,
            "{v}"
        );
    }

    #[test]
    fn rejects_other_files() {
        assert!(read_geoid("notes.txt", b"hello").is_err());
        assert!(read_geoid("short.gtx", &[0; 20]).is_err());
        assert!(is_geoid_file("AUSGeoid2020_20180201.GSB") && !is_geoid_file("survey.s3d"));
    }

    /// Real files, when downloaded into `SURVEY3D_GEOID_DIR`: AUSGeoid2020
    /// (PROJ's GeoTIFF and GA's NTv2) and EGM96 (GeoTIFF and GTX) agree.
    #[test]
    fn real_files_agree() {
        let Ok(dir) = std::env::var("SURVEY3D_GEOID_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let load = |name: &str| {
            let path = dir.join(name);
            path.exists()
                .then(|| read_geoid(name, &std::fs::read(path).expect("value")).expect("value"))
        };
        if let (Some(tif), Some(gsb)) = (
            load("au_ga_AUSGeoid2020_20180201.tif"),
            load("AUSGeoid2020_20180201.gsb"),
        ) {
            // Flinders Peak and Sydney.
            for (lat, lon) in [
                (-37.951034, 144.424868),
                (-33.8688, 151.2093),
                (-12.4634, 130.8456),
            ] {
                let (a, b) = (
                    tif.separation(lat, lon).expect("value"),
                    gsb.separation(lat, lon).expect("value"),
                );
                eprintln!("AUSGeoid2020 at {lat} {lon}: tif {a:.4} gsb {b:.4}");
                assert!((a - b).abs() < 0.005, "{a} vs {b}");
            }
        }
        if let Some(g18) = load("us_noaa_g2018p0.tif") {
            // GEOID18 at San Juan, Puerto Rico: about −42 m.
            let n = g18.separation(18.4655, -66.1057).expect("value");
            assert!((-44.0..-40.0).contains(&n), "{n}");
        }
        if let (Some(tif), Some(gtx)) = (load("us_nga_egm96_15.tif"), load("egm96_15.gtx")) {
            for (lat, lon) in [(-37.95, 144.42), (51.5, -0.12), (0.0, 179.9)] {
                let (a, b) = (
                    tif.separation(lat, lon).expect("value"),
                    gtx.separation(lat, lon).expect("value"),
                );
                eprintln!("EGM96 at {lat} {lon}: tif {a:.4} gtx {b:.4}");
                assert!((a - b).abs() < 0.005, "{a} vs {b} at {lat} {lon}");
            }
        }
    }
}
