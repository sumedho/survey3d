use egui_cad::command_line::LogKind;
use glam::DVec3;
use survey_core::traverse::TraverseRecord;
use survey_model::{Entity, RecordKind, SheetRecord};

use super::*;
use crate::settings::AppSettings;

fn session() -> Session {
    Session::new(Project::new(&AppSettings::default()).expect("project"))
}

fn last_log(s: &Session) -> (LogKind, String) {
    let l = s.log.lines().last().expect("a log line");
    (l.kind, l.text.clone())
}

fn polylines(s: &Session) -> Vec<survey_model::Polyline> {
    s.project
        .doc
        .entities()
        .filter_map(|(_, e)| match e {
            Entity::Polyline(l) => Some(l.clone()),
            _ => None,
        })
        .collect()
}

/// A closed 10 m square drawn by typing, as on the command line.
fn draw_square(s: &mut Session) {
    for line in ["pl", "1000,2000", "1010,2000", "1010,2010", "1000,2010", "c"] {
        s.run(line);
    }
}

#[test]
fn typed_input_drives_tools_like_the_command_line() {
    let mut s = session();
    for line in ["point", "1000,2000,50", "1010,2000", ""] {
        s.run(line);
    }
    let names: Vec<String> = s
        .project
        .doc
        .entities()
        .filter_map(|(_, e)| match e {
            Entity::Point(p) => Some(p.name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        names.len(),
        2,
        "{:?}",
        s.log.lines().map(|l| &l.text).collect::<Vec<_>>()
    );

    draw_square(&mut s);
    let lines = polylines(&s);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].closed);
    assert_eq!(lines[0].vertices.len(), 4);
    // The tool stays active for the next line.
    assert_eq!(s.active_tool.id(), "polyline");
    s.run("select");
    assert_eq!(s.active_tool.id(), SELECT_TOOL_ID);

    // A point name works as point input.
    s.run("pl");
    s.run(&names[0]);
    s.run(&names[1]);
    s.run("");
    assert_eq!(polylines(&s).len(), 2);

    s.run("undo");
    assert_eq!(polylines(&s).len(), 1);
    s.run("redo");
    assert_eq!(polylines(&s).len(), 2);

    s.run("frobnicate");
    assert_eq!(last_log(&s).0, LogKind::Error);
}

#[test]
fn selection_commands_and_records() {
    let mut s = session();
    draw_square(&mut s);
    let (id, _) = s.project.doc.entities().next().expect("square");
    s.project.selection.set([id]);
    s.run("mkparcel");
    assert_eq!(s.project.doc.parcels().len(), 1);
    assert_eq!(s.project.doc.parcels()[0].boundary, id);
    assert!(s.take_events().contains(&Event::Open(Panel::Parcels)));
    // Already a parcel: nothing more.
    s.run("mkparcel");
    assert_eq!(s.project.doc.parcels().len(), 1);

    s.run("list");
    let events = s.take_events();
    assert!(
        matches!(&events[..], [Event::Report { title, .. }] if title.starts_with("Listing")),
        "{events:?}"
    );

    s.run("mkalign");
    assert_eq!(s.project.doc.alignments().len(), 1);
    let a = s.project.doc.alignments()[0].clone();
    assert_eq!(a.name, "MC01");
}

#[test]
fn records_are_edited_by_id() {
    let mut s = session();
    let sheet = |name: &str| SheetRecord {
        name: name.into(),
        ..Default::default()
    };
    let a = s.add_record(sheet("A")).expect("id");
    let b = s.add_record(sheet("B")).expect("id");
    assert_ne!(a, b);
    let audit_before = s.project.history.pending_audit().len();
    // Unchanged: no undo step.
    s.update_record(b, sheet("B"));
    assert_eq!(s.project.history.pending_audit().len(), audit_before);

    s.update_record(b, sheet("B2"));
    s.delete_record::<SheetRecord>(a);
    let names = |s: &Session| {
        s.project
            .doc
            .sheets()
            .iter()
            .map(|r| r.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&s), ["B2"]);
    s.undo();
    assert_eq!(names(&s), ["A", "B2"]);
    s.undo();
    assert_eq!(names(&s), ["A", "B"]);
    // Gone records are ignored, not an error.
    s.delete_record::<SheetRecord>(survey_model::RecordId(999));
    assert!(s.project.doc.dirty().records.contains(&RecordKind::Sheet));

    let traverse = |legs: usize| TraverseRecord {
        name: "Loop".into(),
        start: "1".into(),
        legs: vec![
            survey_core::traverse::Leg::new(
                "2",
                survey_core::traverse::Direction::Angle(survey_core::Angle::from_degrees(90.0)),
                25.0,
            );
            legs
        ],
        ..Default::default()
    };
    s.save_traverse(traverse(1));
    s.save_traverse(traverse(2));
    assert_eq!(s.project.doc.traverses().len(), 1, "saved by name");
    assert_eq!(s.project.doc.traverses()[0].legs.len(), 2);
    s.delete_traverse("Loop");
    assert!(s.project.doc.traverses().is_empty());
}

#[test]
fn raw_data_imports_replace_by_name() {
    let mut s = session();
    let gsi =
        b"110001+0000000000000001 81..00+0000000000100000 82..00+0000000000200000 83..00+0000000000001000\n";
    s.import_raw("job.gsi", gsi);
    assert_eq!(s.project.doc.field_data().len(), 1, "{:?}", last_log(&s));
    let id = s.project.doc.field_data()[0].id;
    let events = s.take_events();
    assert!(events.contains(&Event::Open(Panel::RawData { name: "job".into() })));
    s.import_raw("job.gsi", gsi);
    assert_eq!(s.project.doc.field_data().len(), 1);
    assert_eq!(s.project.doc.field_data()[0].id, id, "the same record, replaced");
}

#[test]
fn unnamed_points_number_on_from_the_batch_and_project() {
    let mut s = session();
    let p = |name: &str, e: f64| survey_io::ImportedPoint {
        name: name.into(),
        easting: e,
        northing: 0.0,
        height: None,
        code: String::new(),
        description: String::new(),
    };
    s.add_points(&[p("7", 0.0)], None, "test");
    s.add_points(&[p("", 1.0), p("20", 2.0), p("", 3.0)], Some("New"), "test");
    let mut names: Vec<String> = s
        .project
        .doc
        .entities()
        .filter_map(|(_, e)| match e {
            Entity::Point(p) => Some(p.name.clone()),
            _ => None,
        })
        .collect();
    names.sort_by_key(|n| n.parse::<u32>().unwrap_or(0));
    assert_eq!(names, ["7", "20", "21", "22"]);
    assert!(s.project.doc.layer_by_name("new").is_some());
    assert_eq!(
        s.project.history.undo_label().as_deref(),
        Some("Import 3 points from test")
    );
}

#[test]
fn commands_end_the_active_tool_and_help_works() {
    let mut s = session();
    for line in ["point", "0,0", "10,0", "", "inverse", "1", "2", ""] {
        s.run(line);
    }
    assert_eq!(s.active_tool.id(), SELECT_TOOL_ID, "Enter finishes the inverse");
    s.run("inverse");
    s.run("1");
    s.run("delete");
    assert_eq!(s.active_tool.id(), SELECT_TOOL_ID, "a command ends the tool");
    assert_eq!(
        last_log(&s),
        (LogKind::Warning, "Select objects to delete first".into())
    );
    s.run("help");
    let text: Vec<String> = s.log.lines().map(|l| l.text.clone()).collect();
    assert!(
        text.iter().any(|l| l.trim_start().starts_with("polyline, pl")),
        "{text:?}"
    );
    assert!(text.iter().any(|l| l.contains("mkparcel")));
    assert!(text.iter().any(|l| l.starts_with("Point input")));
}

#[test]
fn text_tool_places_notes() {
    let mut s = session();
    for line in ["text", "100,200", "SMITH STREET", "", "tx", "110,200", "pl"] {
        s.run(line);
    }
    let texts: Vec<(String, f64)> = s
        .project
        .doc
        .entities()
        .filter_map(|(_, e)| match e {
            Entity::Text(t) => Some((t.text.clone(), t.position.x)),
            _ => None,
        })
        .collect();
    // Typed after placing, even a command name is text.
    assert_eq!(
        texts,
        [("SMITH STREET".to_string(), 100.0), ("pl".to_string(), 110.0)]
    );
}

#[test]
fn imported_points_go_to_their_codes_layer() {
    let mut s = session();
    let mut library = survey_core::field_to_finish::FeatureLibrary::default();
    library.codes.push(survey_core::field_to_finish::FeatureCode::new(
        "SSM",
        "State survey mark",
        "CONTROL",
        [200, 0, 0],
        false,
    ));
    s.apply_feature_codes(library);
    let p = |name: &str, code: &str| survey_io::ImportedPoint {
        name: name.into(),
        easting: 0.0,
        northing: 0.0,
        height: None,
        code: code.into(),
        description: String::new(),
    };
    s.add_points(&[p("1", "SSM"), p("2", "XYZ"), p("3", "")], None, "test");
    s.add_points(&[p("4", "SSM")], Some("Mine"), "test");
    let doc = &s.project.doc;
    let layer_of = |name: &str| {
        let (_, point) = doc.find_point_by_name(name).expect("point");
        doc.layer(point.layer).map(|l| l.name.clone()).unwrap_or_default()
    };
    assert_eq!(layer_of("1"), "CONTROL", "the code's layer");
    assert_eq!(layer_of("2"), "0", "unknown code: the current layer");
    assert_eq!(layer_of("3"), "0");
    assert_eq!(layer_of("4"), "Mine", "a layer asked for wins");
    let control = doc.layer_by_name("CONTROL").expect("created");
    assert_eq!(
        control.color,
        survey_model::Color::rgb(200, 0, 0),
        "in the code's colour"
    );
}

fn point_positions(s: &Session) -> Vec<(String, glam::DVec2)> {
    let mut v: Vec<(String, glam::DVec2)> = s
        .project
        .doc
        .entities()
        .filter_map(|(_, e)| match e {
            Entity::Point(p) => Some((p.name.clone(), p.position.truncate())),
            _ => None,
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn has_point_at(s: &Session, e: f64, n: f64) -> bool {
    point_positions(s)
        .iter()
        .any(|(_, p)| (p.x - e).abs() < 1e-6 && (p.y - n).abs() < 1e-6)
}

/// A lot keyed as bearings and distances: each leg moves on, `c` closes on
/// the start (its end point is the start corner) and reports the misclosure.
#[test]
fn bearing_distance_closes_a_lot() {
    let mut s = session();
    for line in ["bd", "1000,2000", "0 10", "90 10", "180 10", "270 10.01", "c"] {
        s.run(line);
    }
    let points = point_positions(&s);
    assert_eq!(points.len(), 4, "start and three corners: {points:?}");
    assert!(
        has_point_at(&s, 1000.0, 2010.0)
            && has_point_at(&s, 1010.0, 2010.0)
            && has_point_at(&s, 1010.0, 2000.0)
    );
    let lines = polylines(&s);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].closed && lines[0].vertices.len() == 4);
    let report = s
        .log
        .lines()
        .map(|l| l.text.clone())
        .find(|t| t.contains("misclose"))
        .expect("a closure report");
    assert!(
        report.contains("0.010") && report.contains("1:4001") && report.contains("100"),
        "{report}"
    );
}

#[test]
fn bearing_distance_undo_sideshots_and_open_runs() {
    let mut s = session();
    for line in ["bd", "0,0", "0 10", "u"] {
        s.run(line);
    }
    assert_eq!(point_positions(&s).len(), 1, "undo removed the leg's point");
    // A sideshot does not move on; the next leg starts from the same point.
    for line in ["s", "90 5", "s", "90 10", "0 10", ""] {
        s.run(line);
    }
    assert!(has_point_at(&s, 5.0, 0.0) && has_point_at(&s, 10.0, 0.0) && has_point_at(&s, 10.0, 10.0));
    let lines = polylines(&s);
    assert_eq!(lines.len(), 1);
    assert!(
        !lines[0].closed && lines[0].vertices.len() == 3,
        "the sideshot is not in the line"
    );
}

/// Radiate in traverse mode occupies each new point, backsighting the last.
#[test]
fn radiate_traverse_moves_the_setup() {
    let mut s = session();
    for line in ["rad", "0,0", "0", "t", "90 10", "90 10"] {
        s.run(line);
    }
    // North backsight, 90° right: east to (10, 0). Occupying it, backsight west,
    // 90° right: north to (10, 10).
    assert!(has_point_at(&s, 10.0, 0.0), "{:?}", point_positions(&s));
    assert!(has_point_at(&s, 10.0, 10.0), "{:?}", point_positions(&s));
}

/// Each command name starts one thing: a tool, or a session command.
#[test]
fn command_names_are_unique() {
    let s = session();
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let session_names = SESSION_COMMANDS
        .iter()
        .map(|(n, _)| (*n, "session command"))
        .chain(COMMAND_ALIASES.iter().map(|(a, _)| (*a, "session alias")));
    for t in s.tools.tools() {
        for c in t.commands {
            if let Some(other) = seen.insert(c.to_string(), t.id.to_string()) {
                panic!("'{c}' starts both {other} and {}", t.id);
            }
        }
    }
    for (name, what) in session_names {
        if let Some(tool) = seen.get(name) {
            panic!("'{name}' is both the tool {tool} and a {what}");
        }
    }
}

fn log_text(s: &Session) -> Vec<String> {
    s.log.lines().map(|l| l.text.clone()).collect()
}

/// A lot with a curved corner, keyed as a plan shows it: a tangent curve of
/// radius 10 turning right, closed on its start and named as a parcel.
#[test]
fn bearing_distance_curved_lot_becomes_a_parcel() {
    let mut s = session();
    for line in [
        "bd",
        "0,0",
        "0 20",
        "90 10",
        "r=10 d=90 r",
        "180 10",
        "270 20",
        "c",
        "Lot 7",
    ] {
        s.run(line);
    }
    let log = log_text(&s);
    assert!(
        has_point_at(&s, 20.0, 10.0),
        "the curve ends at (20, 10): {log:?}"
    );
    let lines = polylines(&s);
    assert_eq!(lines.len(), 1);
    let lot = &lines[0];
    assert!(lot.closed && lot.vertices.len() == 5);
    assert!(lot.bulge(2) < 0.0, "the curve turns right: {:?}", lot.bulges);
    let (_, _, area) = survey_core::arc::polyline_metrics(&lot.vertices, &lot.bulges, true);
    let expected = 300.0 + 25.0 * std::f64::consts::PI;
    assert!((area.abs() - expected).abs() < 1e-6, "{area}");
    let report = log.iter().find(|t| t.contains("misclose")).expect("closure");
    assert!(report.contains("exact") && report.contains("378.54"), "{report}");
    let parcels = s.project.doc.parcels();
    assert_eq!(parcels.len(), 1);
    assert_eq!(parcels[0].name, "Lot 7");
    assert_eq!(
        s.project.doc.entity(parcels[0].boundary),
        Some(&Entity::Polyline(lot.clone()))
    );
}

/// `j` computes the closing leg; the lot prompt then takes `y` for the next
/// lot number, Enter to skip, or a command to move on.
#[test]
fn bearing_distance_joins_and_names_lots() {
    let mut s = session();
    for line in ["bd", "0,0", "0 10", "90 10", "j"] {
        s.run(line);
    }
    let log = log_text(&s);
    let closing = log
        .iter()
        .find(|t| t.contains("Closing leg"))
        .expect("closing leg");
    assert!(closing.contains("225") && closing.contains("14.142"), "{closing}");
    assert!(s.prompt().contains("y for Lot 1"), "{}", s.prompt());
    s.run("y");
    assert_eq!(s.project.doc.parcels()[0].name, "Lot 1");
    // The next run starts from typed coordinates; Enter skips the parcel.
    for line in ["100,0", "0 10", "90 10", "180 10", "j", ""] {
        s.run(line);
    }
    assert_eq!(s.project.doc.parcels().len(), 1, "skipped");
    assert_eq!(polylines(&s).iter().filter(|l| l.closed).count(), 2);
    // A command after closing switches tools instead of naming a lot.
    for line in ["200,0", "0 10", "90 10", "j", "pl"] {
        s.run(line);
    }
    assert_eq!(s.project.doc.parcels().len(), 1);
    assert_eq!(s.active_tool.id(), "polyline", "{}", s.prompt());
}

#[test]
fn reduce_command_takes_the_file_name() {
    let mut s = session();
    let rw5 = b"MO,AD0,UN1,SF1.0,EC0,EO0.0,AU0
OC,OP1,N 100.0,E 100.0,EL10.0
LS,HI1.5,HR1.5
SS,OP1,FP5,AZ90.0000,ZE90.0000,SD10.0,--TR
";
    s.import_raw("a.rw5", rw5);
    s.import_raw("b.rw5", rw5);
    s.run("reduce");
    let (kind, text) = last_log(&s);
    assert!(
        kind == LogKind::Error && text.contains("Name the raw data file"),
        "{text}"
    );
    s.run("reduce b");
    let (_, p) = s.project.doc.find_point_by_name("5").expect("reduced");
    assert!((p.position - DVec3::new(110.0, 100.0, 10.0)).length() < 1e-9);
    assert_eq!(p.code, "TR");
}

#[test]
fn instrument_coordinates_are_recomputed_and_reimport_keeps_corrections() {
    let mut s = session();
    // Station S1 (100, 200), keyed backsight BS1 due north, and a shot P1 due
    // east carrying the instrument's coordinates, 3 mm out.
    let gsi = b"110001+000000S1 84..10+00100000 85..10+00200000 88..10+00001500
110002+00000BS1 81..00+00100000 82..00+00300000
110003+00000BS1 21.104+00000000 22.104+09000000 31..00+00100000
110004+000000P1 21.104+09000000 22.104+09000000 31..00+00010000 81..00+00110003 82..00+00200000 71....+000000KB
";
    s.import_raw("job.gsi", gsi);
    let p1 = |s: &Session| s.project.doc.find_point_by_name("P1").expect("P1").1.clone();
    assert!((p1(&s).position.x - 110.003).abs() < 1e-9, "shown on import");
    assert_eq!(s.project.doc.field_data()[0].reduced, ["P1"]);

    let summary = s.reduce_raw("job").expect("reduce");
    assert!(
        summary.contains("1 updated") && summary.contains("1 check"),
        "{summary}"
    );
    assert!(
        (p1(&s).position.x - 110.0).abs() < 1e-9,
        "recomputed from the shot"
    );
    assert_eq!(p1(&s).code, "KB");

    s.edit_raw(
        "job",
        survey_core::network::RawEdit {
            setup: 0,
            observation: Some(1),
            code: Some("FH".into()),
            ..Default::default()
        },
        false,
    )
    .expect("edit");
    assert_eq!(p1(&s).code, "FH");
    // The crew sends the file again: the correction and the link to P1 stay.
    s.import_raw("job.gsi", gsi);
    let f = &s.project.doc.field_data()[0];
    assert_eq!(f.edits.len(), 1);
    assert_eq!(f.reduced, ["P1"]);
    s.reduce_raw("job").expect("reduce again");
    assert_eq!(p1(&s).code, "FH");
    assert!((p1(&s).position.x - 110.0).abs() < 1e-9);
}

#[test]
fn bearing_distance_turned_angles_and_deflections() {
    let mut s = session();
    for line in [
        "bd",
        "0,0",
        "a90.0000 10",
        "0 10",
        "a90.0000 10",
        "dr90.0000 10",
        "dl 90.0000 10",
        "",
    ] {
        s.run(line);
    }
    let log = log_text(&s);
    assert!(
        log.iter()
            .any(|t| t.contains("An angle turns from the line before")),
        "the first leg needs a backsight: {log:?}"
    );
    // North 10, then 90° right of the back line (south) is west, a right
    // deflection off west is north, and a left one off north is west.
    for (e, n) in [(0.0, 10.0), (-10.0, 10.0), (-10.0, 20.0), (-20.0, 20.0)] {
        assert!(has_point_at(&s, e, n), "({e}, {n}): {log:?}");
    }
    // A backsight lets the first leg be an angle.
    for line in ["bd", "100,0", "bs 0", "a90.0000 10", ""] {
        s.run(line);
    }
    assert!(has_point_at(&s, 110.0, 0.0), "{:?}", log_text(&s));
}

#[test]
fn renumber_raise_and_scale() {
    let mut s = session();
    for line in ["point", "0,0,10", "10,0,10", "10,10", ""] {
        s.run(line);
    }
    let ids: Vec<EntityId> = s.project.doc.entities().map(|(id, _)| id).collect();
    s.project.selection.set(ids.clone());
    s.run("renumber 101");
    let name = |s: &Session, id: EntityId| match s.project.doc.entity(id) {
        Some(Entity::Point(p)) => p.name.clone(),
        _ => String::new(),
    };
    assert_eq!(
        ids.iter().map(|id| name(&s, *id)).collect::<Vec<_>>(),
        ["101", "102", "103"],
        "{:?}",
        log_text(&s)
    );
    s.run("renumber 101-102 +1000");
    assert_eq!(name(&s, ids[0]), "1101");
    assert_eq!(name(&s, ids[2]), "103", "outside the range");
    // A taken number is refused.
    s.project.selection.set([ids[2]]);
    s.run("renumber 1102");
    assert!(last_log(&s).1.contains("in use"), "{:?}", last_log(&s));
    assert_eq!(name(&s, ids[2]), "103");

    // Heights: the two with heights go up; the plan-only one is left.
    s.project.selection.set(ids.clone());
    s.run("raise 0.25");
    assert!(last_log(&s).1.contains("Raised 2 objects"), "{:?}", last_log(&s));
    let z = |s: &Session, id: EntityId| match s.project.doc.entity(id) {
        Some(Entity::Point(p)) => p.position,
        _ => DVec3::ZERO,
    };
    assert!((z(&s, ids[0]).z - 10.25).abs() < 1e-9);

    // Scale about the first point by measured and true lengths: 10 → 10.01.
    for line in ["scale", "0,0", "10 10.01"] {
        s.run(line);
    }
    assert!(
        (z(&s, ids[1]) - DVec3::new(10.01, 0.0, 10.25)).length() < 1e-9,
        "{:?}",
        log_text(&s)
    );
    s.undo();
    assert!((z(&s, ids[1]).x - 10.0).abs() < 1e-9);
}

#[test]
fn apparent_intersection_trim_and_extend() {
    let mut s = session();
    for line in ["point", "0,0", "5,0", "10,2", "10,8", ""] {
        s.run(line);
    }
    // The corner the two faces would meet at, typed as point pairs.
    for line in ["ix", "1 2", "3 4", ""] {
        s.run(line);
    }
    assert!(has_point_at(&s, 10.0, 0.0), "{:?}", log_text(&s));

    // A kerb crossing two lines: trim its middle out (all lines are boundaries).
    for line in ["pl", "0,20", "30,20", ""] {
        s.run(line);
    }
    for line in ["pl", "10,15", "10,25", ""] {
        s.run(line);
    }
    for line in ["pl", "20,15", "20,25", ""] {
        s.run(line);
    }
    let kerb = s
        .project
        .doc
        .entities()
        .find(|(_, e)| matches!(e, Entity::Polyline(l) if l.vertices[0].y == 20.0))
        .map(|(id, _)| id)
        .expect("kerb");
    s.project.selection.clear();
    s.run("trim");
    s.run("");
    s.pointer.hovered = Some(kerb);
    s.run("15,20");
    s.run("");
    assert_eq!(polylines(&s).len(), 4, "{:?}", log_text(&s));
    // Extend the first piece's end (now at x = 10, on a boundary) out to the
    // line at x = 20, chosen as the only boundary.
    let far = s
        .project
        .doc
        .entities()
        .find(|(_, e)| matches!(e, Entity::Polyline(l) if l.vertices[0].x == 20.0))
        .map(|(id, _)| id)
        .expect("far");
    s.project.selection.set([far]);
    s.run("extend");
    s.pointer.hovered = Some(kerb);
    s.run("9,20");
    s.run("");
    let Some(Entity::Polyline(l)) = s.project.doc.entity(kerb) else {
        panic!()
    };
    assert_eq!(l.vertices.last().map(|v| v.x), Some(20.0), "{:?}", log_text(&s));
}
