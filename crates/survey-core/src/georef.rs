//! Georeferencing images: the affine transform from pixels to plan
//! coordinates, read from world files and GeoTIFF tags or fitted to control
//! points (Helmert: scale, rotation and shift; or affine) with residuals.
//!
//! Pixel coordinates are (column, row) from the image's top-left corner,
//! rows downwards, so pixel (i, j) covers [i, i + 1) × [j, j + 1) and its
//! centre is (i + 0.5, j + 0.5).

use glam::DVec2;
use serde::{Deserialize, Serialize};

/// x = a·u + b·v + c, y = d·u + e·v + f for pixel (u, v).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageTransform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl ImageTransform {
    /// North-up pixels of `size` with the image's top-left corner at `top_left`.
    pub fn north_up(top_left: DVec2, size: f64) -> Self {
        Self {
            a: size,
            b: 0.0,
            c: top_left.x,
            d: 0.0,
            e: -size,
            f: top_left.y,
        }
    }

    pub fn apply(&self, pixel: DVec2) -> DVec2 {
        DVec2::new(
            self.a * pixel.x + self.b * pixel.y + self.c,
            self.d * pixel.x + self.e * pixel.y + self.f,
        )
    }

    fn det(&self) -> f64 {
        self.a * self.e - self.b * self.d
    }

    /// The pixel at a plan position; `None` for a degenerate transform.
    pub fn inverse(&self, p: DVec2) -> Option<DVec2> {
        let det = self.det();
        if det.abs() < 1e-300 || !det.is_finite() {
            return None;
        }
        let (x, y) = (p.x - self.c, p.y - self.f);
        Some(DVec2::new(
            (self.e * x - self.b * y) / det,
            (self.a * y - self.d * x) / det,
        ))
    }

    /// Ground size of a pixel (the square root of its area).
    pub fn pixel_size(&self) -> f64 {
        self.det().abs().sqrt()
    }

    /// Rotation of the image's x axis from east (radians, anticlockwise).
    pub fn rotation(&self) -> f64 {
        self.d.atan2(self.a)
    }

    /// The plan corners of a `width` × `height` image: top-left, top-right,
    /// bottom-right, bottom-left.
    pub fn corners(&self, width: f64, height: f64) -> [DVec2; 4] {
        [
            self.apply(DVec2::ZERO),
            self.apply(DVec2::new(width, 0.0)),
            self.apply(DVec2::new(width, height)),
            self.apply(DVec2::new(0.0, height)),
        ]
    }

    /// From a world file (`.tfw`, `.jgw`, `.pgw`, `.wld`): six numbers A, D,
    /// B, E, C, F, where C and F are the centre of the top-left pixel.
    pub fn from_world_file(text: &str) -> Result<Self, String> {
        let v: Vec<f64> = text
            .split_whitespace()
            .map(|t| t.replace(',', "."))
            .map(|t| t.parse::<f64>().map_err(|_| format!("'{t}' is not a number")))
            .collect::<Result<_, _>>()?;
        let [a, d, b, e, c, f] = v[..] else {
            return Err(format!("a world file has six numbers, this one {}", v.len()));
        };
        let t = Self {
            a,
            b,
            // The file gives the top-left pixel's centre; the transform its corner.
            c: c - 0.5 * a - 0.5 * b,
            d,
            e,
            f: f - 0.5 * d - 0.5 * e,
        };
        if t.det().abs() < 1e-300 || !t.det().is_finite() {
            return Err("the world file's pixel size is zero".into());
        }
        Ok(t)
    }

    /// As a world file's six lines.
    pub fn to_world_file(&self) -> String {
        let centre = self.apply(DVec2::splat(0.5));
        [self.a, self.d, self.b, self.e, centre.x, centre.y]
            .iter()
            .map(|v| format!("{v:.10}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    /// From GeoTIFF's model tie point and pixel scale. `area` is true for
    /// PixelIsArea (the tie point is a pixel's corner, the usual case).
    pub fn from_tie_point(tie: [f64; 4], scale: [f64; 2], area: bool) -> Self {
        let [i, j, x, y] = tie;
        let shift = if area { 0.0 } else { 0.5 };
        Self {
            a: scale[0],
            b: 0.0,
            c: x - (i + shift) * scale[0],
            d: 0.0,
            e: -scale[1],
            f: y + (j + shift) * scale[1],
        }
    }

    /// From GeoTIFF's 4 × 4 model transformation (row-major).
    pub fn from_matrix(m: &[f64], area: bool) -> Option<Self> {
        if m.len() < 8 {
            return None;
        }
        let mut t = Self {
            a: m[0],
            b: m[1],
            c: m[3],
            d: m[4],
            e: m[5],
            f: m[7],
        };
        if !area {
            // The raster space's (0, 0) is the first pixel's centre.
            let o = t.apply(DVec2::splat(-0.5));
            t.c = o.x;
            t.f = o.y;
        }
        Some(t)
    }
}

/// A control point: a pixel of the image and where it belongs on the plan.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlPoint {
    pub pixel: [f64; 2],
    pub world: [f64; 2],
    /// Used in the fit (else only checked against it).
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// How control points are fitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitKind {
    /// Scale, rotation and shift (2 points or more): the image keeps its shape.
    #[default]
    Helmert,
    /// Separate scales and shear as well (3 points or more).
    Affine,
}

impl FitKind {
    pub fn min_points(self) -> usize {
        match self {
            FitKind::Helmert => 2,
            FitKind::Affine => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FitKind::Helmert => "Helmert (scale, rotation, shift)",
            FitKind::Affine => "Affine",
        }
    }
}

/// A fitted transform and how well each control point agrees with it.
#[derive(Clone, Debug, PartialEq)]
pub struct GeorefFit {
    pub transform: ImageTransform,
    /// Plan position minus transformed pixel, for every point (enabled or not).
    pub residuals: Vec<DVec2>,
    /// Root mean square of the enabled points' residuals.
    pub rms: f64,
    pub used: usize,
}

/// Fits `kind` to the enabled control points.
pub fn fit(points: &[ControlPoint], kind: FitKind) -> Result<GeorefFit, String> {
    let used: Vec<&ControlPoint> = points.iter().filter(|p| p.enabled).collect();
    if used.len() < kind.min_points() {
        return Err(format!(
            "{} needs at least {} control points ({} in use)",
            kind.label(),
            kind.min_points(),
            used.len()
        ));
    }
    let px = |p: &ControlPoint| DVec2::from(p.pixel);
    let wd = |p: &ControlPoint| DVec2::from(p.world);
    let n = used.len() as f64;
    let pc = used.iter().map(|p| px(p)).sum::<DVec2>() / n;
    let wc = used.iter().map(|p| wd(p)).sum::<DVec2>() / n;
    let transform = match kind {
        FitKind::Helmert => {
            // Similarity from (u, −v) (pixels flipped to y up) to the plan, as
            // complex numbers: w − w̄ = α·(z − z̄).
            let (mut num, mut den) = (DVec2::ZERO, 0.0);
            for p in &used {
                let z = px(p) - pc;
                let z = DVec2::new(z.x, -z.y);
                let w = wd(p) - wc;
                // w · conj(z)
                num += DVec2::new(w.x * z.x + w.y * z.y, w.y * z.x - w.x * z.y);
                den += z.length_squared();
            }
            if den < 1e-12 {
                return Err("The control points are all on the same pixel".into());
            }
            let (p, q) = (num.x / den, num.y / den);
            // x = p·u + q·v + c, y = q·u − p·v + f (with z = (u, −v)).
            let mut t = ImageTransform {
                a: p,
                b: q,
                c: 0.0,
                d: q,
                e: -p,
                f: 0.0,
            };
            let at = t.apply(pc);
            t.c = wc.x - at.x;
            t.f = wc.y - at.y;
            t
        }
        FitKind::Affine => {
            // Least squares on centred coordinates: [x y] = M·[u v].
            let (mut suu, mut suv, mut svv) = (0.0, 0.0, 0.0);
            let (mut sxu, mut sxv, mut syu, mut syv) = (0.0, 0.0, 0.0, 0.0);
            for p in &used {
                let (u, w) = (px(p) - pc, wd(p) - wc);
                suu += u.x * u.x;
                suv += u.x * u.y;
                svv += u.y * u.y;
                sxu += w.x * u.x;
                sxv += w.x * u.y;
                syu += w.y * u.x;
                syv += w.y * u.y;
            }
            let det = suu * svv - suv * suv;
            if det.abs() < 1e-9 * (suu * svv).max(1e-300) {
                return Err("The control points are in a line: an affine fit needs them spread out".into());
            }
            let solve = |su: f64, sv: f64| ((su * svv - sv * suv) / det, (sv * suu - su * suv) / det);
            let (a, b) = solve(sxu, sxv);
            let (d, e) = solve(syu, syv);
            ImageTransform {
                a,
                b,
                c: wc.x - a * pc.x - b * pc.y,
                d,
                e,
                f: wc.y - d * pc.x - e * pc.y,
            }
        }
    };
    if transform.det().abs() < 1e-300 {
        return Err("The control points do not fix a scale".into());
    }
    let residuals: Vec<DVec2> = points.iter().map(|p| wd(p) - transform.apply(px(p))).collect();
    let rms = (points
        .iter()
        .zip(&residuals)
        .filter(|(p, _)| p.enabled)
        .map(|(_, r)| r.length_squared())
        .sum::<f64>()
        / n)
        .sqrt();
    Ok(GeorefFit {
        transform,
        residuals,
        rms,
        used: used.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: DVec2, b: DVec2) -> bool {
        a.distance(b) < 1e-6
    }

    #[test]
    fn world_files_round_trip() {
        // GDAL's example: 60 m pixels, top-left pixel centre at (440750, 3751290).
        let t =
            ImageTransform::from_world_file("60.0\n0.0\n0.0\n-60.0\n440750.0\n3751290.0\n").expect("parse");
        assert!(close(
            t.apply(DVec2::splat(0.5)),
            DVec2::new(440_750.0, 3_751_290.0)
        ));
        assert!(close(t.apply(DVec2::ZERO), DVec2::new(440_720.0, 3_751_320.0)));
        let back = ImageTransform::from_world_file(&t.to_world_file()).expect("again");
        assert!((back.c - t.c).abs() < 1e-6 && (back.e - t.e).abs() < 1e-12);
        assert!(ImageTransform::from_world_file("1 0 0").is_err());
        assert!(ImageTransform::from_world_file("0 0 0 0 1 1").is_err());
        let p = DVec2::new(440_800.0, 3_751_000.0);
        assert!(close(t.apply(t.inverse(p).expect("inverse")), p));
    }

    #[test]
    fn geotiff_tie_points() {
        let area = ImageTransform::from_tie_point([0.0, 0.0, 300_000.0, 6_250_000.0], [0.05, 0.05], true);
        assert!(close(area.apply(DVec2::ZERO), DVec2::new(300_000.0, 6_250_000.0)));
        assert!(close(
            area.apply(DVec2::new(100.0, 100.0)),
            DVec2::new(300_005.0, 6_249_995.0)
        ));
        let point = ImageTransform::from_tie_point([0.0, 0.0, 300_000.0, 6_250_000.0], [0.05, 0.05], false);
        // The tie point is the first pixel's centre.
        assert!(close(
            point.apply(DVec2::splat(0.5)),
            DVec2::new(300_000.0, 6_250_000.0)
        ));
        let m = [0.05, 0.0, 0.0, 300_000.0, 0.0, -0.05, 0.0, 6_250_000.0];
        assert_eq!(ImageTransform::from_matrix(&m, true), Some(area));
    }

    /// A rotated, scaled image: 0.1 m pixels turned 30° anticlockwise.
    fn truth() -> ImageTransform {
        let (s, r) = (0.1, 30f64.to_radians());
        ImageTransform {
            a: s * r.cos(),
            b: s * r.sin(),
            c: 1000.0,
            d: s * r.sin(),
            e: -s * r.cos(),
            f: 2000.0,
        }
    }

    fn points(t: &ImageTransform, pixels: &[[f64; 2]]) -> Vec<ControlPoint> {
        pixels
            .iter()
            .map(|p| ControlPoint {
                pixel: *p,
                world: t.apply(DVec2::from(*p)).to_array(),
                enabled: true,
            })
            .collect()
    }

    #[test]
    fn helmert_recovers_a_rotated_image() {
        let t = truth();
        let pts = points(&t, &[[10.0, 20.0], [900.0, 40.0]]);
        let f = fit(&pts, FitKind::Helmert).expect("fit");
        assert!(close(
            f.transform.apply(DVec2::new(500.0, 700.0)),
            t.apply(DVec2::new(500.0, 700.0))
        ));
        assert!(f.rms < 1e-9 && (f.transform.pixel_size() - 0.1).abs() < 1e-12);
        assert!((f.transform.rotation() - 30f64.to_radians()).abs() < 1e-12);
        // A blunder shows in the residuals; switched off, the others fit exactly
        // and it is still checked.
        let mut pts = points(&t, &[[10.0, 20.0], [900.0, 40.0], [400.0, 800.0], [50.0, 600.0]]);
        pts[3].world[0] += 0.5;
        let f = fit(&pts, FitKind::Helmert).expect("fit");
        let worst = f
            .residuals
            .iter()
            .map(|r| r.length())
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(worst.map(|w| w.0), Some(3));
        pts[3].enabled = false;
        let f = fit(&pts, FitKind::Helmert).expect("fit");
        assert!(f.rms < 1e-9 && (f.residuals[3].x - 0.5).abs() < 1e-6 && f.used == 3);
    }

    #[test]
    fn affine_takes_shear_and_needs_three() {
        let mut t = truth();
        t.b += 0.01; // a little shear
        let pts = points(&t, &[[0.0, 0.0], [1000.0, 0.0], [0.0, 800.0], [700.0, 500.0]]);
        let f = fit(&pts, FitKind::Affine).expect("fit");
        assert!(
            f.rms < 1e-9
                && close(
                    f.transform.apply(DVec2::new(321.0, 123.0)),
                    t.apply(DVec2::new(321.0, 123.0))
                )
        );
        // Helmert cannot take the shear: residuals show it.
        assert!(fit(&pts, FitKind::Helmert).expect("helmert").rms > 0.1);
        assert!(fit(&pts[..2], FitKind::Affine).is_err());
        let line = points(&t, &[[0.0, 0.0], [10.0, 10.0], [20.0, 20.0]]);
        assert!(
            fit(&line, FitKind::Affine)
                .expect_err("collinear")
                .contains("in a line")
        );
    }
}
