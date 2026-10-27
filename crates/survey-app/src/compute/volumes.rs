//! Volumes between two surfaces (or a surface and a level) and stockpile
//! volumes, with their reports.

use std::fmt::Write as _;

use survey_core::Conventions;
use survey_core::tin::Tin;
use survey_core::volume::{Datum, Stockpile, StockpileBase, VolumeResult, stockpile, volume};
use survey_model::surface::outline;
use survey_model::{Document, Entity, EntityId, Surface, SurfaceId, SurfaceStyle};

/// One side of a volume: a surface or a level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Side {
    Surface(SurfaceId),
    Level(f64),
}

impl Side {
    pub fn describe(self, doc: &Document) -> String {
        match self {
            Side::Surface(id) => doc.surface(id).map_or("?".into(), |s| s.name.clone()),
            Side::Level(z) => format!("level {}", doc.conventions().format_length(z)),
        }
    }

    fn datum<'a>(self, doc: &'a Document, label: &str) -> Result<Datum<'a>, String> {
        match self {
            Side::Surface(id) => doc
                .surface(id)
                .map(|s| Datum::Surface(&s.tin))
                .ok_or_else(|| format!("Choose the {label} surface")),
            Side::Level(z) => Ok(Datum::Level(z)),
        }
    }
}

/// A volume in the project's units, e.g. `1234.567 m³`.
pub fn format_volume(conv: &Conventions, v: f64) -> String {
    let mpu = conv.linear_unit.metres_per_unit();
    format!("{:.3} {}³", v / mpu.powi(3), conv.linear_unit.suffix())
}

pub struct Volumes {
    pub result: VolumeResult,
    /// The difference surface, top − base.
    pub difference: Tin,
    pub report: String,
}

/// Fill (top above base), cut and net volumes, within `boundary` (a closed
/// polyline) or wherever both sides exist; `density` (t/m³) adds tonnes.
pub fn volumes(
    doc: &Document,
    base: Side,
    top: Side,
    boundary: Option<EntityId>,
    density: Option<f64>,
) -> Result<Volumes, String> {
    let outline = match boundary {
        Some(id) => Some(outline(doc, id).ok_or("The boundary must be a closed polyline")?),
        None => None,
    };
    let (result, difference) = volume(
        base.datum(doc, "base")?,
        top.datum(doc, "top")?,
        outline.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    let report = volume_report(doc, base, top, boundary, &result, density);
    Ok(Volumes {
        result,
        difference,
        report,
    })
}

/// A cut/fill (isopach) surface of top − base.
pub fn cut_fill_surface(doc: &Document, base: Side, top: Side, difference: Tin) -> Surface {
    Surface {
        name: format!("{} − {}", top.describe(doc), base.describe(doc)),
        definition: None,
        tin: difference,
        style: SurfaceStyle {
            isopach: true,
            ..SurfaceStyle::default()
        },
    }
}

fn tonnes_line(r: &VolumeResult, density: Option<f64>) -> Option<String> {
    density.map(|d| {
        format!(
            "Density {d} t/m³: fill {:.1} t, cut {:.1} t, net {:.1} t",
            r.fill * d,
            r.cut * d,
            r.net() * d
        )
    })
}

fn volume_report(
    doc: &Document,
    base: Side,
    top: Side,
    boundary: Option<EntityId>,
    r: &VolumeResult,
    density: Option<f64>,
) -> String {
    let conv = doc.conventions();
    let vol = |v: f64| format_volume(conv, v);
    let mut s = String::new();
    let _ = writeln!(s, "VOLUME");
    let _ = writeln!(s, "Base       {}", base.describe(doc));
    let _ = writeln!(s, "Top        {}", top.describe(doc));
    let boundary = boundary
        .and_then(|id| doc.entity(id))
        .map_or("whole overlap".to_string(), |e| match e {
            Entity::Polyline(l) if !l.name.is_empty() => l.name.clone(),
            _ => "selected polyline".into(),
        });
    let _ = writeln!(s, "Boundary   {boundary}");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Fill       {}   over {}",
        vol(r.fill),
        conv.format_area(r.fill_area)
    );
    let _ = writeln!(
        s,
        "Cut        {}   over {}",
        vol(r.cut),
        conv.format_area(r.cut_area)
    );
    let _ = writeln!(s, "Net        {}   (fill − cut)", vol(r.net()));
    let _ = writeln!(s, "Area       {}", conv.format_area(r.area));
    if let Some(t) = tonnes_line(r, density) {
        let _ = writeln!(s, "{t}");
    }
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Fill: top above base. Cut: top below base. Computed exactly on the merged triangulations."
    );
    s
}

/// A stockpile's volume: `surface` above its base inside the closed `toe`
/// polyline. Returns the result and its report (titled with `name`).
pub fn stockpile_volume(
    doc: &Document,
    surface: SurfaceId,
    toe: EntityId,
    base: StockpileBase,
    density: Option<f64>,
    name: &str,
) -> Result<(Stockpile, String), String> {
    let s = doc.surface(surface).ok_or("Choose the surface")?;
    let toe_outline = outline(doc, toe).ok_or("The toe must be a closed polyline")?;
    let pile = stockpile(&s.tin, &toe_outline, base).map_err(|e| e.to_string())?;
    let report = stockpile_report(doc, &s.name, base, &pile, density, name);
    Ok((pile, report))
}

fn stockpile_report(
    doc: &Document,
    surface: &str,
    base: StockpileBase,
    s: &Stockpile,
    density: Option<f64>,
    name: &str,
) -> String {
    let conv = doc.conventions();
    let vol = |v: f64| format_volume(conv, v);
    let name = if name.trim().is_empty() {
        "Stockpile"
    } else {
        name.trim()
    };
    let base = match base {
        StockpileBase::Toe => "triangulated toe".to_string(),
        StockpileBase::LowestToe => format!("lowest toe {}", conv.format_length(s.base_min)),
        StockpileBase::Level(z) => format!("level {}", conv.format_length(z)),
    };
    let r = &s.result;
    let mut t = String::new();
    let _ = writeln!(t, "STOCKPILE  {name}");
    let _ = writeln!(t, "Surface      {surface}");
    let _ = writeln!(t, "Base         {base}");
    let _ = writeln!(t);
    let _ = writeln!(t, "Volume       {}", vol(r.net()));
    if let Some(d) = density {
        let _ = writeln!(t, "Density      {d} t/m³");
        let _ = writeln!(t, "Tonnes       {:.1} t", r.net() * d);
    }
    let _ = writeln!(t, "Above base   {}", vol(r.fill));
    let _ = writeln!(t, "Below base   {}", vol(r.cut));
    let _ = writeln!(t, "Plan area    {}", conv.format_area(r.area));
    let _ = writeln!(
        t,
        "Toe heights  {} to {}",
        conv.format_length(s.base_min),
        conv.format_length(s.base_max)
    );
    t
}
