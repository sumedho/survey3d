//! Transverse Mercator projections (UTM, and national grids built on it such
//! as MGA or ETRS89-UTM) with point scale factor and grid convergence, using
//! Krüger's series to sixth order in n (as given by Karney, 2011), accurate to
//! well under a millimetre within a zone.
//!
//! Angles in the public API are degrees. Grid convergence is the grid bearing
//! of true north (the meridian through the point), so
//! `grid bearing = true bearing + convergence`.

use serde::{Deserialize, Serialize};

/// A reference ellipsoid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipsoid {
    pub name: &'static str,
    /// Semi-major axis (metres).
    pub a: f64,
    /// Inverse flattening.
    pub inv_f: f64,
}

impl Ellipsoid {
    pub const GRS80: Ellipsoid = Ellipsoid {
        name: "GRS80",
        a: 6_378_137.0,
        inv_f: 298.257_222_101,
    };
    pub const WGS84: Ellipsoid = Ellipsoid {
        name: "WGS84",
        a: 6_378_137.0,
        inv_f: 298.257_223_563,
    };
    pub const ALL: [Ellipsoid; 2] = [Ellipsoid::GRS80, Ellipsoid::WGS84];

    pub fn f(&self) -> f64 {
        1.0 / self.inv_f
    }

    pub fn e2(&self) -> f64 {
        let f = self.f();
        f * (2.0 - f)
    }

    /// Geometric mean radius of curvature √(ρν) at a latitude (degrees).
    pub fn mean_radius(&self, lat_deg: f64) -> f64 {
        let e2 = self.e2();
        let s = lat_deg.to_radians().sin();
        let w = (1.0 - e2 * s * s).sqrt();
        let rho = self.a * (1.0 - e2) / (w * w * w);
        let nu = self.a / w;
        (rho * nu).sqrt()
    }
}

/// A Transverse Mercator projection.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransverseMercator {
    #[serde(with = "ellipsoid_by_name")]
    pub ellipsoid: Ellipsoid,
    pub central_meridian: f64,
    pub latitude_of_origin: f64,
    pub scale: f64,
    pub false_easting: f64,
    pub false_northing: f64,
}

/// Ellipsoids are stored by name (or as a/1/f when not built in).
mod ellipsoid_by_name {
    use super::Ellipsoid;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Stored {
        name: String,
        a: f64,
        inv_f: f64,
    }

    pub fn serialize<S: Serializer>(e: &Ellipsoid, s: S) -> Result<S::Ok, S::Error> {
        Stored {
            name: e.name.into(),
            a: e.a,
            inv_f: e.inv_f,
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Ellipsoid, D::Error> {
        let s = Stored::deserialize(d)?;
        Ok(Ellipsoid::ALL
            .into_iter()
            .find(|e| e.name == s.name)
            .unwrap_or(Ellipsoid {
                name: "Custom",
                a: s.a,
                inv_f: s.inv_f,
            }))
    }
}

/// Grid coordinates with the scale factor and convergence at the point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridPoint {
    pub easting: f64,
    pub northing: f64,
    /// Point scale factor.
    pub scale: f64,
    /// Grid convergence (degrees).
    pub convergence: f64,
}

/// Geographic coordinates with the scale factor and convergence at the point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub scale: f64,
    pub convergence: f64,
}

struct Series {
    n: f64,
    /// Rectifying radius.
    big_a: f64,
    alpha: [f64; 6],
    beta: [f64; 6],
    delta: [f64; 6],
}

fn series(e: &Ellipsoid) -> Series {
    let f = e.f();
    let n = f / (2.0 - f);
    let n2 = n * n;
    let n3 = n2 * n;
    let n4 = n3 * n;
    let n5 = n4 * n;
    let n6 = n5 * n;
    Series {
        n,
        big_a: e.a / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0),
        alpha: [
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0
                + 7891.0 * n6 / 37800.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0
                - 1_983_433.0 * n6 / 1_935_360.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167_603.0 * n6 / 181_440.0,
            49561.0 * n4 / 161_280.0 - 179.0 * n5 / 168.0 + 6_601_661.0 * n6 / 7_257_600.0,
            34729.0 * n5 / 80640.0 - 3_418_889.0 * n6 / 1_995_840.0,
            212_378_941.0 * n6 / 319_334_400.0,
        ],
        beta: [
            n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0 - 81.0 * n5 / 512.0
                + 96199.0 * n6 / 604_800.0,
            n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0 + 46.0 * n5 / 105.0 - 1_118_711.0 * n6 / 3_870_720.0,
            17.0 * n3 / 480.0 - 37.0 * n4 / 840.0 - 209.0 * n5 / 4480.0 + 5569.0 * n6 / 90720.0,
            4397.0 * n4 / 161_280.0 - 11.0 * n5 / 504.0 - 830_251.0 * n6 / 7_257_600.0,
            4583.0 * n5 / 161_280.0 - 108_847.0 * n6 / 3_991_680.0,
            20_648_693.0 * n6 / 638_668_800.0,
        ],
        delta: [
            2.0 * n - 2.0 * n2 / 3.0 - 2.0 * n3 + 116.0 * n4 / 45.0 + 26.0 * n5 / 45.0 - 2854.0 * n6 / 675.0,
            7.0 * n2 / 3.0 - 8.0 * n3 / 5.0 - 227.0 * n4 / 45.0 + 2704.0 * n5 / 315.0 + 2323.0 * n6 / 945.0,
            56.0 * n3 / 15.0 - 136.0 * n4 / 35.0 - 1262.0 * n5 / 105.0 + 73814.0 * n6 / 2835.0,
            4279.0 * n4 / 630.0 - 332.0 * n5 / 35.0 - 399_572.0 * n6 / 14175.0,
            4174.0 * n5 / 315.0 - 144_838.0 * n6 / 6237.0,
            601_676.0 * n6 / 22275.0,
        ],
    }
}

impl TransverseMercator {
    /// A UTM zone (1–60) on an ellipsoid.
    pub fn utm(zone: u32, south: bool, ellipsoid: Ellipsoid) -> Self {
        Self {
            ellipsoid,
            central_meridian: f64::from(zone.clamp(1, 60)) * 6.0 - 183.0,
            latitude_of_origin: 0.0,
            scale: 0.9996,
            false_easting: 500_000.0,
            false_northing: if south { 10_000_000.0 } else { 0.0 },
        }
    }

    /// The UTM zone containing a longitude (degrees).
    pub fn utm_zone(longitude: f64) -> u32 {
        (((longitude + 180.0) / 6.0).floor() as i64).rem_euclid(60) as u32 + 1
    }

    /// Unscaled projection relative to the central meridian:
    /// (ξ-based northing, η-based easting, σ, τ, t, λ') in units of A.
    fn project(&self, s: &Series, lat: f64, dlon: f64) -> (f64, f64, f64, f64, f64) {
        let e = 2.0 * s.n.sqrt() / (1.0 + s.n);
        let sin_phi = lat.sin();
        let t = (sin_phi.atanh() - e * (e * sin_phi).atanh()).sinh();
        let xi_p = t.atan2(dlon.cos());
        let eta_p = (dlon.sin() / (1.0 + t * t).sqrt()).atanh();
        let (mut xi, mut eta, mut sigma, mut tau) = (xi_p, eta_p, 1.0, 0.0);
        for (j, a) in s.alpha.iter().enumerate() {
            let k = 2.0 * (j + 1) as f64;
            let (s2, c2) = (k * xi_p).sin_cos();
            let (sh, ch) = ((k * eta_p).sinh(), (k * eta_p).cosh());
            xi += a * s2 * ch;
            eta += a * c2 * sh;
            sigma += k * a * c2 * ch;
            tau += k * a * s2 * sh;
        }
        (xi, eta, sigma, tau, t)
    }

    /// Geographic (degrees) to grid.
    pub fn forward(&self, latitude: f64, longitude: f64) -> GridPoint {
        let s = series(&self.ellipsoid);
        let lat = latitude.to_radians();
        let dlon = (longitude - self.central_meridian).to_radians();
        let (xi, eta, sigma, tau, t) = self.project(&s, lat, dlon);
        let (xi0, ..) = self.project(&s, self.latitude_of_origin.to_radians(), 0.0);
        let k0a = self.scale * s.big_a;
        let easting = self.false_easting + k0a * eta;
        let northing = self.false_northing + k0a * (xi - xi0);
        let ratio = (1.0 - s.n) / (1.0 + s.n);
        let scale = k0a / self.ellipsoid.a
            * ((1.0 + (ratio * lat.tan()).powi(2)) * (sigma * sigma + tau * tau)
                / (t * t + dlon.cos().powi(2)))
            .sqrt();
        let root = (1.0 + t * t).sqrt();
        let convergence = (tau * root + sigma * t * dlon.tan()).atan2(sigma * root - tau * t * dlon.tan());
        GridPoint {
            easting,
            northing,
            scale,
            // Karney's γ is measured the other way round.
            convergence: -convergence.to_degrees(),
        }
    }

    /// Grid to geographic (degrees).
    pub fn inverse(&self, easting: f64, northing: f64) -> GeoPoint {
        let s = series(&self.ellipsoid);
        let (xi0, ..) = self.project(&s, self.latitude_of_origin.to_radians(), 0.0);
        let k0a = self.scale * s.big_a;
        let xi = (northing - self.false_northing) / k0a + xi0;
        let eta = (easting - self.false_easting) / k0a;
        let (mut xi_p, mut eta_p, mut sigma, mut tau) = (xi, eta, 1.0, 0.0);
        for (j, b) in s.beta.iter().enumerate() {
            let k = 2.0 * (j + 1) as f64;
            let (s2, c2) = (k * xi).sin_cos();
            let (sh, ch) = ((k * eta).sinh(), (k * eta).cosh());
            xi_p -= b * s2 * ch;
            eta_p -= b * c2 * sh;
            sigma -= k * b * c2 * ch;
            tau += k * b * s2 * sh;
        }
        let chi = (xi_p.sin() / eta_p.cosh()).asin();
        let mut lat = chi;
        for (j, d) in s.delta.iter().enumerate() {
            lat += d * (2.0 * (j + 1) as f64 * chi).sin();
        }
        let dlon = eta_p.sinh().atan2(xi_p.cos());
        let ratio = (1.0 - s.n) / (1.0 + s.n);
        let scale = k0a / self.ellipsoid.a
            * ((1.0 + (ratio * lat.tan()).powi(2)) * (xi_p.cos().powi(2) + eta_p.sinh().powi(2))
                / (sigma * sigma + tau * tau))
                .sqrt();
        let convergence =
            (tau + sigma * xi_p.tan() * eta_p.tanh()).atan2(sigma - tau * xi_p.tan() * eta_p.tanh());
        GeoPoint {
            latitude: lat.to_degrees(),
            longitude: self.central_meridian + dlon.to_degrees(),
            scale,
            convergence: -convergence.to_degrees(),
        }
    }

    /// Height (sea-level) factor for a mean ellipsoidal height at a latitude:
    /// ground distances × this × point scale factor = grid distances.
    pub fn height_factor(&self, latitude: f64, ellipsoidal_height: f64) -> f64 {
        let r = self.ellipsoid.mean_radius(latitude);
        r / (r + ellipsoidal_height)
    }

    /// Combined scale factor at a grid point for an ellipsoidal height.
    pub fn combined_factor(&self, easting: f64, northing: f64, ellipsoidal_height: f64) -> f64 {
        let g = self.inverse(easting, northing);
        g.scale * self.height_factor(g.latitude, ellipsoidal_height)
    }
}

/// Degrees, minutes, seconds to decimal degrees (sign from `negative`).
pub fn dms(negative: bool, d: f64, m: f64, s: f64) -> f64 {
    let v = d + m / 60.0 + s / 3600.0;
    if negative { -v } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flinders_peak() {
        // Geoscience Australia's published example: Flinders Peak (GDA94 / MGA zone 55).
        let lat = dms(true, 37.0, 57.0, 3.72030);
        let lon = dms(false, 144.0, 25.0, 29.52440);
        let tm = TransverseMercator::utm(55, true, Ellipsoid::GRS80);
        let g = tm.forward(lat, lon);
        assert!((g.easting - 273_741.297).abs() < 0.001, "{}", g.easting);
        assert!((g.northing - 5_796_489.777).abs() < 0.001, "{}", g.northing);
        // Scale and convergence checked numerically: a short step north along the meridian.
        let d = 1e-6;
        let g2 = tm.forward(lat + d, lon);
        let (de, dn) = (g2.easting - g.easting, g2.northing - g.northing);
        let e2 = Ellipsoid::GRS80.e2();
        let sl = lat.to_radians().sin();
        let rho = Ellipsoid::GRS80.a * (1.0 - e2) / (1.0 - e2 * sl * sl).powf(1.5);
        let k = (de * de + dn * dn).sqrt() / (rho * d.to_radians());
        assert!((g.scale - k).abs() < 1e-7, "{} vs {k}", g.scale);
        // The meridian's grid bearing is the convergence (negative here: west of the central meridian, south).
        let grid_bearing_of_north = de.atan2(dn).to_degrees();
        assert!(
            (g.convergence - grid_bearing_of_north).abs() < 1e-6,
            "{} vs {grid_bearing_of_north}",
            g.convergence
        );
        assert!(g.convergence < 0.0);
        let back = tm.inverse(g.easting, g.northing);
        assert!((back.latitude - lat).abs() < 1e-10 && (back.longitude - lon).abs() < 1e-10);
        assert!((back.scale - g.scale).abs() < 1e-12 && (back.convergence - g.convergence).abs() < 1e-9);
    }

    #[test]
    fn round_trips_across_a_zone() {
        let tm = TransverseMercator::utm(32, false, Ellipsoid::WGS84);
        for lat in [0.0, 12.5, 45.0, 60.0, 79.0] {
            for dlon in [-3.0, -1.0, 0.0, 2.5, 3.0] {
                let lon = tm.central_meridian + dlon;
                let g = tm.forward(lat, lon);
                let b = tm.inverse(g.easting, g.northing);
                assert!(
                    (b.latitude - lat).abs() < 1e-9 && (b.longitude - lon).abs() < 1e-9,
                    "{lat} {lon}"
                );
            }
        }
        // On the central meridian at the equator: E = 500 000, N = 0, k = k0.
        let g = tm.forward(0.0, tm.central_meridian);
        assert!((g.easting - 500_000.0).abs() < 1e-6 && g.northing.abs() < 1e-6);
        assert!((g.scale - 0.9996).abs() < 1e-12 && g.convergence.abs() < 1e-12);
        assert_eq!(TransverseMercator::utm_zone(144.4), 55);
        assert_eq!(TransverseMercator::utm_zone(-0.5), 30);
    }

    #[test]
    fn combined_factor() {
        let tm = TransverseMercator::utm(55, true, Ellipsoid::GRS80);
        // At 500 m above the ellipsoid near Flinders Peak the height factor is about 1 − 500/6.37e6.
        let h = tm.height_factor(-37.95, 500.0);
        assert!((h - (1.0 - 500.0 / 6_370_000.0)).abs() < 2e-6, "{h}");
        let c = tm.combined_factor(273_741.297, 5_796_489.777, 500.0);
        let k = tm.inverse(273_741.297, 5_796_489.777).scale;
        assert!((c - k * h).abs() < 1e-10 && (k - 1.000_23).abs() < 1e-5, "{k}");
        let json = serde_json::to_string(&tm).expect("json");
        let back: TransverseMercator = serde_json::from_str(&json).expect("back");
        assert_eq!(back, tm);
    }
}
