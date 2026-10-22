use egui_cad::command_line::LogKind;
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
    assert!(events.contains(&Event::Open(Panel::Network {
        source: "f:job".into()
    })));
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
