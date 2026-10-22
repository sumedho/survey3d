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
                        let v: Vec<f64> = nums.iter().filter_map(|s| s.parse().ok()).collect();
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

/// Writes surfaces as LandXML 1.2 (metres).
pub fn write_surfaces(project: &str, surfaces: &[(&str, &Tin)]) -> String {
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
        let xml = write_surfaces("Job & Co", &[("Existing <EG>", &tin)]);
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
}
