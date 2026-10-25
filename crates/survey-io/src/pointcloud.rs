//! Point clouds: plain-text XYZ (optionally with RGB) and LAS 1.0–1.4
//! (uncompressed; point formats 0–10, colour from formats 2, 3, 5, 7, 8, 10).

use glam::DVec3;

use crate::IoError;

/// Points read from a file (absolute coordinates, metres as stored).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawCloud {
    pub points: Vec<DVec3>,
    /// One colour per point, when the file has them.
    pub colors: Option<Vec<[u8; 3]>>,
}

/// Reads XYZ text: `x y z [r g b]` per line, separated by spaces, tabs,
/// commas or semicolons. Lines that do not start with three numbers (headers,
/// comments) are skipped. 16-bit colours are scaled to 8 bits.
pub fn read_xyz(bytes: &[u8]) -> Result<RawCloud, IoError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::NotText)?;
    let mut points = Vec::new();
    let mut colors: Vec<[u16; 3]> = Vec::new();
    let mut all_have_color = true;
    for line in text.lines() {
        let mut v = [0f64; 6];
        let mut n = 0;
        for tok in line
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .filter(|t| !t.is_empty())
        {
            if n == 6 {
                break;
            }
            match tok.parse::<f64>() {
                Ok(x) => {
                    v[n] = x;
                    n += 1;
                }
                Err(_) => break,
            }
        }
        if n < 3 {
            continue;
        }
        points.push(DVec3::new(v[0], v[1], v[2]));
        if n >= 6 {
            colors.push([v[3], v[4], v[5]].map(|c| c.clamp(0.0, 65535.0) as u16));
        } else {
            all_have_color = false;
        }
    }
    if points.is_empty() {
        return Err(IoError::NoPoints(" — no x y z lines found".into()));
    }
    let colors = (all_have_color && colors.len() == points.len()).then(|| scale_colors(&colors));
    Ok(RawCloud { points, colors })
}

/// 16-bit colours are scaled down when any channel exceeds 255.
pub(crate) fn scale_colors(c: &[[u16; 3]]) -> Vec<[u8; 3]> {
    let wide = c.iter().any(|x| x.iter().any(|v| *v > 255));
    c.iter()
        .map(|x| x.map(|v| if wide { (v >> 8) as u8 } else { v as u8 }))
        .collect()
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn i32_at(b: &[u8], i: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}
fn f64_at(b: &[u8], i: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// Reads an uncompressed LAS file.
pub fn read_las(bytes: &[u8]) -> Result<RawCloud, IoError> {
    let bad = |m: &str| IoError::Invalid(format!("LAS: {m}"));
    if bytes.get(0..4) != Some(b"LASF") {
        return Err(bad("not a LAS file (LAZ compressed files are not supported yet)"));
    }
    let minor = *bytes.get(25).ok_or_else(|| bad("truncated header"))?;
    let offset = u32_at(bytes, 96).ok_or_else(|| bad("truncated header"))? as usize;
    let format_raw = *bytes.get(104).ok_or_else(|| bad("truncated header"))?;
    // Bit 7 marks compression (LAZ).
    if format_raw & 0x80 != 0 {
        return Err(bad("compressed (LAZ) point data is not supported yet"));
    }
    let format = format_raw & 0x3f;
    let record = u16_at(bytes, 105).ok_or_else(|| bad("truncated header"))? as usize;
    let legacy_count = u32_at(bytes, 107).ok_or_else(|| bad("truncated header"))? as u64;
    let count = if minor >= 4 && legacy_count == 0 {
        u64_at(bytes, 247).unwrap_or(0)
    } else {
        legacy_count
    } as usize;
    let scale = [131, 139, 147].map(|i| f64_at(bytes, i).unwrap_or(1.0));
    let off = [155, 163, 171].map(|i| f64_at(bytes, i).unwrap_or(0.0));
    // Byte offset of the RGB fields for formats that carry colour.
    let rgb_at = match format {
        2 => Some(20),
        3 | 5 => Some(28),
        7 | 8 | 10 => Some(30),
        _ => None,
    };
    if record < 12 {
        return Err(bad("point records are too short"));
    }
    let available = bytes.len().saturating_sub(offset) / record;
    let n = count.min(available);
    let mut points = Vec::with_capacity(n);
    let mut colors: Vec<[u16; 3]> = Vec::with_capacity(if rgb_at.is_some() { n } else { 0 });
    for k in 0..n {
        let base = offset + k * record;
        let xyz = [0, 4, 8].map(|i| i32_at(bytes, base + i).unwrap_or(0));
        points.push(DVec3::new(
            f64::from(xyz[0]) * scale[0] + off[0],
            f64::from(xyz[1]) * scale[1] + off[1],
            f64::from(xyz[2]) * scale[2] + off[2],
        ));
        if let Some(c) = rgb_at {
            colors.push([0, 2, 4].map(|i| u16_at(bytes, base + c + i).unwrap_or(0)));
        }
    }
    if points.is_empty() {
        return Err(IoError::NoPoints(" — the LAS file has no points".into()));
    }
    let colors = rgb_at.map(|_| scale_colors(&colors));
    Ok(RawCloud { points, colors })
}

/// Reads LAS or XYZ text by content.
pub fn read_cloud(bytes: &[u8]) -> Result<RawCloud, IoError> {
    if bytes.starts_with(b"LASF") {
        read_las(bytes)
    } else {
        read_xyz(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xyz_with_and_without_colour() {
        let c = read_xyz(b"# header\nx,y,z\n1 2 3\n4,5,6\n").expect("xyz");
        assert_eq!(
            c.points,
            vec![DVec3::new(1.0, 2.0, 3.0), DVec3::new(4.0, 5.0, 6.0)]
        );
        assert!(c.colors.is_none());
        let c = read_xyz(b"512000.1 6912000.2 10.5 65535 0 32768\n").expect("xyz");
        assert_eq!(c.colors, Some(vec![[255, 0, 128]]));
        assert!(read_xyz(b"no numbers here\n").is_err());
    }

    /// A minimal LAS 1.2, point format 2 (with RGB).
    fn las(points: &[(i32, i32, i32, [u16; 3])]) -> Vec<u8> {
        let mut h = vec![0u8; 227];
        h[0..4].copy_from_slice(b"LASF");
        h[24] = 1;
        h[25] = 2;
        h[94..96].copy_from_slice(&227u16.to_le_bytes());
        h[96..100].copy_from_slice(&227u32.to_le_bytes());
        h[104] = 2;
        h[105..107].copy_from_slice(&26u16.to_le_bytes());
        h[107..111].copy_from_slice(&(points.len() as u32).to_le_bytes());
        for (i, s) in [131, 139, 147].iter().zip([0.001f64, 0.001, 0.001]) {
            h[*i..*i + 8].copy_from_slice(&s.to_le_bytes());
        }
        for (i, o) in [155, 163, 171].iter().zip([512_000.0f64, 6_912_000.0, 0.0]) {
            h[*i..*i + 8].copy_from_slice(&o.to_le_bytes());
        }
        for (x, y, z, rgb) in points {
            let mut r = vec![0u8; 26];
            r[0..4].copy_from_slice(&x.to_le_bytes());
            r[4..8].copy_from_slice(&y.to_le_bytes());
            r[8..12].copy_from_slice(&z.to_le_bytes());
            for (k, c) in rgb.iter().enumerate() {
                r[20 + 2 * k..22 + 2 * k].copy_from_slice(&c.to_le_bytes());
            }
            h.extend(r);
        }
        h
    }

    #[test]
    fn las_points_and_colours() {
        let bytes = las(&[
            (1500, 2500, 100_250, [65535, 0, 0]),
            (-500, 0, 99_000, [0, 65535, 256]),
        ]);
        let c = read_cloud(&bytes).expect("las");
        assert_eq!(c.points.len(), 2);
        assert!((c.points[0] - DVec3::new(512_001.5, 6_912_002.5, 100.25)).length() < 1e-9);
        assert!((c.points[1] - DVec3::new(511_999.5, 6_912_000.0, 99.0)).length() < 1e-9);
        assert_eq!(c.colors, Some(vec![[255, 0, 0], [0, 255, 1]]));
        let mut laz = bytes.clone();
        laz[104] |= 0x80;
        assert!(read_las(&laz).is_err());
    }
}
