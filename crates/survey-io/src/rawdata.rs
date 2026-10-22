//! Raw total-station data: Leica GSI (GSI-8 and GSI-16) and Trimble JobXML,
//! read into instrument setups for the network adjustment.

use glam::DVec3;
use quick_xml::Reader;
use quick_xml::events::Event;
use survey_core::Angle;
use survey_core::network::{FieldData, RawObservation, Setup};

use crate::IoError;

/// Puts a face-right pointing on face left (zenith over 180°).
fn face_left(hz: Option<Angle>, zenith: Option<Angle>) -> (Option<Angle>, Option<Angle>) {
    match zenith {
        Some(z) if z.degrees() > 180.0 => (
            hz.map(|h| (h + Angle::from_degrees(180.0)).normalized()),
            Some(Angle::from_degrees(360.0) - z),
        ),
        _ => (hz, zenith),
    }
}

// ---------------------------------------------------------------- GSI

/// One GSI word: code, unit digit, sign and data.
struct Word<'a> {
    wi: u32,
    unit: char,
    negative: bool,
    data: &'a str,
}

fn words(line: &str) -> impl Iterator<Item = Word<'_>> {
    line.trim_start_matches('*').split_whitespace().filter_map(|w| {
        if w.len() < 8 || !w.is_char_boundary(7) {
            return None;
        }
        let wi = w.get(0..2)?.parse().ok()?;
        let unit = w.as_bytes()[5] as char;
        let negative = w.as_bytes()[6] == b'-';
        Some(Word {
            wi,
            unit,
            negative,
            data: &w[7..],
        })
    })
}

impl Word<'_> {
    fn number(&self) -> Option<f64> {
        let v: f64 = self
            .data
            .trim_start_matches('0')
            .parse()
            .or_else(|_| {
                if self.data.chars().all(|c| c == '0') {
                    Ok(0.0)
                } else {
                    Err(())
                }
            })
            .ok()?;
        Some(if self.negative { -v } else { v })
    }

    /// Distances: unit 0 mm, 1 mft, 6 0.1 mm, 7 0.1 mft, 8 0.01 mm.
    fn length(&self) -> Option<f64> {
        let v = self.number()?;
        Some(match self.unit {
            '1' => v * 0.3048e-3,
            '6' => v * 1e-4,
            '7' => v * 0.3048e-4,
            '8' => v * 1e-5,
            _ => v * 1e-3,
        })
    }

    /// Angles: unit 2 gon, 3 decimal degrees, 4 DDDMMSSs, 5 mil; five decimals.
    fn angle(&self) -> Option<Angle> {
        let v = self.number()?;
        Some(match self.unit {
            '2' => Angle::from_gons(v * 1e-5),
            '3' => Angle::from_degrees(v * 1e-5),
            '4' => {
                let a = v.abs();
                let d = (a / 100_000.0).floor();
                let m = ((a - d * 100_000.0) / 1000.0).floor();
                let s = (a - d * 100_000.0 - m * 1000.0) / 10.0;
                Angle::from_dms(v < 0.0, d, m, s)
            }
            '5' => Angle::from_degrees(v * 1e-5 * 360.0 / 6400.0),
            _ => Angle::from_gons(v * 1e-5),
        })
    }

    /// Point names: leading zeros are padding.
    fn name(&self) -> String {
        let t = self.data.trim_start_matches('0');
        if t.is_empty() { "0".into() } else { t.to_string() }
    }
}

/// Reads Leica GSI-8 or GSI-16. A line carrying an instrument height (88) or
/// station coordinates (84–86) starts a setup at its point (11); lines with
/// angles or distances are pointings from the current setup; lines with
/// target coordinates (81–83) only are recorded as points.
pub fn read_gsi(name: &str, bytes: &[u8]) -> Result<FieldData, IoError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::NotText)?;
    let mut data = FieldData {
        name: name.to_string(),
        format: "Leica GSI".into(),
        ..Default::default()
    };
    let mut current: Option<Setup> = None;
    let mut target_height = 0.0;
    let mut recognised = 0;
    for line in text.lines() {
        let mut point = None;
        let (mut hz, mut v, mut sd, mut hd, mut dh) = (None, None, None, None, None);
        let (mut e, mut n, mut h) = (None, None, None);
        let (mut se, mut sn, mut sh, mut ih) = (None, None, None, None);
        let mut any = false;
        for w in words(line) {
            any = true;
            match w.wi {
                11 => point = Some(w.name()),
                21 => hz = w.angle(),
                22 => v = w.angle(),
                31 => sd = w.length(),
                32 => hd = w.length(),
                33 => dh = w.length(),
                81 => e = w.length(),
                82 => n = w.length(),
                83 => h = w.length(),
                84 => se = w.length(),
                85 => sn = w.length(),
                86 => sh = w.length(),
                87 => {
                    if let Some(t) = w.length() {
                        target_height = t;
                    }
                }
                88 => ih = w.length(),
                _ => {}
            }
        }
        if !any {
            continue;
        }
        recognised += 1;
        let Some(point) = point else { continue };
        if ih.is_some() || se.is_some() || sn.is_some() {
            if let Some(s) = current.take()
                && !s.observations.is_empty()
            {
                data.setups.push(s);
            }
            if let (Some(x), Some(y)) = (se, sn) {
                data.points.retain(|(p, _)| *p != point);
                data.points
                    .push((point.clone(), DVec3::new(x, y, sh.unwrap_or(0.0))));
            }
            current = Some(Setup {
                station: point,
                instrument_height: ih.unwrap_or(0.0),
                orientation: None,
                observations: Vec::new(),
            });
            continue;
        }
        if hz.is_some() || sd.is_some() || hd.is_some() {
            let (hz, zenith) = face_left(hz, v);
            let (zenith, distance) = match (zenith, sd, hd) {
                (Some(z), Some(s), _) => (Some(z), Some(s)),
                (_, _, Some(h)) => (None, Some(h)),
                (z, None, None) => (z, None),
                (None, Some(s), None) => (None, Some(s)),
            };
            if let Some(s) = current.as_mut() {
                s.observations.push(RawObservation {
                    target: point.clone(),
                    hz,
                    zenith,
                    distance,
                    target_height,
                    dz: if zenith.is_none() { dh } else { None },
                });
            }
        }
        if let (Some(x), Some(y)) = (e, n) {
            data.points.retain(|(p, _)| *p != point);
            data.points.push((point, DVec3::new(x, y, h.unwrap_or(0.0))));
        }
    }
    if let Some(s) = current
        && !s.observations.is_empty()
    {
        data.setups.push(s);
    }
    if recognised == 0 {
        return Err(IoError::Invalid("GSI: no GSI words found".into()));
    }
    if data.setups.is_empty() && data.points.is_empty() {
        return Err(IoError::Invalid(
            "GSI: no setups (instrument height or station coordinates) or points found".into(),
        ));
    }
    Ok(data)
}

// ---------------------------------------------------------------- JobXML

/// A minimal element tree.
#[derive(Default, Debug)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim()).filter(|t| !t.is_empty())
    }
    fn num(&self, name: &str) -> Option<f64> {
        self.text_of(name)?.parse().ok()
    }
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
    fn deleted(&self) -> bool {
        self.text_of("Deleted")
            .is_some_and(|d| d.eq_ignore_ascii_case("true"))
    }
}

fn parse_tree(text: &str) -> Result<Node, IoError> {
    let mut reader = Reader::from_str(text);
    let mut stack = vec![Node::default()];
    let open = |e: &quick_xml::events::BytesStart<'_>| Node {
        name: e.local_name().as_ref().to_string(),
        attrs: e
            .attributes()
            .flatten()
            .filter_map(|a| {
                let key = a.key.local_name().as_ref().to_string();
                let v = a
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()?
                    .into_owned();
                Some((key, v))
            })
            .collect(),
        ..Default::default()
    };
    loop {
        match reader
            .read_event()
            .map_err(|e| IoError::Invalid(format!("JobXML: {e}")))?
        {
            Event::Start(e) => stack.push(open(&e)),
            Event::Empty(e) => {
                let n = open(&e);
                if let Some(top) = stack.last_mut() {
                    top.children.push(n);
                }
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(t.as_ref());
                }
            }
            Event::End(_) if stack.len() > 1 => {
                let n = stack.pop().expect("stack");
                if let Some(top) = stack.last_mut() {
                    top.children.push(n);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(stack.swap_remove(0))
}

fn grid(n: &Node) -> Option<DVec3> {
    let g = n.child("Grid").or_else(|| n.child("ComputedGrid"))?;
    Some(DVec3::new(
        g.num("East")?,
        g.num("North")?,
        g.num("Elevation").unwrap_or(0.0),
    ))
}

/// Reads Trimble JobXML: station, target and point records from the field
/// book (angles in degrees, distances in metres, as JobXML stores them) and
/// coordinates from the reductions or keyed-in points.
pub fn read_jobxml(name: &str, bytes: &[u8]) -> Result<FieldData, IoError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::NotText)?;
    let root = parse_tree(text)?;
    let job = root
        .children
        .iter()
        .find(|c| c.name == "JOBFile")
        .ok_or_else(|| IoError::Invalid("JobXML: no JOBFile element".into()))?;
    let mut data = FieldData {
        name: name.to_string(),
        format: "Trimble JobXML".into(),
        ..Default::default()
    };
    let target_heights: Vec<(&str, f64)> = job
        .child("FieldBook")
        .into_iter()
        .flat_map(|f| f.children.iter())
        .filter(|r| r.name == "TargetRecord")
        .filter_map(|r| Some((r.attr("ID")?, r.num("TargetHeight").unwrap_or(0.0))))
        .collect();
    let mut station_index: Vec<(String, usize)> = Vec::new();
    if let Some(book) = job.child("FieldBook") {
        for r in &book.children {
            if r.deleted() {
                continue;
            }
            match r.name.as_str() {
                "StationRecord" => {
                    let Some(station) = r.text_of("StationName") else {
                        continue;
                    };
                    data.setups.push(Setup {
                        station: station.to_string(),
                        instrument_height: r.num("TheodoliteHeight").unwrap_or(0.0),
                        orientation: None,
                        observations: Vec::new(),
                    });
                    if let Some(id) = r.attr("ID") {
                        station_index.push((id.to_string(), data.setups.len() - 1));
                    }
                }
                "PointRecord" => {
                    let Some(point) = r.text_of("Name") else { continue };
                    if let Some(c) = r.child("Circle") {
                        let setup = r
                            .text_of("StationID")
                            .and_then(|id| station_index.iter().find(|(s, _)| s == id).map(|(_, k)| *k))
                            .or_else(|| data.setups.len().checked_sub(1));
                        let Some(k) = setup else { continue };
                        let hz = c.num("HorizontalCircle").map(Angle::from_degrees);
                        let v = c.num("VerticalCircle").map(Angle::from_degrees);
                        let (hz, zenith) = face_left(hz, v);
                        let distance = c.num("EDMDistance");
                        let target_height = r
                            .text_of("TargetID")
                            .and_then(|id| target_heights.iter().find(|(t, _)| *t == id))
                            .map_or(0.0, |(_, h)| *h);
                        data.setups[k].observations.push(RawObservation {
                            target: point.to_string(),
                            hz,
                            zenith: if distance.is_some() || v.is_some() {
                                zenith
                            } else {
                                None
                            },
                            distance,
                            target_height,
                            dz: None,
                        });
                    } else if let Some(p) = grid(r) {
                        data.points.retain(|(n, _)| n != point);
                        data.points.push((point.to_string(), p));
                    }
                }
                _ => {}
            }
        }
    }
    // Reduced coordinates win over keyed-in ones.
    if let Some(red) = job.child("Reductions") {
        for p in red.children.iter().filter(|p| p.name == "Point") {
            if let (Some(name), Some(g)) = (p.text_of("Name"), grid(p)) {
                data.points.retain(|(n, _)| n != name);
                data.points.push((name.to_string(), g));
            }
        }
    }
    data.setups.retain(|s| !s.observations.is_empty());
    if data.setups.is_empty() && data.points.is_empty() {
        return Err(IoError::Invalid(
            "JobXML: no stations, observations or points found".into(),
        ));
    }
    Ok(data)
}

/// Reads GSI or JobXML by content.
pub fn read_raw(name: &str, bytes: &[u8]) -> Result<FieldData, IoError> {
    let head = std::str::from_utf8(&bytes[..bytes.len().min(512)]).unwrap_or("");
    if head.trim_start().starts_with('<') {
        read_jobxml(name, bytes)
    } else {
        read_gsi(name, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gsi16_setup_and_pointings() {
        // Station 1 at known coordinates (mm), instrument height 1.550;
        // backsight 2 at 0 gon; foresight 3 face left and face right.
        let gsi = "\
*110001+0000000000000001 84..10+0000000500000000 85..10+0000007000000000 86..10+0000000000010000 88..10+0000000000001550
*110002+0000000000000002 21.322+0000000000000000 22.322+0000000010000000 31..00+0000000000100000 87..10+0000000000001500
*110003+0000000000000003 21.322+0000000010000000 22.322+0000000009000000 31..00+0000000000050000 87..10+0000000000001500
*110004+0000000000000003 21.322+0000000030000000 22.322+0000000031000000 31..00+0000000000050000 87..10+0000000000001500
";
        let d = read_raw("job.gsi", gsi.as_bytes()).expect("gsi");
        assert_eq!(d.setups.len(), 1);
        let s = &d.setups[0];
        assert_eq!(s.station, "1");
        assert!((s.instrument_height - 1.55).abs() < 1e-12);
        assert_eq!(s.observations.len(), 3);
        let o = &s.observations[1];
        assert_eq!(o.target, "3");
        assert!((o.hz.expect("hz").gons() - 100.0).abs() < 1e-9);
        assert!((o.zenith.expect("v").gons() - 90.0).abs() < 1e-9);
        assert!((o.distance.expect("d") - 50.0).abs() < 1e-12);
        // Face right reduced to face left.
        let fr = &s.observations[2];
        assert!((fr.hz.expect("hz").gons() - 100.0).abs() < 1e-9);
        assert!((fr.zenith.expect("v").gons() - 90.0).abs() < 1e-9);
        assert_eq!(
            d.points,
            vec![("1".to_string(), DVec3::new(500_000.0, 7_000_000.0, 10.0))]
        );
    }

    #[test]
    fn gsi8_degrees_and_coordinates() {
        let gsi = "\
110001+000000S1 88..10+00001500
110002+00000BS1 21.104+09030150 22.104+09000000 31..00+00025000
110003+000000P9 81..00+12345678 82..00-00001000 83..00+00010000
";
        let d = read_gsi("a", gsi.as_bytes()).expect("gsi");
        assert_eq!(d.setups[0].station, "S1");
        let o = &d.setups[0].observations[0];
        assert_eq!(o.target, "BS1");
        // 090°30'15.0"
        assert!((o.hz.expect("hz").degrees() - (90.0 + 30.0 / 60.0 + 15.0 / 3600.0)).abs() < 1e-9);
        assert!((o.distance.expect("d") - 25.0).abs() < 1e-12);
        assert_eq!(d.points[0].0, "P9");
        assert!((d.points[0].1 - DVec3::new(12345.678, -1.0, 10.0)).length() < 1e-9);
        assert!(read_gsi("x", b"hello world\n").is_err());
    }

    #[test]
    fn jobxml_records() {
        let xml = r#"<?xml version="1.0"?>
<JOBFile jobName="Test" version="5.6">
  <FieldBook>
    <PointRecord ID="p1"><Name>CP1</Name><Grid><North>7000000</North><East>500000</East><Elevation>10</Elevation></Grid></PointRecord>
    <TargetRecord ID="t1"><TargetHeight>1.8</TargetHeight></TargetRecord>
    <StationRecord ID="s1"><StationName>CP1</StationName><TheodoliteHeight>1.6</TheodoliteHeight></StationRecord>
    <PointRecord ID="p2"><Name>CP2</Name><StationID>s1</StationID><TargetID>t1</TargetID>
      <Circle><HorizontalCircle>0</HorizontalCircle><VerticalCircle>90</VerticalCircle><EDMDistance>120.5</EDMDistance></Circle></PointRecord>
    <PointRecord ID="p3"><Name>X</Name><Deleted>true</Deleted><StationID>s1</StationID>
      <Circle><HorizontalCircle>10</HorizontalCircle><VerticalCircle>90</VerticalCircle></Circle></PointRecord>
    <PointRecord ID="p4"><Name>T1</Name><StationID>s1</StationID><TargetID>t1</TargetID>
      <Circle><HorizontalCircle>225</HorizontalCircle><VerticalCircle>275</VerticalCircle><EDMDistance>40</EDMDistance></Circle></PointRecord>
  </FieldBook>
  <Reductions><Point><Name>CP2</Name><Grid><North>7000120.5</North><East>500000</East><Elevation>9.8</Elevation></Grid></Point></Reductions>
</JOBFile>"#;
        let d = read_raw("t.jxl", xml.as_bytes()).expect("jobxml");
        assert_eq!(d.format, "Trimble JobXML");
        assert_eq!(d.setups.len(), 1);
        let s = &d.setups[0];
        assert_eq!((s.station.as_str(), s.instrument_height), ("CP1", 1.6));
        assert_eq!(s.observations.len(), 2);
        assert!((s.observations[0].target_height - 1.8).abs() < 1e-12);
        let t1 = &s.observations[1];
        assert!((t1.hz.expect("hz").degrees() - 45.0).abs() < 1e-9);
        assert!((t1.zenith.expect("v").degrees() - 85.0).abs() < 1e-9);
        assert_eq!(d.points.len(), 2);
        assert!(
            d.points
                .iter()
                .any(|(n, p)| n == "CP2" && (p.z - 9.8).abs() < 1e-12)
        );
    }
}
