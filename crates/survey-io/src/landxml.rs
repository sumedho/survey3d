//! LandXML 1.2 surfaces (TINs): reading and writing `<Surface>` definitions.
//!
//! Points are `<P id="…">northing easting elevation</P>` and faces
//! `<F>id id id</F>` (faces marked invisible with `i="1"` are skipped). Feet
//! (`Imperial` units) are converted to metres on reading; files are written in
//! metres.

use std::collections::HashMap;
use std::fmt::Write as _;

use glam::DVec3;
use quick_xml::Reader;
use quick_xml::events::Event;
use survey_core::tin::Tin;

use crate::IoError;

/// A surface read from LandXML.
#[derive(Clone, Debug, PartialEq)]
pub struct LandXmlSurface {
    pub name: String,
    pub tin: Tin,
}

#[derive(Default)]
struct Building {
    name: String,
    ids: HashMap<String, u32>,
    tin: Tin,
    skipped_faces: usize,
}

/// Reads every TIN surface in a LandXML file. Returns the surfaces and warnings.
pub fn read_surfaces(bytes: &[u8]) -> Result<(Vec<LandXmlSurface>, Vec<String>), IoError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::NotText)?;
    let mut reader = Reader::from_str(text);
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let mut scale = 1.0;
    let mut current: Option<Building> = None;
    // The element whose text we are reading, with its attributes of interest.
    let mut text_of: Option<(&'static str, Option<String>, bool)> = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| IoError::Invalid(format!("LandXML: {e}")))?;
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name();
                let attr = |key: &str| -> Option<String> {
                    e.try_get_attribute(key).ok().flatten().and_then(|a| {
                        a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .ok()
                            .map(|v| v.into_owned())
                    })
                };
                match name.as_ref() {
                    "Imperial" => {
                        let unit = attr("linearUnit").unwrap_or_default().to_ascii_lowercase();
                        scale = if unit.contains("ussurvey") {
                            1200.0 / 3937.0
                        } else {
                            0.3048
                        };
                    }
                    "Metric" => scale = 1.0,
                    "Surface" => {
                        current = Some(Building {
                            name: attr("name").unwrap_or_else(|| format!("Surface {}", out.len() + 1)),
                            ..Default::default()
                        });
                    }
                    "P" if current.is_some() => text_of = Some(("P", attr("id"), false)),
                    "F" if current.is_some() => {
                        let invisible = attr("i").is_some_and(|v| v.trim() == "1");
                        text_of = Some(("F", None, invisible));
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                let (Some((kind, id, invisible)), Some(b)) = (&text_of, current.as_mut()) else {
                    continue;
                };
                let nums: Vec<&str> = t.as_ref().split_whitespace().collect();
                match *kind {
                    "P" => {
                        let v: Vec<f64> = nums.iter().filter_map(|s| crate::finite(s)).collect();
                        let Some(id) = id.clone() else {
                            continue;
                        };
                        if v.len() >= 3 {
                            // LandXML order: northing, easting, elevation.
                            b.ids.insert(id, b.tin.vertices.len() as u32);
                            b.tin.vertices.push(DVec3::new(v[1], v[0], v[2]) * scale);
                        }
                    }
                    _ => {
                        if *invisible {
                            b.skipped_faces += 1;
                            continue;
                        }
                        let f: Option<Vec<u32>> =
                            nums.iter().take(3).map(|s| b.ids.get(*s).copied()).collect();
                        match f {
                            Some(f) if f.len() == 3 => b.tin.triangles.push([f[0], f[1], f[2]]),
                            _ => b.skipped_faces += 1,
                        }
                    }
                }
            }
            Event::End(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    "P" | "F" => text_of = None,
                    "Surface" => {
                        if let Some(b) = current.take() {
                            if b.skipped_faces > 0 {
                                warnings.push(format!(
                                    "{}: skipped {} invisible or invalid faces",
                                    b.name, b.skipped_faces
                                ));
                            }
                            if b.tin.triangles.is_empty() {
                                warnings.push(format!("{}: no faces, not imported", b.name));
                            } else {
                                out.push(LandXmlSurface {
                                    name: b.name,
                                    tin: orient(b.tin),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if out.is_empty() {
        return Err(IoError::NoPoints(" — no TIN surfaces in the LandXML file".into()));
    }
    Ok((out, warnings))
}

/// Makes every triangle anticlockwise in plan.
fn orient(mut tin: Tin) -> Tin {
    for t in &mut tin.triangles {
        let [a, b, c] = t.map(|i| tin.vertices[i as usize].truncate());
        if (b - a).perp_dot(c - a) < 0.0 {
            t.swap(1, 2);
        }
    }
    tin
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The coordinate system a LandXML file is written in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CrsInfo {
    pub name: String,
    pub epsg: Option<u32>,
    /// OGC WKT of the projection.
    pub wkt: Option<String>,
    pub ellipsoid: Option<String>,
    /// The geoid model heights are on.
    pub geoid: Option<String>,
}

/// `<CoordinateSystem>` (LandXML 1.2 attributes), when one is known.
fn write_crs(s: &mut String, crs: Option<&CrsInfo>) {
    let Some(c) = crs else { return };
    let mut attrs = format!(r#"name="{}""#, escape(&c.name));
    if let Some(code) = c.epsg {
        attrs.push_str(&format!(r#" epsgCode="{code}""#));
    }
    if let Some(e) = &c.ellipsoid {
        attrs.push_str(&format!(r#" ellipsoidName="{}""#, escape(e)));
    }
    if let Some(g) = &c.geoid {
        attrs.push_str(&format!(r#" geoidName="{}""#, escape(g)));
    }
    if let Some(w) = &c.wkt {
        attrs.push_str(&format!(r#" ogcWktCode="{}""#, escape(w)));
    }
    let _ = writeln!(s, "  <CoordinateSystem {attrs}/>");
}

/// Writes surfaces as LandXML 1.2 (metres).
pub fn write_surfaces(project: &str, crs: Option<&CrsInfo>, surfaces: &[(&str, &Tin)]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        s,
        r#"<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2" language="English">"#
    );
    let _ = writeln!(
        s,
        r#"  <Units><Metric areaUnit="squareMeter" linearUnit="meter" volumeUnit="cubicMeter" temperatureUnit="celsius" pressureUnit="milliBars" angularUnit="decimal degrees" directionUnit="decimal degrees"/></Units>"#
    );
    write_crs(&mut s, crs);
    let _ = writeln!(s, r#"  <Project name="{}"/>"#, escape(project));
    let _ = writeln!(
        s,
        r#"  <Application name="survey3d" version="{}"/>"#,
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(s, "  <Surfaces>");
    for (name, tin) in surfaces {
        let _ = writeln!(s, r#"    <Surface name="{}">"#, escape(name));
        let _ = writeln!(s, r#"      <Definition surfType="TIN">"#);
        let _ = writeln!(s, "        <Pnts>");
        for (i, v) in tin.vertices.iter().enumerate() {
            let _ = writeln!(
                s,
                r#"          <P id="{}">{:.4} {:.4} {:.4}</P>"#,
                i + 1,
                v.y,
                v.x,
                v.z
            );
        }
        let _ = writeln!(s, "        </Pnts>");
        let _ = writeln!(s, "        <Faces>");
        for t in &tin.triangles {
            let _ = writeln!(s, "          <F>{} {} {}</F>", t[0] + 1, t[1] + 1, t[2] + 1);
        }
        let _ = writeln!(s, "        </Faces>");
        let _ = writeln!(s, "      </Definition>");
        let _ = writeln!(s, "    </Surface>");
    }
    let _ = writeln!(s, "  </Surfaces>");
    let _ = writeln!(s, "</LandXML>");
    s
}

/// A parcel to write: its boundary (closed, with bulges) and stated area.
pub struct ParcelGeometry<'a> {
    pub name: &'a str,
    pub description: &'a str,
    /// Plan area in square metres.
    pub area: f64,
    pub vertices: &'a [DVec3],
    pub bulges: &'a [f64],
}

/// Writes parcels as LandXML 1.2: shared corners as `<CgPoints>`, and each
/// parcel's boundary as `<CoordGeom>` lines and curves referring to them.
pub fn write_parcels(project: &str, crs: Option<&CrsInfo>, parcels: &[ParcelGeometry<'_>]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        s,
        r#"<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2" language="English">"#
    );
    let _ = writeln!(
        s,
        r#"  <Units><Metric areaUnit="squareMeter" linearUnit="meter" volumeUnit="cubicMeter" temperatureUnit="celsius" pressureUnit="milliBars" angularUnit="decimal degrees" directionUnit="decimal degrees"/></Units>"#
    );
    write_crs(&mut s, crs);
    let _ = writeln!(s, r#"  <Project name="{}"/>"#, escape(project));
    let _ = writeln!(
        s,
        r#"  <Application name="survey3d" version="{}"/>"#,
        env!("CARGO_PKG_VERSION")
    );
    // Corners shared between parcels become one point (millimetre match).
    let mut names: HashMap<(i64, i64), usize> = HashMap::new();
    let mut points: Vec<DVec3> = Vec::new();
    let mut point_name = |p: DVec3, names: &mut HashMap<(i64, i64), usize>| -> usize {
        let key = ((p.x * 1000.0).round() as i64, (p.y * 1000.0).round() as i64);
        *names.entry(key).or_insert_with(|| {
            points.push(p);
            points.len()
        })
    };
    let mut bodies = String::new();
    for parcel in parcels {
        let n = parcel.vertices.len();
        let _ = writeln!(
            bodies,
            r#"    <Parcel name="{}" desc="{}" area="{:.3}" class="lot">"#,
            escape(parcel.name),
            escape(parcel.description),
            parcel.area
        );
        let _ = writeln!(bodies, "      <CoordGeom>");
        for i in 0..n {
            let (a, b) = (parcel.vertices[i], parcel.vertices[(i + 1) % n]);
            let (pa, pb) = (point_name(a, &mut names), point_name(b, &mut names));
            let bulge = parcel.bulges.get(i).copied().unwrap_or(0.0);
            match survey_core::arc::Arc::from_bulge(a, b, bulge) {
                Some(arc) => {
                    let _ = writeln!(
                        bodies,
                        r#"        <Curve rot="{}" radius="{:.4}" length="{:.4}"><Start pntRef="{pa}"/><Center>{:.4} {:.4}</Center><End pntRef="{pb}"/></Curve>"#,
                        if arc.sweep < 0.0 { "cw" } else { "ccw" },
                        arc.radius,
                        arc.length_2d(),
                        arc.center.y,
                        arc.center.x
                    );
                }
                None => {
                    let _ = writeln!(
                        bodies,
                        r#"        <Line><Start pntRef="{pa}"/><End pntRef="{pb}"/></Line>"#
                    );
                }
            }
        }
        let _ = writeln!(bodies, "      </CoordGeom>");
        let _ = writeln!(bodies, "    </Parcel>");
    }
    let _ = writeln!(s, "  <CgPoints>");
    for (i, p) in points.iter().enumerate() {
        let _ = writeln!(
            s,
            r#"    <CgPoint name="{}">{:.4} {:.4} {:.4}</CgPoint>"#,
            i + 1,
            p.y,
            p.x,
            p.z
        );
    }
    let _ = writeln!(s, "  </CgPoints>");
    let _ = writeln!(s, "  <Parcels>");
    s.push_str(&bodies);
    let _ = writeln!(s, "  </Parcels>");
    let _ = writeln!(s, "</LandXML>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Tin {
        Tin {
            vertices: vec![
                DVec3::new(512_000.0, 6_912_000.0, 10.0),
                DVec3::new(512_010.0, 6_912_000.0, 11.0),
                DVec3::new(512_010.0, 6_912_010.0, 12.0),
                DVec3::new(512_000.0, 6_912_010.0, 13.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        }
    }

    #[test]
    fn round_trip() {
        let tin = square();
        let crs = CrsInfo {
            name: "MGA2020 zone 56".into(),
            epsg: Some(7856),
            geoid: Some("AUSGeoid2020".into()),
            ..Default::default()
        };
        let xml = write_surfaces("Job & Co", Some(&crs), &[("Existing <EG>", &tin)]);
        assert!(
            xml.contains(
                r#"<CoordinateSystem name="MGA2020 zone 56" epsgCode="7856" geoidName="AUSGeoid2020"/>"#
            ),
            "{xml}"
        );
        let (surfaces, warnings) = read_surfaces(xml.as_bytes()).expect("read");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0].name, "Existing <EG>");
        assert_eq!(surfaces[0].tin, tin);
    }

    #[test]
    fn reads_other_writers() {
        // Ids that are not 1..n, an invisible face, feet, and clockwise faces.
        let xml = r#"<?xml version="1.0"?>
<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2">
  <Units><Imperial linearUnit="USSurveyFoot"/></Units>
  <Surfaces>
    <Surface name="EG">
      <Definition surfType="TIN">
        <Pnts>
          <P id="10">1000 2000 100</P>
          <P id="20">1000 2010 101</P>
          <P id="30">1010 2010 102</P>
          <P id="40">1010 2000 103</P>
        </Pnts>
        <Faces>
          <F>10 30 20</F>
          <F i="1">10 40 30</F>
        </Faces>
      </Definition>
    </Surface>
  </Surfaces>
</LandXML>"#;
        let (surfaces, warnings) = read_surfaces(xml.as_bytes()).expect("read");
        assert_eq!(warnings.len(), 1);
        let tin = &surfaces[0].tin;
        assert_eq!(tin.triangles.len(), 1);
        let ft = 1200.0 / 3937.0;
        assert!((tin.vertices[0] - DVec3::new(2000.0, 1000.0, 100.0) * ft).length() < 1e-9);
        // Re-oriented anticlockwise.
        let [a, b, c] = tin.triangles[0].map(|i| tin.vertices[i as usize].truncate());
        assert!((b - a).perp_dot(c - a) > 0.0);
        assert!(read_surfaces(b"<LandXML/>").is_err());
    }

    #[test]
    fn parcels_share_corners_and_parse() {
        let a = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(20.0, 30.0, 0.0),
            DVec3::new(0.0, 30.0, 0.0),
        ];
        let b = [
            DVec3::new(20.0, 0.0, 0.0),
            DVec3::new(40.0, 0.0, 0.0),
            DVec3::new(40.0, 30.0, 0.0),
            DVec3::new(20.0, 30.0, 0.0),
        ];
        let bulges = [0.0, 0.0, 0.2, 0.0];
        let xml = write_parcels(
            "Estate",
            None,
            &[
                ParcelGeometry {
                    name: "Lot 1",
                    description: "",
                    area: 600.0,
                    vertices: &a,
                    bulges: &[],
                },
                ParcelGeometry {
                    name: "Lot 2",
                    description: "corner",
                    area: 612.3,
                    vertices: &b,
                    bulges: &bulges,
                },
            ],
        );
        // Two shared corners: 6 unique points.
        assert_eq!(xml.matches("<CgPoint ").count(), 6);
        assert_eq!(xml.matches("<Line>").count(), 7);
        assert_eq!(xml.matches("<Curve ").count(), 1);
        // Well-formed XML.
        let mut reader = Reader::from_str(&xml);
        loop {
            match reader.read_event() {
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
}
