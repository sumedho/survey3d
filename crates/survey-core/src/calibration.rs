//! Site calibration (localisation): fits GNSS-derived grid coordinates to the
//! local control a site is built on, as Trimble, Leica and Carlson controllers
//! do (see the Trimble Access calculations reference).
//!
//! - **Horizontal:** an unweighted least-squares 2-D similarity (shifts,
//!   rotation and scale; or shifts and rotation with the scale held at 1). One
//!   pair gives a shift only.
//! - **Vertical:** a constant height shift, or an inclined plane (a shift plus
//!   a tilt north and east) fitted to the height differences.
//!
//! Residuals are control minus calibrated measured coordinates. Everything is
//! computed about the centroid of the pairs, so grid coordinates keep full
//! precision.

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How heights are adjusted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalMethod {
    /// Heights are left as measured.
    None,
    /// The mean height difference.
    Constant,
    /// A shift plus a tilt north and east (three or more points).
    #[default]
    InclinedPlane,
}

impl VerticalMethod {
    pub const ALL: [VerticalMethod; 3] = [
        VerticalMethod::None,
        VerticalMethod::Constant,
        VerticalMethod::InclinedPlane,
    ];

    pub fn key(self) -> &'static str {
        match self {
            VerticalMethod::None => "none",
            VerticalMethod::Constant => "constant",
            VerticalMethod::InclinedPlane => "plane",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VerticalMethod::None => "None",
            VerticalMethod::Constant => "Constant shift",
            VerticalMethod::InclinedPlane => "Inclined plane",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(VerticalMethod::None),
            "constant" | "shift" => Some(VerticalMethod::Constant),
            "plane" | "inclined" | "inclined_plane" | "tilt" => Some(VerticalMethod::InclinedPlane),
            _ => None,
        }
    }
}

/// Fit settings and the residual limits that flag a bad pair.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalibrationOptions {
    /// Hold the scale at exactly 1 (fit shifts and rotation only).
    pub fixed_scale: bool,
    pub vertical: VerticalMethod,
    /// Largest acceptable horizontal residual (metres).
    pub tolerance_h: f64,
    /// Largest acceptable vertical residual (metres).
    pub tolerance_v: f64,
}

impl Default for CalibrationOptions {
    fn default() -> Self {
        Self {
            fixed_scale: false,
            vertical: VerticalMethod::InclinedPlane,
            tolerance_h: 0.02,
            tolerance_v: 0.03,
        }
    }
}

/// One point known in both systems.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair {
    /// From GNSS (projected grid coordinates).
    pub measured: DVec3,
    /// Local control.
    pub control: DVec3,
    /// Used in the horizontal fit.
    pub horizontal: bool,
    /// Used in the vertical fit (both heights must be known).
    pub vertical: bool,
}

/// A 2-D similarity about `origin`: `p' = origin_control + [a −b; b a](p − origin)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Horizontal {
    /// Centroid of the measured points (the transformation's origin).
    pub origin: DVec2,
    /// Where the origin lands in the control system.
    pub origin_control: DVec2,
    /// scale · cos θ and scale · sin θ (θ anticlockwise).
    pub a: f64,
    pub b: f64,
}

impl Horizontal {
    pub fn apply(&self, p: DVec2) -> DVec2 {
        let d = p - self.origin;
        self.origin_control + DVec2::new(self.a * d.x - self.b * d.y, self.b * d.x + self.a * d.y)
    }

    pub fn scale(&self) -> f64 {
        self.a.hypot(self.b)
    }

    /// Rotation as a change of bearing (clockwise, radians).
    pub fn rotation(&self) -> f64 {
        -self.b.atan2(self.a)
    }

    /// The shift at the origin.
    pub fn shift(&self) -> DVec2 {
        self.origin_control - self.origin
    }
}

/// A height shift `constant + slope_north·(N − N₀) + slope_east·(E − E₀)`, in
/// measured coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vertical {
    pub origin: DVec2,
    pub constant: f64,
    /// Metres per metre.
    pub slope_north: f64,
    pub slope_east: f64,
}

impl Vertical {
    pub fn shift_at(&self, p: DVec2) -> f64 {
        let d = p - self.origin;
        self.constant + self.slope_north * d.y + self.slope_east * d.x
    }

    /// The steepest tilt (metres per metre).
    pub fn tilt(&self) -> f64 {
        self.slope_north.hypot(self.slope_east)
    }
}

/// The fitted transformation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub horizontal: Option<Horizontal>,
    pub vertical: Option<Vertical>,
}

impl Calibration {
    /// Measured (GNSS grid) coordinates to local coordinates.
    pub fn apply(&self, p: DVec3) -> DVec3 {
        let xy = p.truncate();
        let z = p.z + self.vertical.map_or(0.0, |v| v.shift_at(xy));
        self.horizontal.map_or(xy, |h| h.apply(xy)).extend(z)
    }

    /// Plan positions only (heights unchanged).
    pub fn apply_plan(&self, p: DVec3) -> DVec3 {
        self.horizontal
            .map_or(p.truncate(), |h| h.apply(p.truncate()))
            .extend(p.z)
    }

    /// Rotation as a change of bearing (clockwise, radians).
    pub fn rotation(&self) -> f64 {
        self.horizontal.map_or(0.0, |h| h.rotation())
    }
}

/// A pair's misfit: control minus calibrated measured.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Residual {
    pub horizontal: Option<DVec2>,
    pub vertical: Option<f64>,
}

/// A calibration with its residuals and the checks a surveyor should see.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub calibration: Calibration,
    /// One per pair, in order.
    pub residuals: Vec<Residual>,
    pub horizontal_points: usize,
    pub vertical_points: usize,
    /// Observations beyond those needed (0 = no check).
    pub horizontal_redundancy: usize,
    pub vertical_redundancy: usize,
    /// The height method actually used (a plane falls back to a constant with
    /// too few points); `None` without a height fit.
    pub vertical_method: Option<VerticalMethod>,
    /// Plain-language cautions (weak geometry, odd scale, steep tilt…).
    pub warnings: Vec<String>,
}

impl Fit {
    pub fn max_horizontal(&self) -> Option<f64> {
        self.residuals
            .iter()
            .filter_map(|r| r.horizontal.map(|v| v.length()))
            .reduce(f64::max)
    }

    pub fn max_vertical(&self) -> Option<f64> {
        self.residuals
            .iter()
            .filter_map(|r| r.vertical.map(f64::abs))
            .reduce(f64::max)
    }

    /// Pairs whose residual is over the limits.
    pub fn outliers(&self, opts: &CalibrationOptions) -> Vec<usize> {
        self.residuals
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.horizontal.is_some_and(|v| v.length() > opts.tolerance_h)
                    || r.vertical.is_some_and(|v| v.abs() > opts.tolerance_v)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum CalibrationError {
    #[error("Use at least one pair horizontally or vertically.")]
    NoPairs,
    #[error("The horizontal pairs are all at one place; use points spread around the site.")]
    Coincident,
}

/// A scale this far from 1 usually means a mismatched pair or the wrong
/// coordinate system (1000 ppm = 1 m per km).
const SCALE_WARNING: f64 = 1e-3;
/// A plane tilt beyond this (100 ppm = 0.1 m per km) deserves a look.
const TILT_WARNING: f64 = 1e-4;

/// Fits a calibration to `pairs`.
pub fn fit(pairs: &[Pair], opts: &CalibrationOptions) -> Result<Fit, CalibrationError> {
    let hp: Vec<&Pair> = pairs.iter().filter(|p| p.horizontal).collect();
    let vp: Vec<&Pair> = pairs
        .iter()
        .filter(|p| p.vertical && opts.vertical != VerticalMethod::None)
        .collect();
    if hp.is_empty() && vp.is_empty() {
        return Err(CalibrationError::NoPairs);
    }
    let mut warnings = Vec::new();

    let horizontal = if hp.is_empty() {
        None
    } else {
        Some(fit_horizontal(&hp, opts.fixed_scale)?)
    };
    let free_params = if opts.fixed_scale { 3 } else { 4 };
    let h_redundancy = match hp.len() {
        0 | 1 => 0,
        n => (2 * n).saturating_sub(free_params),
    };
    match hp.len() {
        0 => {}
        1 => warnings.push(
            "One horizontal pair gives a shift only, with no rotation, scale or check. \
             Use 3 or more pairs spread around the site."
                .into(),
        ),
        _ if h_redundancy == 0 => warnings.push(
            "Two horizontal pairs fit exactly, so there is no check. \
             Use 3 or more pairs (4 or more is better) around the site."
                .into(),
        ),
        _ => {}
    }
    if let Some(h) = horizontal
        && !opts.fixed_scale
        && hp.len() >= 2
        && (h.scale() - 1.0).abs() > SCALE_WARNING
    {
        warnings.push(format!(
            "The scale is {:+.0} ppm from 1: check that each pair matches the same mark \
             and that both sets use the right coordinate system.",
            (h.scale() - 1.0) * 1e6
        ));
    }

    let (vertical, v_redundancy, vertical_method) = if vp.is_empty() {
        (None, 0, None)
    } else {
        let (v, plane) = fit_vertical(&vp, opts.vertical, &mut warnings);
        let (unknowns, method) = if plane {
            (3, VerticalMethod::InclinedPlane)
        } else {
            (1, VerticalMethod::Constant)
        };
        (Some(v), vp.len().saturating_sub(unknowns), Some(method))
    };
    if !vp.is_empty() && v_redundancy == 0 {
        warnings.push(format!(
            "{} vertical point{} fit exactly, so there is no height check. Use 4 or more.",
            vp.len(),
            if vp.len() == 1 { "" } else { "s" }
        ));
    }
    if let Some(v) = vertical
        && v.tilt() > TILT_WARNING
    {
        warnings.push(format!(
            "The height plane tilts {:.0} ppm ({:.3} m per km): check the heights; a wrong tilt \
             grows height errors away from the control.",
            v.tilt() * 1e6,
            v.tilt() * 1000.0
        ));
    }

    let calibration = Calibration { horizontal, vertical };
    let residuals = pairs
        .iter()
        .map(|p| Residual {
            horizontal: horizontal
                .filter(|_| p.horizontal)
                .map(|h| p.control.truncate() - h.apply(p.measured.truncate())),
            vertical: vertical
                .filter(|_| p.vertical && opts.vertical != VerticalMethod::None)
                .map(|v| p.control.z - (p.measured.z + v.shift_at(p.measured.truncate()))),
        })
        .collect();
    let fit = Fit {
        calibration,
        residuals,
        horizontal_points: hp.len(),
        vertical_points: vp.len(),
        horizontal_redundancy: h_redundancy,
        vertical_redundancy: v_redundancy,
        vertical_method,
        warnings,
    };
    let outliers = fit.outliers(opts).len();
    let mut fit = fit;
    if outliers > 0 {
        fit.warnings.push(format!(
            "{outliers} pair{} over the residual limits ({} m horizontal, {} m vertical). \
             Check the marks, or turn off a suspect pair and refit.",
            if outliers == 1 { " is" } else { "s are" },
            opts.tolerance_h,
            opts.tolerance_v
        ));
    }
    Ok(fit)
}

fn fit_horizontal(pairs: &[&Pair], fixed_scale: bool) -> Result<Horizontal, CalibrationError> {
    let n = pairs.len() as f64;
    let origin = pairs.iter().map(|p| p.measured.truncate()).sum::<DVec2>() / n;
    let origin_control = pairs.iter().map(|p| p.control.truncate()).sum::<DVec2>() / n;
    if pairs.len() == 1 {
        return Ok(Horizontal {
            origin,
            origin_control,
            a: 1.0,
            b: 0.0,
        });
    }
    let (mut sxx, mut sa, mut sb) = (0.0, 0.0, 0.0);
    for p in pairs {
        let m = p.measured.truncate() - origin;
        let c = p.control.truncate() - origin_control;
        sxx += m.length_squared();
        sa += m.dot(c);
        sb += m.perp_dot(c);
    }
    if sxx < 1e-12 {
        return Err(CalibrationError::Coincident);
    }
    let (a, b) = if fixed_scale {
        let theta = sb.atan2(sa);
        (theta.cos(), theta.sin())
    } else {
        (sa / sxx, sb / sxx)
    };
    Ok(Horizontal {
        origin,
        origin_control,
        a,
        b,
    })
}

/// The height adjustment, and whether it is a plane (else a constant).
fn fit_vertical(pairs: &[&Pair], method: VerticalMethod, warnings: &mut Vec<String>) -> (Vertical, bool) {
    let n = pairs.len() as f64;
    let origin = pairs.iter().map(|p| p.measured.truncate()).sum::<DVec2>() / n;
    let dh: Vec<f64> = pairs.iter().map(|p| p.control.z - p.measured.z).collect();
    let constant = dh.iter().sum::<f64>() / n;
    let flat = Vertical {
        origin,
        constant,
        slope_north: 0.0,
        slope_east: 0.0,
    };
    if method != VerticalMethod::InclinedPlane {
        return (flat, false);
    }
    if pairs.len() < 3 {
        warnings.push("An inclined plane needs 3 or more vertical points; used a constant shift.".into());
        return (flat, false);
    }
    // Normal equations for the tilt about the centroid (the shift is the mean).
    let (mut nn, mut ne, mut ee, mut nh, mut eh) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (p, h) in pairs.iter().zip(&dh) {
        let d = p.measured.truncate() - origin;
        let r = h - constant;
        nn += d.y * d.y;
        ne += d.x * d.y;
        ee += d.x * d.x;
        nh += d.y * r;
        eh += d.x * r;
    }
    let det = nn * ee - ne * ne;
    if det <= 1e-9 * (nn + ee).powi(2) {
        warnings.push(
            "The vertical points lie in a line, so the plane's tilt across it is unknown; \
             used a constant shift."
                .into(),
        );
        return (flat, false);
    }
    let slope_north = (nh * ee - eh * ne) / det;
    let slope_east = (eh * nn - nh * ne) / det;
    (
        Vertical {
            origin,
            constant,
            slope_north,
            slope_east,
        },
        true,
    )
}

/// One row of a saved calibration: points by name, or control typed as
/// coordinates.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PairSetup {
    /// Name of the GNSS-measured point.
    pub measured: String,
    /// Name of the control point, or its coordinates (`E,N[,Z]` in display order).
    pub control: String,
    pub horizontal: bool,
    pub vertical: bool,
}

/// A project's calibration: the pairs, the options and the transformation
/// last applied.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteCalibration {
    pub pairs: Vec<PairSetup>,
    pub options: CalibrationOptions,
    /// The transformation, once applied.
    pub applied: Option<Calibration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MGA-sized grid coordinates rotated, scaled and shifted to a local grid.
    fn truth() -> Calibration {
        let angle = (-0.75f64).to_radians(); // 0°45' clockwise
        let scale = 1.000_25;
        Calibration {
            horizontal: Some(Horizontal {
                origin: DVec2::new(331_000.0, 6_250_000.0),
                origin_control: DVec2::new(10_000.0, 5_000.0),
                a: scale * angle.cos(),
                b: scale * angle.sin(),
            }),
            vertical: Some(Vertical {
                origin: DVec2::new(331_000.0, 6_250_000.0),
                constant: -32.5,
                slope_north: 20e-6,
                slope_east: -15e-6,
            }),
        }
    }

    fn pairs(cal: &Calibration) -> Vec<Pair> {
        [
            (-400.0, -300.0, 12.0),
            (450.0, -250.0, 18.0),
            (380.0, 420.0, 25.0),
            (-350.0, 380.0, 9.0),
            (20.0, 30.0, 15.0),
        ]
        .iter()
        .map(|&(e, n, z)| {
            let measured = DVec3::new(331_000.0 + e, 6_250_000.0 + n, z);
            Pair {
                measured,
                control: cal.apply(measured),
                horizontal: true,
                vertical: true,
            }
        })
        .collect()
    }

    #[test]
    fn recovers_a_known_transformation() {
        let cal = truth();
        let fit = fit(&pairs(&cal), &CalibrationOptions::default()).expect("fit");
        let h = fit.calibration.horizontal.expect("horizontal");
        assert!((h.scale() - 1.000_25).abs() < 1e-12);
        assert!((h.rotation() - 0.75f64.to_radians()).abs() < 1e-12);
        let v = fit.calibration.vertical.expect("vertical");
        assert!((v.slope_north - 20e-6).abs() < 1e-12 && (v.slope_east + 15e-6).abs() < 1e-12);
        assert!(fit.max_horizontal().expect("h") < 1e-6);
        assert!(fit.max_vertical().expect("v") < 1e-6);
        assert_eq!((fit.horizontal_redundancy, fit.vertical_redundancy), (6, 2));
        assert_eq!(fit.vertical_method, Some(VerticalMethod::InclinedPlane));
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);
        // Anywhere on site, not only at the pairs.
        let p = DVec3::new(331_123.4, 6_249_876.5, 20.0);
        assert!(fit.calibration.apply(p).distance(cal.apply(p)) < 1e-6);
    }

    #[test]
    fn a_bad_pair_shows_in_its_residual() {
        let mut ps = pairs(&truth());
        ps[2].control.x += 0.08;
        let opts = CalibrationOptions::default();
        let fit = fit(&ps, &opts).expect("fit");
        let worst = fit
            .residuals
            .iter()
            .enumerate()
            .max_by(|a, b| {
                let l = |r: &Residual| r.horizontal.map_or(0.0, |v| v.length());
                l(a.1).total_cmp(&l(b.1))
            })
            .map(|(i, _)| i);
        assert_eq!(worst, Some(2));
        assert!(fit.outliers(&opts).contains(&2));
        assert!(
            fit.warnings
                .iter()
                .any(|w| w.contains("over the residual limits"))
        );
        // Turned off, the rest fit exactly again.
        ps[2].horizontal = false;
        let fit = super::fit(&ps, &opts).expect("fit");
        assert!(fit.max_horizontal().expect("h") < 1e-6);
        assert_eq!(fit.residuals[2].horizontal, None);
    }

    #[test]
    fn fixed_scale_keeps_scale_one() {
        let opts = CalibrationOptions {
            fixed_scale: true,
            ..Default::default()
        };
        let fit = fit(&pairs(&truth()), &opts).expect("fit");
        let h = fit.calibration.horizontal.expect("horizontal");
        assert!((h.scale() - 1.0).abs() < 1e-12);
        assert!((h.rotation() - 0.75f64.to_radians()).abs() < 1e-9);
        // The 250 ppm scale it can't absorb shows up as residuals (~0.1 m over 400 m).
        let worst = fit.max_horizontal().expect("h");
        assert!(worst > 0.05 && worst < 0.2, "{worst}");
    }

    #[test]
    fn few_points_fall_back_and_warn() {
        let all = pairs(&truth());
        // One pair: a shift only.
        let fit1 = fit(&all[..1], &CalibrationOptions::default()).expect("fit");
        let h = fit1.calibration.horizontal.expect("h");
        assert_eq!((h.a, h.b), (1.0, 0.0));
        assert!(fit1.residuals[0].horizontal.expect("r").length() < 1e-9);
        assert!(fit1.warnings.iter().any(|w| w.contains("shift only")));
        assert!(fit1.warnings.iter().any(|w| w.contains("constant shift")));
        // Two pairs: exact, no check.
        let fit2 = fit(&all[..2], &CalibrationOptions::default()).expect("fit");
        assert_eq!(fit2.horizontal_redundancy, 0);
        assert!(fit2.warnings.iter().any(|w| w.contains("no check")));
        // Heights only.
        let mut heights = all.clone();
        for p in &mut heights {
            p.horizontal = false;
        }
        let fit3 = fit(&heights, &CalibrationOptions::default()).expect("fit");
        assert!(fit3.calibration.horizontal.is_none() && fit3.calibration.vertical.is_some());
        // Nothing used.
        for p in &mut heights {
            p.vertical = false;
        }
        assert_eq!(
            fit(&heights, &CalibrationOptions::default()),
            Err(CalibrationError::NoPairs)
        );
    }

    #[test]
    fn collinear_heights_use_a_constant() {
        let mut ps = pairs(&truth());
        for (i, p) in ps.iter_mut().enumerate() {
            p.measured = DVec3::new(331_000.0 + 100.0 * i as f64, 6_250_000.0, p.measured.z);
        }
        let mut warnings = Vec::new();
        let refs: Vec<&Pair> = ps.iter().collect();
        let (v, plane) = fit_vertical(&refs, VerticalMethod::InclinedPlane, &mut warnings);
        assert!(!plane && v.slope_north == 0.0);
        assert!(warnings[0].contains("in a line"));
    }

    #[test]
    fn odd_scale_is_flagged() {
        let mut cal = truth();
        if let Some(h) = &mut cal.horizontal {
            h.a *= 1.01;
            h.b *= 1.01;
        }
        let fit = fit(&pairs(&cal), &CalibrationOptions::default()).expect("fit");
        assert!(fit.warnings.iter().any(|w| w.contains("ppm from 1")));
    }

    #[test]
    fn saved_calibration_round_trips() {
        let saved = SiteCalibration {
            pairs: vec![PairSetup {
                measured: "GPS1".into(),
                control: "CP1".into(),
                horizontal: true,
                vertical: false,
            }],
            options: CalibrationOptions::default(),
            applied: Some(truth()),
        };
        let json = serde_json::to_string(&saved).expect("json");
        let back: SiteCalibration = serde_json::from_str(&json).expect("parse");
        assert_eq!((&back.pairs, back.options), (&saved.pairs, saved.options));
        let p = DVec3::new(331_100.0, 6_250_100.0, 10.0);
        let (a, b) = (back.applied.expect("applied"), truth());
        assert!(a.apply(p).distance(b.apply(p)) < 1e-9);
        assert_eq!(
            VerticalMethod::from_key("plane"),
            Some(VerticalMethod::InclinedPlane)
        );
    }
}
