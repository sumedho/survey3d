//! Grid ↔ geographic conversion with the project's projection and geoid:
//! latitude and longitude, grid coordinates, orthometric and ellipsoidal
//! heights, point scale factor, grid convergence, and height and combined
//! scale factors.

use std::fmt::Write as _;

use glam::DVec3;
use survey_core::projection::TransverseMercator;
use survey_core::{Angle, Conventions};
use survey_model::Document;
use survey_model::ground::Separation;

/// One converted position.
#[derive(Clone, Debug)]
pub struct Converted {
    pub name: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub easting: f64,
    pub northing: f64,
    /// Height above the geoid (H).
    pub height: f64,
    /// Height above the ellipsoid (h = H + N).
    pub ellipsoidal: f64,
    pub separation: Separation,
    pub scale: f64,
    /// Grid convergence (degrees): grid bearing = true bearing + convergence.
    pub convergence: f64,
    pub height_factor: f64,
}

impl Converted {
    pub fn combined(&self) -> f64 {
        self.scale * self.height_factor
    }
}

/// Which height a geographic position comes with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeightKind {
    /// Above the geoid (H), as the project's points are.
    Orthometric,
    /// Above the ellipsoid (h), as GNSS gives it.
    Ellipsoidal,
}

fn projection(doc: &Document) -> Result<TransverseMercator, String> {
    doc.meta()
        .coord_system
        .projection()
        .ok_or_else(|| "Set a projection first (Project → Settings → Coordinates).".to_string())
}

fn finish(
    doc: &Document,
    tm: &TransverseMercator,
    easting: f64,
    northing: f64,
    height: f64,
    name: Option<String>,
) -> Converted {
    let g = tm.inverse(easting, northing);
    let separation = doc.separation_at(easting, northing);
    let ellipsoidal = height + separation.n;
    Converted {
        name,
        latitude: g.latitude,
        longitude: g.longitude,
        easting,
        northing,
        height,
        ellipsoidal,
        separation,
        scale: g.scale,
        convergence: g.convergence,
        height_factor: tm.height_factor(g.latitude, ellipsoidal),
    }
}

/// A grid position with a height above the geoid.
pub fn from_grid(doc: &Document, p: DVec3, name: Option<String>) -> Result<Converted, String> {
    let tm = projection(doc)?;
    Ok(finish(doc, &tm, p.x, p.y, p.z, name))
}

/// A latitude and longitude (degrees) with a height of either kind.
pub fn from_geographic(
    doc: &Document,
    latitude: f64,
    longitude: f64,
    height: f64,
    kind: HeightKind,
    name: Option<String>,
) -> Result<Converted, String> {
    if !(-90.0..=90.0).contains(&latitude) || !(-360.0..=360.0).contains(&longitude) {
        return Err(format!("{latitude}, {longitude} is not a latitude and longitude"));
    }
    let tm = projection(doc)?;
    let g = tm.forward(latitude, longitude);
    let orthometric = match kind {
        HeightKind::Orthometric => height,
        HeightKind::Ellipsoidal => height - doc.separation_at(g.easting, g.northing).n,
    };
    let mut c = finish(doc, &tm, g.easting, g.northing, orthometric, name);
    // Keep the latitude and longitude exactly as given.
    (c.latitude, c.longitude) = (latitude, longitude);
    Ok(c)
}

/// Latitude or longitude as signed d°m's.sssss" (finer than survey angles need).
pub fn geodetic(degrees: f64) -> String {
    let d = Angle::from_degrees(degrees.abs()).to_dms(5);
    format!(
        "{}{}°{:02}'{:08.5}\"",
        if degrees < 0.0 { "-" } else { "" },
        d.degrees,
        d.minutes,
        d.seconds
    )
}

fn signed_angle(conv: &Conventions, degrees: f64) -> String {
    let a = conv.format_angle(Angle::from_degrees(degrees.abs()));
    if degrees < 0.0 { format!("-{a}") } else { a }
}

/// The conversion as a few labelled lines.
pub fn report(conv: &Conventions, r: &Converted) -> String {
    let mut s = String::new();
    if let Some(n) = &r.name {
        let _ = writeln!(s, "Point            {n}");
    }
    let _ = writeln!(
        s,
        "Latitude         {}  ({:.9}°)",
        geodetic(r.latitude),
        r.latitude
    );
    let _ = writeln!(
        s,
        "Longitude        {}  ({:.9}°)",
        geodetic(r.longitude),
        r.longitude
    );
    let (ea, nb) = conv.horizontal_labels();
    let (a, b) = conv.horizontal_values(DVec3::new(r.easting, r.northing, 0.0));
    let _ = writeln!(s, "{ea:<16} {}", conv.format_length(a));
    let _ = writeln!(s, "{nb:<16} {}", conv.format_length(b));
    let _ = writeln!(s, "Height (H)       {}", conv.format_length(r.height));
    let source = r
        .separation
        .model
        .as_deref()
        .map_or_else(|| "project constant".to_string(), str::to_string);
    let _ = writeln!(
        s,
        "Geoid N          {}  ({source})",
        conv.format_length(r.separation.n)
    );
    let _ = writeln!(s, "Ellipsoidal (h)  {}", conv.format_length(r.ellipsoidal));
    let _ = writeln!(s, "Point scale      {:.9}", r.scale);
    let _ = writeln!(s, "Convergence      {}", signed_angle(conv, r.convergence));
    let _ = writeln!(s, "Height factor    {:.9}", r.height_factor);
    let _ = writeln!(s, "Combined factor  {:.9}", r.combined());
    let _ = write!(s, "Grid distance = ground distance × combined factor");
    s
}
