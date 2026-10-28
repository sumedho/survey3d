use super::*;

/// A fresh directory for files a test writes.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("survey3d-mcp-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Calls a tool; returns its text and whether it failed.
fn call(server: &mut Server, name: &str, args: Value) -> (String, bool) {
    let result = server.call(name, &args).to_json();
    let text = result["content"]
        .as_array()
        .expect("content")
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (text, result["isError"].as_bool().expect("isError"))
}

fn ok(server: &mut Server, name: &str, args: Value) -> String {
    let (text, error) = call(server, name, args.clone());
    assert!(!error, "{name} {args} failed: {text}");
    text
}

fn ok_json(server: &mut Server, name: &str, args: Value) -> Value {
    serde_json::from_str(&ok(server, name, args)).expect("json result")
}

#[test]
fn points_queries_and_selection() {
    let mut s = Server::new_for_tests();
    let added = ok_json(
        &mut s,
        "add_points",
        json!({ "points": [
            { "name": "100", "e": 1000.0, "n": 2000.0, "z": 50.0, "code": "CP" },
            { "e": 1010.0, "n": 2000.0, "code": "fence" },
            { "e": 1010.0, "n": 2010.0 }
        ], "layer": "Control" }),
    );
    let rows = added["added"].as_array().expect("added");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1]["name"], "101", "unnamed points continue the numbering");
    assert_eq!(rows[1]["z"], Value::Null, "no height given");
    assert_eq!(rows[0]["layer"], "Control");

    let q = ok_json(
        &mut s,
        "query",
        json!({ "kind": "points", "filter": { "code": "cp" } }),
    );
    assert_eq!(q["total"], 1);
    assert_eq!(q["rows"][0]["e"], 1000.0);
    let q = ok_json(
        &mut s,
        "query",
        json!({ "kind": "points", "filter": { "within": [1005, 1995, 1015, 2005] } }),
    );
    assert_eq!(q["total"], 1);
    let q = ok_json(
        &mut s,
        "query",
        json!({ "kind": "points", "limit": 2, "offset": 2 }),
    );
    assert_eq!((q["total"].as_u64(), q["returned"].as_u64()), (Some(3), Some(1)));
    assert!(
        call(
            &mut s,
            "query",
            json!({ "kind": "points", "filter": { "layer": "nope" } })
        )
        .1
    );

    let sel = ok_json(
        &mut s,
        "select",
        json!({ "kind": "points", "filter": { "names": ["100", "101"] } }),
    );
    assert_eq!(sel["selected"], 2);
    let out = ok(&mut s, "run_commands", json!({ "lines": ["average"] }));
    assert!(out.contains("Average of 2 points"), "{out}");
    let sel = ok_json(&mut s, "select", json!({ "mode": "clear" }));
    assert_eq!(sel["selected"], 0);

    let info = ok_json(&mut s, "project_info", json!({}));
    assert_eq!(info["counts"]["points"], 4);
    assert_eq!(info["undo"], "Average point 103");
}

#[test]
fn command_line_drawing_and_records() {
    let mut s = Server::new_for_tests();
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["pl", "1000,2000", "1040,2000", "@30<0", "1000,2030", "c"] }),
    );
    assert!(out.starts_with("> "), "{out}");
    assert!(out.contains("Prompt: Polyline"), "the tool stays active: {out}");
    let lines = ok_json(&mut s, "query", json!({ "kind": "polylines" }));
    assert_eq!(lines["total"], 1);
    let square = &lines["rows"][0];
    assert_eq!(square["closed"], true);
    assert!(
        (square["area"].as_f64().expect("area") - 1200.0).abs() < 1e-6,
        "{square}"
    );

    // Parcels from the selection.
    ok(&mut s, "run_commands", json!({ "lines": ["select"] }));
    ok(&mut s, "select", json!({ "kind": "polylines" }));
    let out = ok(&mut s, "run_commands", json!({ "lines": ["mkparcel"] }));
    assert!(out.contains("Created 1 parcel"), "{out}");
    let parcels = ok_json(&mut s, "query", json!({ "kind": "parcel" }));
    let parcel_id = parcels["rows"][0]["id"].as_i64().expect("id");

    // A sheet: create, edit a nested field, delete.
    let sheet = ok_json(
        &mut s,
        "set_record",
        json!({ "kind": "sheet", "record": { "name": "Plan", "centre": [1020.0, 2015.0], "scale": 200.0 } }),
    );
    let sheet_id = sheet["id"].as_i64().expect("id");
    assert_eq!(sheet["paper"], "a3", "defaults fill the rest");
    let sheet = ok_json(
        &mut s,
        "set_record",
        json!({ "kind": "sheet", "id": sheet_id, "record": { "title": { "title": "Lot 1" } } }),
    );
    assert_eq!(sheet["title"]["title"], "Lot 1");
    assert_eq!(sheet["scale"], 200.0, "other fields unchanged");
    assert!(
        call(
            &mut s,
            "set_record",
            json!({ "kind": "sheet", "id": 999, "record": {} })
        )
        .1
    );
    assert!(
        call(
            &mut s,
            "set_record",
            json!({ "kind": "sheet", "record": { "scale": "big" } })
        )
        .1
    );

    let rendered = s
        .call("render", &json!({ "sheet_id": sheet_id, "width": 400 }))
        .to_json();
    assert_eq!(rendered["isError"], false, "{rendered}");
    let image = &rendered["content"][1];
    assert_eq!(image["mimeType"], "image/png");
    assert!(
        image["data"].as_str().expect("data").starts_with("iVBORw0KGgo"),
        "PNG signature"
    );

    ok(
        &mut s,
        "delete_record",
        json!({ "kind": "parcel", "id": parcel_id }),
    );
    assert_eq!(ok_json(&mut s, "query", json!({ "kind": "parcel" }))["total"], 0);
    let out = ok(&mut s, "run_commands", json!({ "lines": ["undo"] }));
    assert!(out.contains("Undo: Remove parcel"), "{out}");
    assert_eq!(ok_json(&mut s, "query", json!({ "kind": "parcel" }))["total"], 1);

    let audit = ok_json(&mut s, "query", json!({ "kind": "audit", "limit": 3 }));
    assert_eq!(audit["rows"][0]["action"], "undo");
    assert!(audit["rows"][0]["user"].as_str().expect("user").contains("MCP"));
}

#[test]
fn files_save_open_and_guards() {
    let dir = temp_dir("files");
    let mut s = Server::new_for_tests();
    let csv = dir.join("pts.csv");
    std::fs::write(&csv, "P,E,N,Z,C\n1,500.0,600.0,10.0,CP\n2,510.0,600.0,11.0,CP\n").expect("csv");
    let out = ok(&mut s, "import_file", json!({ "path": csv }));
    assert!(out.contains("Imported 2 points"), "{out}");

    // Exports, and no silent overwrite.
    for (format, file) in [
        ("csv", "out.csv"),
        ("pdf", "plan.pdf"),
        ("dxf", "drawing.dxf"),
        ("gpkg", "out.gpkg"),
    ] {
        let path = dir.join(file);
        ok(&mut s, "export_file", json!({ "format": format, "path": path }));
        assert!(std::fs::metadata(&path).expect("written").len() > 0, "{format}");
    }
    let (text, error) = call(
        &mut s,
        "export_file",
        json!({ "format": "csv", "path": dir.join("out.csv") }),
    );
    assert!(error && text.contains("overwrite"), "{text}");
    let exported = std::fs::read_to_string(dir.join("out.csv")).expect("csv");
    assert!(
        exported.starts_with("1,500.0000,600.0000,10.0000,CP"),
        "{exported}"
    );
    let (text, error) = call(
        &mut s,
        "export_file",
        json!({ "format": "landxml", "path": dir.join("s.xml") }),
    );
    assert!(error && text.contains("no surfaces"), "{text}");

    // Unsaved work is not dropped by accident.
    let (text, error) = call(&mut s, "new_project", json!({}));
    assert!(error && text.contains("unsaved"), "{text}");
    let (text, error) = call(&mut s, "save_project", json!({}));
    assert!(error && text.contains("no file yet"), "{text}");

    let file = dir.join("job.s3d");
    ok(&mut s, "save_project", json!({ "path": file }));
    let info = ok_json(&mut s, "project_info", json!({}));
    assert_eq!(info["unsaved_changes"], false);
    ok(
        &mut s,
        "add_points",
        json!({ "points": [{ "e": 1.0, "n": 2.0 }] }),
    );
    ok(&mut s, "save_project", json!({}));

    let info = ok_json(
        &mut s,
        "new_project",
        json!({ "name": "Second", "conventions": "north american" }),
    );
    assert_eq!(info["name"], "Second");
    assert_eq!(info["counts"]["points"], 0);
    let info = ok_json(
        &mut s,
        "open_project",
        json!({ "path": file, "discard_changes": true }),
    );
    assert_eq!(info["counts"]["points"], 3);
    assert_eq!(info["records"]["sheet"], json!([]));
    // Reopening the file this server holds reverts to it as saved.
    ok(
        &mut s,
        "add_points",
        json!({ "points": [{ "e": 3.0, "n": 4.0 }] }),
    );
    let info = ok_json(
        &mut s,
        "open_project",
        json!({ "path": file, "discard_changes": true }),
    );
    assert_eq!(info["counts"]["points"], 3);

    // A sheet given no centre or scale frames the drawing.
    let sheet = ok_json(
        &mut s,
        "set_record",
        json!({ "kind": "sheet", "record": { "title": { "title": "Job" } } }),
    );
    let centre = &sheet["centre"];
    assert!((centre[0].as_f64().expect("e") - 255.5).abs() < 1.0, "{sheet}");
    assert_eq!(sheet["title"]["title"], "Job");
    assert_eq!(sheet["name"], "Sheet 1");

    let mut read_only = Server::new(Options {
        project: Some(file.clone()),
        read_only: true,
        user: "t".into(),
    })
    .expect("server");
    let (text, error) = call(&mut read_only, "save_project", json!({}));
    assert!(error && text.contains("read-only"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The gaps reported from a real job, each checked end to end.
#[test]
fn reported_gaps_are_closed() {
    let dir = temp_dir("gaps");
    let mut s = Server::new_for_tests();

    // 3. Coordinate system from its name.
    let project = ok_json(
        &mut s,
        "set_project",
        json!({ "name": "Lot 12", "coordinate_system": { "name": "MGA2020 Zone 50" } }),
    );
    assert_eq!(project["coordinate_system"], "MGA2020 Zone 50 (EPSG:7850)");
    assert_eq!(project["coordinate_system_detail"]["Projected"]["zone"], 50);

    // 1. Feature codes: symbols, layers, linework.
    ok(
        &mut s,
        "set_feature_codes",
        json!({ "codes": [
            { "code": "SSM", "description": "State survey mark", "layer": "CONTROL", "style": { "symbol": "triangle_dot" } },
            { "code": "BM", "description": "Benchmark", "layer": "CONTROL", "style": { "symbol": "benchmark" } },
            { "code": "PEG", "description": "Peg", "layer": "BOUNDARY MARKS", "style": { "symbol": "circle_dot" } },
            { "code": "KB", "description": "Kerb", "layer": "KERBS", "color": "#00aaff", "linework": true },
            { "code": "FNC", "description": "Fence", "layer": "FENCES", "linework": true, "style": { "line_style": "dash_dot" } }
        ] }),
    );
    let codes = ok_json(&mut s, "query", json!({ "kind": "feature_codes" }));
    assert_eq!(codes["total"], 5);
    assert_eq!(codes["rows"][3]["color"], json!([0, 170, 255]));
    assert!(
        call(
            &mut s,
            "set_feature_codes",
            json!({ "codes": [{ "code": "X", "style": { "symbol": "nope" } }] })
        )
        .1
    );
    ok(
        &mut s,
        "add_points",
        json!({ "points": [
            { "e": 0.0, "n": 0.0, "z": 10.0, "code": "KB1 ST" },
            { "e": 10.0, "n": 0.0, "z": 10.1, "code": "KB1" },
            { "e": 20.0, "n": 1.0, "z": 10.2, "code": "KB1 END" },
            { "e": 5.0, "n": 5.0, "z": 11.0, "code": "SSM" }
        ] }),
    );
    let out = ok(&mut s, "run_commands", json!({ "lines": ["linework"] }));
    assert!(!out.contains("empty"), "{out}");
    let kerb = ok_json(
        &mut s,
        "query",
        json!({ "kind": "polylines", "filter": { "code": "KB" } }),
    );
    assert_eq!(kerb["total"], 1, "{out}");
    assert_eq!(kerb["rows"][0]["vertices"].as_array().map(Vec::len), Some(3));

    // 2. Layers: distinct colours for new layers, styles, current layer.
    let a = ok_json(&mut s, "set_layer", json!({ "layer": "ROADS" }));
    let b = ok_json(
        &mut s,
        "set_layer",
        json!({ "layer": "EASEMENTS", "line_style": "dashed", "line_weight": 1.0, "current": true }),
    );
    assert_ne!(a["color"], b["color"]);
    assert_eq!(
        (b["line_style"].as_str(), b["current"].as_bool()),
        (Some("dashed"), Some(true))
    );
    let b = ok_json(
        &mut s,
        "set_layer",
        json!({ "layer": "EASEMENTS", "color": "#ff00ff" }),
    );
    assert_eq!(b["color"], "#ff00ff");
    assert!(
        call(
            &mut s,
            "set_layer",
            json!({ "layer": "ROADS", "rename": "easements" })
        )
        .1
    );

    // 4. Text, on the current layer.
    let text = ok_json(
        &mut s,
        "add_text",
        json!({ "texts": [
            { "text": "Easement A", "at": [10.0, 3.0], "rotation": 5.0 },
            { "text": "SMITH STREET", "at": [10.0, -4.0], "height_mm": 3.5, "align": "left" }
        ] }),
    );
    assert_eq!(text["added"][0]["layer"], "EASEMENTS");
    assert_eq!(text["added"][1]["align"], "left");
    let plan = s.call("render", &json!({ "width": 600 })).to_json();
    assert_eq!(plan["isError"], false, "{plan}");
    ok(
        &mut s,
        "export_file",
        json!({ "format": "dxf", "path": dir.join("t.dxf") }),
    );
    let dxf = std::fs::read_to_string(dir.join("t.dxf")).expect("dxf");
    assert!(dxf.contains("Easement A") && dxf.contains("SMITH STREET"));
    let svg_path = dir.join("t.svg");
    ok(
        &mut s,
        "export_file",
        json!({ "format": "svg", "path": svg_path }),
    );
    let svg = std::fs::read_to_string(&svg_path).expect("svg");
    assert!(svg.contains(">Easement A<") && svg.contains("EPSG:7850"));

    // 5. Edit lines (and other objects) in place.
    let id = kerb["rows"][0]["id"].as_i64().expect("id");
    let edited = ok_json(
        &mut s,
        "edit_entities",
        json!({ "filter": { "ids": [id] }, "set": { "name": "Kerb north", "layer": "ROADS" } }),
    );
    assert_eq!(edited["changed"], 1);
    assert_eq!(edited["objects"][0]["name"], "Kerb north");
    assert_eq!(edited["objects"][0]["layer"], "ROADS");
    let edited = ok_json(
        &mut s,
        "edit_entities",
        json!({ "kind": "texts", "filter": { "names": ["Easement A"] }, "set": { "text": "Easement A (3.0 wide)" } }),
    );
    assert_eq!(edited["objects"][0]["text"], "Easement A (3.0 wide)");
    assert!(
        call(&mut s, "edit_entities", json!({ "set": { "name": "x" } })).1,
        "a filter is required"
    );
    assert!(
        call(
            &mut s,
            "edit_entities",
            json!({ "filter": { "code": "KB" }, "set": { "colour": 1 } })
        )
        .1
    );

    // 6. help.
    let help = ok(&mut s, "run_commands", json!({ "lines": ["help"] }));
    assert!(
        help.contains("polyline, pl") && help.contains("Point input"),
        "{help}"
    );
    assert!(!help.contains("Unknown command"));

    // 7. Lines from plan coordinates have unknown heights.
    let line = ok_json(&mut s, "add_polyline", json!({ "vertices": [[0, 10], [10, 10]] }));
    assert_eq!(line["added"][0]["heights_known"], false);
    assert_eq!(line["added"][0]["vertices"][0], json!([0.0, 10.0]));
    assert!(
        call(
            &mut s,
            "add_polyline",
            json!({ "vertices": [[0, 10], [10, 10, 5]] })
        )
        .1
    );

    // 8. Saving into a new folder; many records in one call; leaving a tool.
    ok(
        &mut s,
        "save_project",
        json!({ "path": dir.join("new/folder/job.s3d") }),
    );
    for corners in [[30.0, 0.0], [50.0, 0.0], [70.0, 0.0]] {
        let [e, n] = corners;
        ok(
            &mut s,
            "add_polyline",
            json!({ "vertices": [[e, n], [e + 10.0, n], [e + 10.0, n + 10.0], [e, n + 10.0]], "closed": true, "layer": "LOTS" }),
        );
    }
    ok(&mut s, "select", json!({ "filter": { "layer": "LOTS" } }));
    ok(&mut s, "run_commands", json!({ "lines": ["mkparcel"] }));
    let parcels = ok_json(&mut s, "query", json!({ "kind": "parcel" }));
    let items: Vec<Value> = parcels["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|p| json!({ "id": p["id"], "record": { "plan": "DP 1234" } }))
        .collect();
    let set = ok_json(&mut s, "set_record", json!({ "kind": "parcel", "items": items }));
    assert_eq!(set.as_array().map(Vec::len), Some(3));
    assert!(
        set.as_array()
            .expect("set")
            .iter()
            .all(|p| p["plan"] == "DP 1234")
    );
    let info = ok_json(&mut s, "project_info", json!({}));
    assert_eq!(info["undo"], "Set 3 parcel records", "one undo step");

    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["inverse", "1", "2", "", "list"] }),
    );
    assert!(
        out.contains("Prompt: Command:") && out.contains("Listing (3 objects)"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn surfaces_are_created_styled_rebuilt_and_deleted() {
    let mut s = Server::new_for_tests();
    // A 5 × 5 grid of ground shots on a slope, a plan-only kerb and a limit.
    let shots: Vec<Value> = (0..25)
        .map(|i| {
            let (x, y) = (f64::from(i % 5) * 10.0, f64::from(i / 5) * 10.0);
            json!({ "e": x, "n": y, "z": 100.0 + x * 0.1, "code": "NS" })
        })
        .collect();
    ok(
        &mut s,
        "add_points",
        json!({ "points": shots, "layer": "GROUND" }),
    );
    ok(
        &mut s,
        "add_points",
        json!({ "points": [{ "e": 25.0, "n": 25.0, "code": "NS" }], "layer": "GROUND" }),
    );
    ok(
        &mut s,
        "add_polyline",
        json!({ "vertices": [[5, 5], [35, 5]], "code": "KB" }),
    );
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "SURFACE_LIMIT", "vertices": [[0, 0], [40, 0], [40, 40], [0, 40]], "closed": true }),
    );

    let made = ok_json(
        &mut s,
        "create_surface",
        json!({
            "name": "Existing ground",
            "point_layers": ["GROUND"],
            "breakline_codes": ["KB"],
            "boundary": "SURFACE_LIMIT",
            "style": { "color": "#336699", "contours": { "show": true, "interval": 0.5 } }
        }),
    );
    assert_eq!(made["points"], 25, "{made}");
    assert!(made["triangles"].as_u64() > Some(10));
    assert_eq!(made["heights"], json!([100.0, 104.0]));
    assert_eq!(made["style"]["color"], "#336699");
    assert_eq!(made["style"]["contours"]["interval"], 0.5);
    let warnings = made["warnings"].to_string();
    assert!(
        warnings.contains("1 selected points have no height"),
        "{warnings}"
    );
    assert!(
        warnings.contains("1 selected breaklines are plan-only"),
        "{warnings}"
    );
    let id = made["id"].as_i64().expect("id");
    assert_eq!(
        ok_json(&mut s, "project_info", json!({}))["surfaces"][0]["name"],
        "Existing ground"
    );

    let (text, error) = call(&mut s, "create_surface", json!({ "name": "existing ground" }));
    assert!(error && text.contains("already"), "{text}");
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "open", "vertices": [[0, 0], [1, 1]] }),
    );
    let (text, error) = call(
        &mut s,
        "create_surface",
        json!({ "name": "X", "boundary": "open" }),
    );
    assert!(error && text.contains("not closed"), "{text}");

    // Restyle, rename, then rebuild after the ground changes.
    let changed = ok_json(
        &mut s,
        "set_surface",
        json!({ "surface": id, "rename": "EGL", "style": { "contours": { "major_every": 4 } } }),
    );
    assert_eq!(changed["name"], "EGL");
    assert_eq!(
        changed["style"]["contours"]["interval"], 0.5,
        "other style fields kept"
    );
    assert_eq!(changed["style"]["contours"]["major_every"], 4);
    ok(
        &mut s,
        "add_points",
        json!({ "points": [{ "e": 15.0, "n": 15.0, "z": 120.0 }], "layer": "GROUND" }),
    );
    let rebuilt = ok_json(
        &mut s,
        "set_surface",
        json!({ "surface": "EGL", "rebuild": true }),
    );
    assert_eq!(rebuilt["points"], 26);
    assert_eq!(rebuilt["heights"][1], 120.0);
    let redefined = ok_json(
        &mut s,
        "set_surface",
        json!({ "surface": "EGL", "boundary": null, "max_edge": 15.0 }),
    );
    assert_eq!(redefined["definition"]["boundary"], Value::Null);

    // Contours show on the sheet.
    let plan = s.call("render", &json!({ "width": 400 })).to_json();
    assert_eq!(plan["isError"], false, "{plan}");

    ok(&mut s, "delete_surface", json!({ "surface": "EGL" }));
    assert_eq!(ok_json(&mut s, "project_info", json!({}))["surfaces"], json!([]));
    ok(&mut s, "run_commands", json!({ "lines": ["undo"] }));
    assert_eq!(
        ok_json(&mut s, "query", json!({ "kind": "surfaces" }))["total"],
        1
    );
    assert!(call(&mut s, "set_surface", json!({ "surface": 99, "rebuild": true })).1);
}

/// The lot example from run_commands' description: bearing & distance legs,
/// closed with the misclosure reported, then made a parcel.
#[test]
fn lot_by_bearing_and_distance() {
    let mut s = Server::new_for_tests();
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["bd", "1000,2000", "0 25.00", "90 40.00", "180 25.00", "270 40.00", "c"] }),
    );
    assert!(out.contains("Closed 4 legs: misclose 0.000"), "{out}");
    assert!(out.contains("area 1000.000"), "{out}");
    let points = ok_json(&mut s, "query", json!({ "kind": "points" }));
    assert_eq!(points["total"], 4, "start and three corners");
    let lines = ok_json(&mut s, "query", json!({ "kind": "polylines" }));
    assert_eq!(lines["total"], 1);
    assert_eq!(lines["rows"][0]["closed"], true);
    ok(&mut s, "run_commands", json!({ "lines": ["select"] }));
    ok(&mut s, "select", json!({ "kind": "polylines" }));
    let out = ok(&mut s, "run_commands", json!({ "lines": ["mkparcel"] }));
    assert!(out.contains("Created 1 parcel"), "{out}");
}

#[test]
fn site_calibration_previews_then_applies() {
    let mut s = Server::new_for_tests();
    // GNSS (MGA) shots of four control marks and a detail point, and the
    // marks' local coordinates: the local grid is the MGA one shifted, turned
    // 1° clockwise and 40 m lower.
    let (cx, cy) = (391_000.0, 6_450_000.0);
    let turn = 1f64.to_radians();
    let local = |e: f64, n: f64| {
        let (de, dn) = (e - cx, n - cy);
        (
            1000.0 + de * turn.cos() + dn * turn.sin(),
            5000.0 - de * turn.sin() + dn * turn.cos(),
        )
    };
    let mut gnss = Vec::new();
    let mut control = Vec::new();
    for (i, (de, dn)) in [(-300.0, -250.0), (320.0, -260.0), (310.0, 280.0), (-290.0, 270.0)]
        .iter()
        .enumerate()
    {
        let (e, n) = (cx + de, cy + dn);
        gnss.push(json!({ "name": format!("G{}", i + 1), "e": e, "n": n, "z": 60.0 + i as f64 }));
        let (le, ln) = local(e, n);
        control.push(json!({ "name": format!("C{}", i + 1), "e": le, "n": ln, "z": 20.0 + i as f64 }));
    }
    gnss.push(json!({ "name": "D1", "e": cx + 10.0, "n": cy + 20.0, "z": 61.5 }));
    ok(&mut s, "add_points", json!({ "points": gnss, "layer": "GNSS" }));
    ok(
        &mut s,
        "add_points",
        json!({ "points": control, "layer": "Control" }),
    );
    // C4 mistyped by 10 cm: it shows up in the preview.
    let pairs: Vec<Value> = (1..=4)
        .map(|i| json!({ "measured": format!("G{i}"), "control": format!("C{i}") }))
        .collect();
    let preview = ok_json(&mut s, "site_calibration", json!({ "pairs": pairs }));
    assert!(preview["preview"].is_string());
    assert!(preview["max_residual_h"].as_f64().expect("max") < 1e-3);
    assert!(preview["max_residual_v"].as_f64().expect("max") < 1e-3);
    assert!(
        (preview["horizontal"]["rotation_degrees_clockwise"]
            .as_f64()
            .expect("rot")
            - 1.0)
            .abs()
            < 1e-9
    );
    assert!(preview["warnings"].as_array().expect("w").is_empty(), "{preview}");
    assert_eq!(preview["vertical"]["method"], "plane");
    let d1 = |s: &mut Server| {
        let q = ok_json(
            s,
            "query",
            json!({ "kind": "points", "filter": { "names": ["D1"] } }),
        );
        (
            q["rows"][0]["e"].as_f64().expect("e"),
            q["rows"][0]["n"].as_f64().expect("n"),
            q["rows"][0]["z"].as_f64().expect("z"),
        )
    };
    assert_eq!(d1(&mut s).0, cx + 10.0, "preview changes nothing");

    // A doubtful pair is flagged; turning it off clears the flag.
    let mut bad = pairs.clone();
    bad[3] = json!({ "measured": "G4", "control": format!("{},{}", local(cx - 290.0, cy + 270.0).0 + 0.1, local(cx - 290.0, cy + 270.0).1) });
    let flagged = ok_json(&mut s, "site_calibration", json!({ "pairs": bad }));
    assert_eq!(flagged["pairs"][3]["over_limit"], true, "{flagged}");
    assert!(
        flagged["warnings"]
            .to_string()
            .contains("over the residual limits")
    );
    bad[3]["horizontal"] = json!(false);
    let left_out = ok_json(&mut s, "site_calibration", json!({ "pairs": bad }));
    assert!(left_out["max_residual_h"].as_f64().expect("max") < 1e-3);
    assert!(
        (left_out["pairs"][3]["residual_e"].as_f64().expect("e") - 0.1).abs() < 1e-3,
        "{left_out}"
    );
    assert_eq!(
        left_out["pairs"][3]["over_limit"], false,
        "not used, so not flagged"
    );

    let applied = ok_json(
        &mut s,
        "site_calibration",
        json!({ "pairs": pairs, "apply": "all_except_control" }),
    );
    assert!(
        applied["applied"]
            .as_str()
            .expect("summary")
            .contains("applied to 5 objects"),
        "{applied}"
    );
    let (e, n, z) = d1(&mut s);
    let (le, ln) = local(cx + 10.0, cy + 20.0);
    assert!(
        (e - le).abs() < 1e-3 && (n - ln).abs() < 1e-3 && (z - 21.5).abs() < 0.05,
        "{e} {n} {z}"
    );
    let info = ok_json(&mut s, "project_info", json!({}));
    assert_eq!(info["site_calibration"]["applied"], true);
    assert_eq!(info["site_calibration"]["pairs"], 4);
    // One undoable step.
    ok(&mut s, "run_commands", json!({ "lines": ["undo"] }));
    assert_eq!(d1(&mut s).0, cx + 10.0);
}

#[test]
fn surface_clipping_holes_and_breaklines_end_to_end() {
    let mut s = Server::new_for_tests();
    // Ground on the plane z = 50 + 0.1 e over 0..100 × 0..100.
    let shots: Vec<Value> = (0..=10)
        .flat_map(|i| (0..=10).map(move |j| (i as f64 * 10.0, j as f64 * 10.0)))
        .map(|(e, n)| json!({ "e": e, "n": n, "z": 50.0 + 0.1 * e }))
        .collect();
    ok(
        &mut s,
        "add_points",
        json!({ "points": shots, "layer": "GROUND" }),
    );
    // A pond rim (closed 3D breakline, picked by code), lower than the ground.
    ok(
        &mut s,
        "add_polyline",
        json!({ "vertices": [[62, 62, 40], [78, 62, 40], [78, 78, 40], [62, 78, 40]], "closed": true, "code": "POND" }),
    );
    // A boundary that runs off the data on the west and south, and a hole.
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "LIMIT", "vertices": [[-30, -30], [85, -30], [85, 85], [-30, 85]], "closed": true }),
    );
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "SHED", "vertices": [[20, 20], [30, 20], [30, 30], [20, 30]], "closed": true }),
    );
    let made = ok_json(
        &mut s,
        "create_surface",
        json!({ "name": "EG", "breakline_codes": ["POND"], "boundary": "LIMIT", "holes": ["SHED"] }),
    );
    assert!(made["warnings"].as_array().expect("w").is_empty(), "{made}");
    let id = made["id"].as_i64().expect("id");
    let doc = s.doc();
    let tin = &doc.surface(survey_model::SurfaceId(id)).expect("surface").tin;
    // Data 0..100 clipped to 0..85, less the 10 × 10 shed.
    assert!(
        (tin.plan_area() - (85.0 * 85.0 - 100.0)).abs() < 1e-6,
        "{}",
        tin.plan_area()
    );
    let index = tin.index();
    let at = |e: f64, n: f64| index.height_at(tin, glam::DVec2::new(e, n));
    // On the rim the surface is at the rim's height; inside the shed there is none.
    assert!((at(70.0, 62.0).expect("rim") - 40.0).abs() < 1e-9);
    assert!((at(78.0, 70.0).expect("rim") - 40.0).abs() < 1e-9);
    assert!(at(25.0, 25.0).is_none());
    // Clipped exactly at the boundary, at the ground's height there.
    assert!((at(85.0, 10.0).expect("edge") - 58.5).abs() < 1e-9);
    assert!(at(86.0, 10.0).is_none());

    // The boundary deleted: the rebuild says so instead of silently not clipping.
    ok(&mut s, "select", json!({ "filter": { "names": ["LIMIT"] } }));
    ok(&mut s, "run_commands", json!({ "lines": ["delete"] }));
    let rebuilt = ok_json(&mut s, "set_surface", json!({ "surface": id, "rebuild": true }));
    assert!(
        rebuilt["warnings"]
            .to_string()
            .contains("boundary polyline has been deleted"),
        "{rebuilt}"
    );
}

#[test]
fn layer_rename_carries_through_and_locks_hold() {
    let mut s = Server::new_for_tests();
    ok(
        &mut s,
        "set_feature_codes",
        json!({ "codes": [{ "code": "TB", "layer": "BANKS" }] }),
    );
    ok(&mut s, "set_layer", json!({ "layer": "BANKS" }));
    let shots: Vec<Value> = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
        .iter()
        .map(|(e, n)| json!({ "e": e, "n": n, "z": 5.0, "code": "TB" }))
        .collect();
    ok(&mut s, "add_points", json!({ "points": shots, "layer": "BANKS" }));
    ok(
        &mut s,
        "create_surface",
        json!({ "name": "EG", "point_layers": ["BANKS"], "breakline_layers": ["BANKS"] }),
    );

    // Bad names are refused.
    let (text, error) = call(&mut s, "set_layer", json!({ "layer": "BANKS", "rename": "  " }));
    assert!(error && text.contains("needs a name"), "{text}");
    let (text, error) = call(&mut s, "set_layer", json!({ "layer": " " }));
    assert!(error, "{text}");
    ok(&mut s, "set_layer", json!({ "layer": "ROADS" }));
    let (text, error) = call(
        &mut s,
        "set_layer",
        json!({ "layer": "BANKS", "rename": "roads" }),
    );
    assert!(error && text.contains("already a layer"), "{text}");

    // A rename reaches the feature codes and the surface definition.
    ok(
        &mut s,
        "set_layer",
        json!({ "layer": "BANKS", "rename": "TOP OF BANK" }),
    );
    let doc = s.doc();
    assert_eq!(doc.meta().feature_codes.codes[0].layer, "TOP OF BANK");
    let def = doc
        .surfaces()
        .next()
        .expect("surface")
        .1
        .definition
        .clone()
        .expect("def");
    assert_eq!(def.point_layers, ["TOP OF BANK"]);
    assert_eq!(def.breakline_layers, ["TOP OF BANK"]);
    let rebuilt = ok_json(&mut s, "set_surface", json!({ "surface": "EG", "rebuild": true }));
    assert_eq!(rebuilt["points"], 4, "the surface still finds its points");

    // Locked: edits are refused, and undoing the rename leaves the lock alone.
    ok(
        &mut s,
        "set_layer",
        json!({ "layer": "TOP OF BANK", "locked": true, "visible": false }),
    );
    let (text, error) = call(
        &mut s,
        "edit_entities",
        json!({ "filter": { "layer": "TOP OF BANK" }, "set": { "code": "XX" } }),
    );
    assert!(error && text.contains("locked"), "{text}");
    ok(&mut s, "run_commands", json!({ "lines": ["undo"] }));
    let doc = s.doc();
    let layer = doc.layer_by_name("BANKS").expect("rename undone");
    assert!(layer.locked && !layer.visible, "display state kept through undo");
    assert_eq!(doc.meta().feature_codes.codes[0].layer, "BANKS");
}

/// Phase 9 through the agent's tools: a lot with a curved corner keyed by
/// bearing and distance and named as a parcel, a lot closed with a computed
/// leg, the curve solver, a dimension, line and curve tables on a plan sheet,
/// and Carlson-style control codes.
#[test]
fn cadastral_deliverables_end_to_end() {
    let mut s = Server::new_for_tests();
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["bd", "1000,2000", "0 20", "90 10", "r=10 d=90 r", "180 10", "270 20", "c", "Lot 7"] }),
    );
    let text = out.clone();
    assert!(text.contains("misclose") && text.contains("exact"), "{text}");
    let parcels = ok_json(&mut s, "query", json!({ "kind": "parcel" }));
    assert_eq!(parcels["rows"][0]["name"], "Lot 7", "{parcels}");
    let lot = ok_json(&mut s, "query", json!({ "kind": "polylines" }));
    let area = lot["rows"][0]["area"].as_f64().expect("area");
    assert!(
        (area - (300.0 + 25.0 * std::f64::consts::PI)).abs() < 1e-6,
        "{area}"
    );
    assert!(lot["rows"][0]["bulges"].is_array(), "the curved corner is an arc");

    // A second lot, closed with the computed leg and the next lot number.
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["bd", "1020,2000", "0 20", "90 15", "j", "y"] }),
    );
    assert!(out.contains("Closing leg"), "{out}");
    let parcels = ok_json(&mut s, "query", json!({ "kind": "parcel" }));
    assert_eq!(parcels["total"], 2);
    assert_eq!(parcels["rows"][1]["name"], "Lot 8");

    // The curve solver: radius and chord have a minor and a major arc.
    let solved = ok_json(&mut s, "solve_curve", json!({ "radius": 10, "chord": 10 }));
    let solutions = solved["solutions"].as_array().expect("solutions");
    assert_eq!(solutions.len(), 2);
    assert!((solutions[0]["delta_degrees"].as_f64().expect("delta") - 60.0).abs() < 1e-6);
    assert!(
        call(&mut s, "solve_curve", json!({ "radius": 10 })).1,
        "one element is not enough"
    );
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["curve", "r=20 d=90", ""] }),
    );
    assert!(out.contains("31.416"), "{out}");

    // A dimension along the first lot's frontage, 3 m outside it.
    let added = ok_json(
        &mut s,
        "add_dimension",
        json!({ "dimensions": [{ "from": [1000.0, 2000.0], "to": [1000.0, 2020.0], "offset": 3.0, "text": "<> (frontage)" }] }),
    );
    assert_eq!(added["added"][0]["shows"], "20.000 (frontage)", "{added}");
    assert_eq!(
        added["added"][0]["line"][0][0], 997.0,
        "left of south → north is west"
    );
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["dim", "h", "1000,2000", "1035,2000", "4", ""] }),
    );
    assert!(out.contains("35.000"), "{out}");
    let dims = ok_json(&mut s, "query", json!({ "kind": "dimensions" }));
    assert_eq!(dims["total"], 2);
    assert_eq!(dims["rows"][1]["kind"], "horizontal");
    let id = dims["rows"][1]["id"].as_i64().expect("id");
    ok(
        &mut s,
        "edit_entities",
        json!({ "filter": { "ids": [id] }, "set": { "text": "<> overall" } }),
    );
    let dims = ok_json(&mut s, "query", json!({ "kind": "dimensions" }));
    assert_eq!(dims["rows"][1]["shows"], "35.000 overall");

    // A plan sheet with every boundary in line and curve tables.
    let sheet = ok_json(
        &mut s,
        "set_record",
        json!({ "kind": "sheet", "record": { "name": "Plan", "centre": [1020.0, 2010.0], "scale": 250.0,
                "content": { "tables": "all" } } }),
    );
    assert_eq!(sheet["content"]["tables"], "all");
    let record = s
        .doc()
        .record::<SheetRecord>(RecordId(sheet["id"].as_i64().expect("id")))
        .cloned()
        .expect("sheet");
    let plot = s.session.plot(&record);
    let svg = survey_io::plot::write_svg(&plot);
    for text in [
        // The lot's area is exact, the arc included (not from its drawn chords).
        "378.540",
        "LINE TABLE",
        "CURVE TABLE",
        ">L1<",
        ">C1<",
        "20.000 (frontage)",
        "Lot 7",
    ] {
        assert!(svg.contains(text), "{text} on the plan");
    }
    if let Ok(dir) = std::env::var("SURVEY3D_PLOT_DIR") {
        let png = svg_to_png(&svg, 2400).expect("png");
        std::fs::write(std::path::Path::new(&dir).join("phase9.png"), png).expect("write");
    }

    // Carlson-style control codes: a rectangle from two points and a tree circle.
    ok(&mut s, "set_feature_codes", json!({ "preset": "example" }));
    ok(
        &mut s,
        "add_points",
        json!({ "points": [
            { "e": 1100.0, "n": 2000.0, "code": "BLD" },
            { "e": 1100.0, "n": 2012.0, "code": "BLD RECT8" },
            { "e": 1120.0, "n": 2005.0, "code": "TR CIR1.5" }
        ] }),
    );
    let out = ok(&mut s, "run_commands", json!({ "lines": ["linework"] }));
    assert!(out.contains("2 strings"), "{out}");
    let lines = ok_json(
        &mut s,
        "query",
        json!({ "kind": "polylines", "filter": { "code": "BLD" } }),
    );
    assert_eq!(lines["rows"][0]["closed"], true);
    assert_eq!(lines["rows"][0]["area"], 96.0);
}

#[test]
fn field_data_end_to_end() {
    let dir = temp_dir("field");
    let mut s = Server::new_for_tests();
    ok(
        &mut s,
        "add_points",
        json!({ "points": [
            { "name": "1", "e": 1000.0, "n": 2000.0, "z": 50.0, "code": "CP" },
            { "name": "2", "e": 1000.0, "n": 2100.0, "z": 51.0, "code": "CP" }
        ] }),
    );
    ok(
        &mut s,
        "set_feature_codes",
        json!({ "codes": [{ "code": "KB1", "linework": true, "layer": "KERB" }] }),
    );
    // A day's RW5: topo from 1 (backsight 2), a traverse leg to 20, and a peg
    // from 20 (backsight 1). Rod 1.5 m throughout; the kerb end was really 2.0.
    let rw5 = "\
JB,NMDAY1,DT10-27-2026,TM08:00:00
MO,AD0,UN1,SF1.00000000,EC0,EO0.0,AU0
OC,OP1,N 2000.0000,E 1000.0000,EL50.000,--CP
LS,HI1.500,HR1.500
BK,OP1,BP2,BC0.0000
BD,OP1,FP2,AR0.0000,ZE90.0000,SD100.000,--CP
SS,OP1,FP10,AR90.0000,ZE90.0000,SD20.000,--KB1 ST
SS,OP1,FP11,AR90.0000,ZE90.0000,SD30.000,--KB1
SS,OP1,FP12,AR90.0000,ZE90.0000,SD40.000,--KB1 END
TR,OP1,FP20,AR180.0000,ZE90.0000,SD50.000,--TRAV
OC,OP20,--TRAV
LS,HI1.600,HR1.500
BK,OP20,BP1,BC0.0000
BD,OP20,FP1,AR0.0000,ZE90.0000,SD50.000
SS,OP20,FP30,AR90.0000,ZE90.0000,SD10.000,--PEG
";
    let path = dir.join("day1.rw5");
    std::fs::write(&path, rw5).expect("write rw5");
    let out = ok(&mut s, "import_file", json!({ "path": path }));
    assert!(out.contains("Carlson RW5") && out.contains("2 setups"), "{out}");

    let shown = ok_json(&mut s, "raw_data", json!({}));
    assert_eq!(shown["setups"][1]["backsight"], "1");
    let shots = ok_json(&mut s, "raw_data", json!({ "setup": 1 }));
    assert_eq!(shots["setups"][0]["shot_list"][3]["code"], "KB1 END");

    let reduced = ok_json(&mut s, "raw_data", json!({ "action": "reduce" }));
    assert!(
        reduced["summary"]
            .as_str()
            .expect("summary")
            .contains("5 new points"),
        "{reduced}"
    );
    let checks = reduced["checks"].as_array().expect("checks");
    assert_eq!(checks.len(), 2, "both backsights");
    assert!(
        checks.iter().all(|c| c["plan"].as_f64() < Some(1e-6)),
        "{reduced}"
    );
    let point = |s: &mut Server, name: &str| {
        let q = ok_json(
            s,
            "query",
            json!({ "kind": "points", "filter": { "names": [name] } }),
        );
        q["rows"][0].clone()
    };
    let p30 = point(&mut s, "30");
    assert!((p30["e"].as_f64().expect("e") - 1010.0).abs() < 1e-6, "{p30}");
    assert!((p30["n"].as_f64().expect("n") - 1950.0).abs() < 1e-6);
    assert!((p30["z"].as_f64().expect("z") - 50.1).abs() < 1e-6);
    assert_eq!(
        point(&mut s, "10")["layer"],
        "KERB",
        "coded points go on their code's layer"
    );

    // The correction re-reduces in the same step and is audited.
    let out = ok(
        &mut s,
        "raw_data",
        json!({ "action": "edit", "setup": 1, "shot": 4, "target_height": 2.0 }),
    );
    assert!(out.contains("target height 1.500 → 2.000"), "{out}");
    assert!((point(&mut s, "12")["z"].as_f64().expect("z") - 49.5).abs() < 1e-6);
    let audit = ok(&mut s, "query", json!({ "kind": "audit", "limit": 1 }));
    assert!(audit.contains("Raw edit day1 setup 1 shot 4"), "{audit}");
    assert!(
        call(
            &mut s,
            "raw_data",
            json!({ "action": "edit", "setup": 1, "shot": 4, "instrument_height": 1.0 })
        )
        .1,
        "setup fields need no shot"
    );
    let report = ok(&mut s, "raw_data", json!({ "action": "report" }));
    assert!(
        report.contains("OFFICE CORRECTIONS") && report.contains("backsight 2"),
        "{report}"
    );

    // Linework from the coded shots.
    ok(&mut s, "run_commands", json!({ "lines": ["linework"] }));
    let lines = ok_json(&mut s, "query", json!({ "kind": "polylines" }));
    assert_eq!(lines["total"], 1, "{lines}");
    assert_eq!(lines["rows"][0]["vertices"].as_array().map(Vec::len), Some(3));

    // Stake-out files of the kerb (selected) and every point.
    let kerb = lines["rows"][0]["id"].clone();
    ok(
        &mut s,
        "select",
        json!({ "kind": "polylines", "filter": { "ids": [kerb] } }),
    );
    let jxl = dir.join("kerb.jxl");
    let out = ok(
        &mut s,
        "export_file",
        json!({ "format": "jobxml", "path": jxl, "selection": true }),
    );
    assert!(out.contains("3 points, 2 lines"), "{out}");
    let text = std::fs::read_to_string(&jxl).expect("jxl");
    assert!(
        text.contains("<StartPoint>10</StartPoint>") && text.contains("<Name>12</Name>"),
        "{text}"
    );
    let out = ok(
        &mut s,
        "export_file",
        json!({ "format": "csv", "path": dir.join("kerb.csv"), "selection": true }),
    );
    assert!(out.contains("CSV") && out.contains("3 points"), "{out}");
    for (format, name) in [("rw5", "all.rw5"), ("gsi", "all.gsi")] {
        let out = ok(
            &mut s,
            "export_file",
            json!({ "format": format, "path": dir.join(name) }),
        );
        assert!(out.contains("7 points"), "{format}: {out}");
    }
    let back = survey_io::rawdata::read_raw("all", &std::fs::read(dir.join("all.gsi")).expect("gsi"))
        .expect("gsi reads back");
    assert_eq!(back.points.len(), 7);

    // As-built: pegs set out and measured as 30S, 3 mm and 25 mm out.
    ok(
        &mut s,
        "add_points",
        json!({ "points": [
            { "name": "30S", "e": 1010.003, "n": 1950.0, "z": 50.1 },
            { "name": "11S", "e": 1030.0, "n": 2000.025, "z": 50.0 }
        ] }),
    );
    let c = ok_json(
        &mut s,
        "compare_asbuilt",
        json!({ "shots": ["30S", "11S"], "strip": "S", "tolerance_h": 0.02, "label": true }),
    );
    assert_eq!(c["compared"], 2, "{c}");
    assert_eq!(c["over"], 1);
    assert_eq!(c["rows"][0]["against"], "30");
    assert!((c["rows"][0]["de"].as_f64().expect("de") - 0.003).abs() < 1e-6);
    assert!(c["labelled"].as_str().is_some());
    let c = ok_json(
        &mut s,
        "compare_asbuilt",
        json!({ "shots": ["11S"], "against": "lines", "lines": [kerb], "report": true }),
    );
    assert!(
        (c["rows"][0]["offset"].as_f64().expect("offset") + 0.025).abs() < 1e-6,
        "{c}"
    );
    assert!((c["rows"][0]["chainage"].as_f64().expect("ch") - 10.0).abs() < 1e-6);
    assert!(
        c["report"]
            .as_str()
            .expect("report")
            .contains("AS-BUILT COMPARISON")
    );
}

#[test]
fn cogo_angles_intersections_trim_and_point_tools() {
    let mut s = Server::new_for_tests();
    // A lot keyed by turned angles from a backsight.
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["bd", "0,0", "bs 0", "a90.0000 20", "a90.0000 10", "a90.0000 20", "a90.0000 10", "c", ""] }),
    );
    assert!(out.contains("exact"), "{out}");
    let lot = ok_json(&mut s, "query", json!({ "kind": "polylines" }));
    assert!(
        (lot["rows"][0]["area"].as_f64().expect("area") - 200.0).abs() < 1e-6,
        "{lot}"
    );

    // Two building faces' corner.
    ok(
        &mut s,
        "add_points",
        json!({ "points": [
            { "name": "A1", "e": 100.0, "n": 0.0 }, { "name": "A2", "e": 105.0, "n": 0.0 },
            { "name": "B1", "e": 110.0, "n": 2.0 }, { "name": "B2", "e": 110.0, "n": 8.0 }
        ] }),
    );
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["ix", "A1 A2", "B1 B2", ""] }),
    );
    assert!(out.contains("at the intersection"), "{out}");

    // Trim a kerb at a crossing line, then extend it back.
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "KERB", "vertices": [[200, 0], [220, 0]] }),
    );
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "DRIVE", "vertices": [[210, -5], [210, 5]] }),
    );
    let t = ok_json(
        &mut s,
        "trim_extend",
        json!({ "target": "KERB", "boundaries": ["DRIVE"], "at": [215, 0] }),
    );
    assert_eq!(t["summary"], "Trimmed", "{t}");
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "FENCE", "vertices": [[230, -5], [230, 5]] }),
    );
    let t = ok_json(
        &mut s,
        "trim_extend",
        json!({ "action": "extend", "target": "KERB", "boundaries": ["FENCE"], "at": [209, 0] }),
    );
    assert!(
        t["summary"].as_str().expect("summary").contains("by 20.000"),
        "{t}"
    );
    assert!(
        call(
            &mut s,
            "trim_extend",
            json!({ "target": "KERB", "boundaries": ["NOPE"], "at": [0, 0] })
        )
        .1
    );

    // Renumber the building points, then raise them.
    ok(
        &mut s,
        "select",
        json!({ "kind": "points", "filter": { "names": ["A1", "A2"] } }),
    );
    let out = ok(&mut s, "run_commands", json!({ "lines": ["renumber 501"] }));
    assert!(out.contains("Renumber 2 points"), "{out}");
    let q = ok_json(
        &mut s,
        "query",
        json!({ "kind": "points", "filter": { "names": ["501", "502"] } }),
    );
    assert_eq!(q["total"], 2, "{q}");
}

#[test]
fn survey_computations_end_to_end() {
    let mut s = Server::new_for_tests();
    let close = |v: &Value, want: f64| (v.as_f64().unwrap_or(f64::NAN) - want).abs() < 1e-3;
    // Two flat surfaces 2 m apart, and a pyramid stockpile on a 20 m square toe.
    let mut points = Vec::new();
    for (code, x0, z) in [("EG", 0.0, 10.0), ("DS", 0.0, 12.0), ("SP", 100.0, 10.0)] {
        for (dx, dy) in [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)] {
            points.push(json!({ "e": x0 + dx, "n": dy, "z": z, "code": code }));
        }
    }
    points.push(json!({ "e": 110.0, "n": 10.0, "z": 15.0, "code": "SP" }));
    ok(&mut s, "add_points", json!({ "points": points }));
    for name in ["EG", "DS"] {
        ok(
            &mut s,
            "create_surface",
            json!({ "name": name, "point_codes": [name] }),
        );
    }
    ok(
        &mut s,
        "create_surface",
        json!({ "name": "PILE", "point_codes": ["SP"] }),
    );
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "BND", "vertices": [[5, 5], [15, 5], [15, 15], [5, 15]], "closed": true }),
    );
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "TOE", "vertices": [[100, 0], [120, 0], [120, 20], [100, 20]], "closed": true }),
    );

    let v = ok_json(
        &mut s,
        "compute_volumes",
        json!({ "base": "EG", "top": "DS", "density": 2.0 }),
    );
    assert!(close(&v["fill"], 800.0) && close(&v["cut"], 0.0), "{v}");
    assert!(close(&v["tonnes"]["net"], 1600.0), "{v}");
    let v = ok_json(
        &mut s,
        "compute_volumes",
        json!({ "base_level": 9, "top": "EG", "boundary": "BND" }),
    );
    assert!(close(&v["fill"], 100.0), "{v}");
    let v = ok_json(
        &mut s,
        "compute_volumes",
        json!({ "base": "DS", "top": "EG", "create_surface": true }),
    );
    assert!(
        close(&v["net"], -800.0) && v["created_surface"]["name"] == "EG − DS",
        "{v}"
    );
    let (text, error) = call(
        &mut s,
        "compute_volumes",
        json!({ "base": "EG", "base_level": 3, "top": "DS" }),
    );
    assert!(error && text.contains("not both"), "{text}");
    let p = ok_json(
        &mut s,
        "stockpile_volume",
        json!({ "surface": "PILE", "toe": "TOE", "density": 1.6 }),
    );
    assert!(close(&p["volume"], 2000.0 / 3.0), "{p}");

    // Sections: a regular table, and cross sections square to the line.
    ok(
        &mut s,
        "add_polyline",
        json!({ "name": "CL", "vertices": [[0, 10], [20, 10]] }),
    );
    let sec = ok_json(
        &mut s,
        "sections",
        json!({ "line": "CL", "sample": 10, "surfaces": ["EG", "DS"] }),
    );
    assert_eq!(sec["section"].as_array().map(Vec::len), Some(3), "{sec}");
    assert!(close(&sec["section"][1]["DS"], 12.0), "{sec}");
    let sec = ok_json(
        &mut s,
        "sections",
        json!({ "line": "CL", "kind": "cross", "interval": 10, "left": 5, "right": 5, "sample": 5 }),
    );
    assert_eq!(sec["sections"].as_array().map(Vec::len), Some(3), "{sec}");
    assert!(close(&sec["sections"][0]["section"][0]["at"], -5.0), "{sec}");

    // A closed loop with a 20 mm distance blunder and 10" of angular error.
    ok(
        &mut s,
        "add_points",
        json!({ "points": [
        { "name": "A", "e": 1000.0, "n": 1000.0, "z": 50.0 },
        { "name": "B", "e": 1000.0, "n": 1100.0, "z": 50.0 },
        { "name": "C", "e": 1100.0, "n": 1000.0, "z": 50.0 }
    ] }),
    );
    let loop_args = json!({ "start": "A", "backsight": "B", "legs": [
        { "to": "T1", "angle": "90", "distance": 100 },
        { "to": "T2", "angle": "270", "distance": 100.02 },
        { "to": "T3", "angle": "270.0010", "distance": 100 },
        { "to": "A", "angle": "270", "distance": 99.99 }
    ], "close_on": "A", "closing_reference": "B", "closing_angle": "180", "save": "Loop", "apply": true });
    let t = ok_json(&mut s, "adjust_traverse", loop_args);
    assert!(close(&t["angular"]["seconds"], 10.0), "{t}");
    assert_eq!(t["linear"]["precision"], "1:14681", "{t}");
    let line = t["added"].as_array().and_then(|a| a.last()).expect("polyline");
    assert_eq!(line["heights_known"], false, "plan-only stations: {line}");
    let n = ok_json(&mut s, "adjust_network", json!({ "sources": ["Loop"] }));
    assert_eq!(
        n["free"],
        json!(["T1", "T2", "T3"]),
        "the loop's own stations are adjusted: {n}"
    );
    assert_eq!(n["chi_square"]["pass"], false, "{n}");
    assert_eq!(
        n["largest_residuals"][0]["observation"], "T1 → T2 distance",
        "the blunder: {n}"
    );
    assert!(n["next"].as_str().is_some_and(|t| t.contains("apply")), "{n}");
    let n = ok_json(&mut s, "adjust_network", json!({ "apply": true }));
    assert!(
        n["applied"].as_str().is_some_and(|t| t.contains("3 updated")),
        "{n}"
    );

    // Resection from three distances, then directions and a distance.
    let r = ok_json(
        &mut s,
        "resection",
        json!({ "observations": [
        { "point": "A", "distance": 50 }, { "point": "B", "distance": 80.6226 }, { "point": "C", "distance": 67.0820 }
    ] }),
    );
    assert!(
        close(&r["solutions"][0]["e"], 1040.0) && close(&r["solutions"][0]["n"], 1030.0),
        "{r}"
    );
    let r = ok_json(
        &mut s,
        "resection",
        json!({ "observations": [
        { "point": "A", "direction": "233.0748", "distance": 50 },
        { "point": "B", "direction": "330.1520" },
        { "point": "C", "direction": "116.3354" }
    ], "station": "STN1", "height": 51.2, "apply": true }),
    );
    assert_eq!(r["added"][0]["name"], "STN1", "{r}");
    assert!(close(&r["added"][0]["z"], 51.2), "{r}");

    // A lot with its last course missing, solved and drawn, then subdivided.
    let c = ok_json(
        &mut s,
        "figure_closure",
        json!({ "courses": [
        { "bearing": "90", "distance": 40 }, { "bearing": "180", "distance": 30 }, { "bearing": "270", "distance": 40 }, {}
    ], "start": "A", "name": "Lot 9", "draw": true }),
    );
    assert!(
        close(&c["solved"][1]["distance"], 30.0) && close(&c["area"], 1200.0),
        "{c}"
    );
    let c = ok_json(&mut s, "figure_closure", json!({ "polyline": "Lot 9" }));
    assert!(close(&c["misclosure"], 0.0), "{c}");
    let d = ok_json(
        &mut s,
        "subdivide",
        json!({ "parcel": "Lot 9", "area": 300, "edge": 1 }),
    );
    assert!(close(&d["solutions"][0]["cut_line"][0][0][1], 992.5), "{d}");
    let d = ok_json(
        &mut s,
        "subdivide",
        json!({ "parcel": "Lot 9", "area": 200, "method": "swing", "hinge": 1 }),
    );
    assert_eq!(d["solutions"].as_array().map(Vec::len), Some(2), "{d}");
    let d = ok_json(
        &mut s,
        "subdivide",
        json!({ "parcel": "Lot 9", "area": 300, "edge": 4, "draw": "parcels" }),
    );
    assert_eq!(d["added"].as_array().map(Vec::len), Some(3), "{d}");
    let (text, error) = call(
        &mut s,
        "subdivide",
        json!({ "parcel": "Lot 9", "area": 300, "edge": 9 }),
    );
    assert!(error && text.contains("not a straight edge"), "{text}");
}

#[test]
fn coordinates_geoid_and_setout() {
    let mut s = Server::new_for_tests();
    let close = |v: &Value, want: f64, tol: f64| (v.as_f64().unwrap_or(f64::NAN) - want).abs() < tol;
    // A geoid model of constant N = 5 round Flinders Peak, in a geoid folder.
    let dir = temp_dir("geoid");
    let mut gtx = Vec::new();
    for v in [-39.0f64, 143.0, 1.0, 1.0] {
        gtx.extend_from_slice(&v.to_be_bytes());
    }
    gtx.extend_from_slice(&3i32.to_be_bytes());
    gtx.extend_from_slice(&3i32.to_be_bytes());
    for _ in 0..9 {
        gtx.extend_from_slice(&5.0f32.to_be_bytes());
    }
    // Naming the model before it is downloaded says why it is not loaded, and
    // naming it again once it is there reads it.
    let early = ok_json(
        &mut s,
        "set_project",
        json!({ "coordinate_system": { "name": "MGA2020 zone 55" }, "geoid_dir": dir, "geoid": "flat.gtx" }),
    );
    assert!(
        early["geoid"]
            .as_str()
            .expect("status")
            .contains("could not be read"),
        "{early}"
    );
    std::fs::write(dir.join("flat.gtx"), gtx).expect("geoid");
    let (text, error) = call(&mut s, "set_project", json!({ "geoid_dir": "/no/such/folder" }));
    assert!(error && text.contains("is not a folder"), "{text}");
    let set = ok_json(
        &mut s,
        "set_project",
        json!({ "coordinate_system": { "name": "MGA2020 zone 55" }, "geoid_dir": dir, "geoid": "flat.gtx" }),
    );
    assert_eq!(set["geoid_files"], json!(["flat.gtx"]));
    assert!(
        set["geoid"].as_str().expect("value").starts_with("flat.gtx"),
        "{set}"
    );

    // Geoscience Australia's Flinders Peak example, 105 m above the ellipsoid.
    let lat = -(37.0 + 57.0 / 60.0 + 3.7203 / 3600.0);
    let lon = 144.0 + 25.0 / 60.0 + 29.5244 / 3600.0;
    let c = ok_json(
        &mut s,
        "convert_coordinates",
        json!({ "geographic": [{ "latitude": lat, "longitude": lon, "height": 105.0, "name": "FP" }],
                "height_kind": "ellipsoidal", "create": true }),
    );
    let p = &c["positions"][0];
    assert!(
        close(&p["easting"], 273_741.297, 1e-3) && close(&p["northing"], 5_796_489.777, 1e-3),
        "{p}"
    );
    assert!(
        close(&p["height"], 100.0, 1e-6) && close(&p["geoid_separation"], 5.0, 1e-6),
        "{p}"
    );
    assert_eq!(p["geoid_source"], "flat.gtx");
    assert_eq!(p["latitude_dms"], "-37°57'03.72030\"");
    assert!(close(
        &p["combined_factor"],
        p["point_scale"].as_f64().expect("value") * p["height_factor"].as_f64().expect("value"),
        1e-9
    ));
    assert_eq!(c["created"][0]["name"], "FP");
    // And back from the point.
    let back = ok_json(&mut s, "convert_coordinates", json!({ "points": ["FP"] }));
    assert!(close(&back["positions"][0]["latitude"], lat, 1e-9), "{back}");
    assert!(close(&back["positions"][0]["ellipsoidal_height"], 105.0, 1e-6));

    // GNSS heights onto the geoid.
    ok(
        &mut s,
        "add_points",
        json!({ "points": [{ "name": "G1", "e": 273_800.0, "n": 5_796_500.0, "z": 110.0 }] }),
    );
    let text = ok(
        &mut s,
        "convert_heights",
        json!({ "points": ["G1"], "to": "orthometric" }),
    );
    assert!(text.contains("from ellipsoidal to orthometric"), "{text}");
    let g1 = ok_json(
        &mut s,
        "query",
        json!({ "kind": "points", "filter": { "names": ["G1"] } }),
    );
    assert!(close(&g1["rows"][0]["z"], 105.0, 1e-6), "{g1}");

    // Set-out from a station at Flinders Peak backsighting due north, to a point 100 m east and 1 m up.
    ok(
        &mut s,
        "add_points",
        json!({ "points": [
            { "name": "ST", "e": 273_741.297, "n": 5_796_489.777, "z": 100.0 },
            { "name": "BS", "e": 273_741.297, "n": 5_796_589.777, "z": 100.0 },
            { "name": "T", "e": 273_841.297, "n": 5_796_489.777, "z": 101.0 },
        ] }),
    );
    let args = json!({ "station": "ST", "backsight": "BS", "points": ["T", "G1"], "hi": 1.5, "ht": 2.0 });
    let so = ok_json(&mut s, "setout", args.clone());
    let t = &so["rows"][0];
    assert!(t["angle"].as_str().expect("value").starts_with("90"), "{so}");
    assert!(
        close(&t["distance"], 100.0, 1e-6) && close(&t["dz"], 1.0, 1e-6),
        "{t}"
    );
    assert!(close(&t["slope_distance"], 100.0f64.hypot(1.5), 1e-4), "{t}");
    assert!(close(&so["backsight_distance"], 100.0, 1e-6) && so["distances"] == "grid");

    // Ground distances: a fixed factor, then the projection's along the line.
    ok(
        &mut s,
        "set_project",
        json!({ "distances": "ground", "ground_factor": 0.9996 }),
    );
    let so = ok_json(&mut s, "setout", args.clone());
    assert!(close(&so["rows"][0]["distance"], 100.0 / 0.9996, 1e-4), "{so}");
    assert!(close(&so["rows"][0]["grid_distance"], 100.0, 1e-6) && so["distances"] == "ground");
    ok(&mut s, "set_project", json!({ "ground_factor": "projection" }));
    let mid = ok_json(
        &mut s,
        "convert_coordinates",
        json!({ "grid": [[273_791.297, 5_796_489.777, 100.5]] }),
    );
    let k = mid["positions"][0]["combined_factor"].as_f64().expect("value");
    let so = ok_json(&mut s, "setout", args);
    assert!(close(&so["rows"][0]["distance"], 100.0 / k, 1e-4), "{so} vs {k}");
    let info = ok_json(&mut s, "project_info", json!({}));
    assert_eq!(
        info["distances"],
        json!({ "shown": "ground", "ground_factor": "projection" })
    );

    // Exports carry the coordinate system.
    let dxf = dir.join("plan.dxf");
    let text = ok(&mut s, "export_file", json!({ "format": "dxf", "path": dxf }));
    assert!(text.contains("plan.prj"), "{text}");
    let prj = std::fs::read_to_string(dir.join("plan.prj")).expect("prj");
    assert!(
        prj.starts_with("PROJCS[\"MGA2020 zone 55") && prj.contains("AUTHORITY[\"EPSG\",\"7855\"]"),
        "{prj}"
    );
    let dxf = std::fs::read_to_string(&dxf).expect("dxf");
    assert!(
        dxf.starts_with("999\nsurvey3d coordinate system: MGA2020 zone 55"),
        "{}",
        &dxf[..200]
    );
    assert!(survey_io::dxf::read_dxf(dxf.as_bytes()).is_ok());

    // Errors say what to do.
    let (text, error) = call(&mut s, "setout", json!({ "station": "nope", "points": ["T"] }));
    assert!(error && text.contains("instrument station"), "{text}");
    let (text, error) = call(
        &mut s,
        "convert_heights",
        json!({ "points": ["T"], "to": "sideways" }),
    );
    assert!(error && text.contains("orthometric"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn alignments_with_spirals_curves_and_buildings() {
    let mut s = Server::new_for_tests();
    // East 300 m, a 200 m radius turning left 60°, then 300 m on.
    let (r, delta) = (200.0f64, 60f64.to_radians());
    let centre = (300.0, r);
    let ct = (
        centre.0 + r * (delta - std::f64::consts::FRAC_PI_2).cos(),
        centre.1 + r * (delta - std::f64::consts::FRAC_PI_2).sin(),
    );
    let end = (ct.0 + 300.0 * delta.cos(), ct.1 + 300.0 * delta.sin());
    let line = ok_json(
        &mut s,
        "add_polyline",
        json!({ "name": "MC01", "vertices": [[0, 0], [300, 0], [ct.0, ct.1], [end.0, end.1]], "bulges": [0, (delta / 4.0).tan(), 0] }),
    );
    let id = line["added"][0]["id"].as_i64().expect("id");
    ok(
        &mut s,
        "select",
        json!({ "kind": "polylines", "filter": { "ids": [id] } }),
    );
    ok(&mut s, "run_commands", json!({ "lines": ["mkalign"] }));
    let rec = ok_json(&mut s, "query", json!({ "kind": "alignment" }));
    let aid = rec["rows"][0]["id"].as_i64().expect("alignment id");
    ok(
        &mut s,
        "set_record",
        json!({ "kind": "alignment", "id": aid, "record": { "start_chainage": 1000.0, "transitions": [{ "segment": 1, "entry": 50.0, "exit": 50.0 }] } }),
    );
    let el = ok_json(&mut s, "alignment", json!({}));
    let kinds: Vec<&str> = el["elements"]
        .as_array()
        .expect("elements")
        .iter()
        .map(|e| e["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(
        kinds,
        ["straight", "spiral_in", "arc", "spiral_out", "straight"],
        "{el}"
    );
    let names: Vec<&str> = el["key_points"]
        .as_array()
        .expect("keys")
        .iter()
        .map(|k| k["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, ["Start", "TS", "SC", "CS", "ST", "End"]);
    assert!(el["report"].as_str().expect("report").contains("Spiral"), "{el}");
    // The PI is R·tan 30° on from the plain curve's tangent point (300, 0), and
    // the TS is Ts = (R + p)·tan 30° + k back from it.
    let sp = survey_core::spiral::Spiral::new(200.0, 50.0);
    let tan30 = 30f64.to_radians().tan();
    let want = 300.0 + 200.0 * tan30 - ((200.0 + sp.p) * tan30 + sp.k);
    let ts_e = el["key_points"][1]["e"].as_f64().expect("e");
    assert!((ts_e - want).abs() < 1e-3, "{ts_e} vs {want}");

    // A point 5 m left on the entry spiral, and back again.
    let ch = el["elements"][1]["start_chainage"].as_f64().expect("ch") + 25.0;
    let at = ok_json(
        &mut s,
        "alignment",
        json!({ "action": "at", "stations": [[ch, -5.0]] }),
    );
    let p = &at["positions"][0];
    let back = ok_json(
        &mut s,
        "alignment",
        json!({ "action": "station", "points": [[p["e"], p["n"]]] }),
    );
    let st = &back["stations"][0];
    assert!(
        (st["chainage"].as_f64().expect("ch") - ch).abs() < 1e-3
            && (st["offset"].as_f64().expect("off") + 5.0).abs() < 1e-3,
        "{back}"
    );
    // Points made from chainages keep the centreline's heights (none here).
    let made = ok_json(
        &mut s,
        "alignment",
        json!({ "action": "at", "stations": [[ch, 0]], "create": true }),
    );
    assert_eq!(made["created"][0]["z"], Value::Null, "{made}");
    // A point before the start is reported as beyond it.
    let off = ok_json(
        &mut s,
        "alignment",
        json!({ "action": "station", "points": [[-10, 3]] }),
    );
    assert_eq!(off["stations"][0]["beyond"], "start", "{off}");
    assert!(
        (off["stations"][0]["distance_beyond"].as_f64().expect("gap") - 10.0).abs() < 1e-3,
        "{off}"
    );
    // Key points are named in the set-out.
    let list = ok_json(&mut s, "alignment", json!({ "action": "setout", "interval": 0 }));
    let keys: Vec<&str> = list["points"]
        .as_array()
        .expect("points")
        .iter()
        .filter_map(|p| p["key"].as_str())
        .collect();
    assert_eq!(keys, ["Start", "TS", "SC", "CS", "ST", "End"], "{list}");
    // Set-out every 50 m on the centreline, and the centreline drawn with its spirals.
    let so = ok_json(
        &mut s,
        "alignment",
        json!({ "action": "setout", "interval": 50, "create": true }),
    );
    assert!(so["created"].as_array().expect("created").len() > 10, "{so}");
    let cl = ok_json(&mut s, "alignment", json!({ "action": "draw_centreline" }));
    assert_eq!(cl["created"][0]["name"], "MC01 CL");
    assert!(cl["created"][0].get("vertices").is_none(), "only a summary: {cl}");
    // A transition that does not fit is explained.
    ok(
        &mut s,
        "set_record",
        json!({ "kind": "alignment", "id": aid, "record": { "transitions": [{ "segment": 1, "entry": 300.0, "exit": 300.0 }] } }),
    );
    let (text, error) = call(&mut s, "alignment", json!({}));
    assert!(
        error && text.contains("Transition on segment 1 (the polyline's segment 2"),
        "{text}"
    );

    // Curve types.
    let c = ok_json(
        &mut s,
        "solve_curve",
        json!({ "type": "spiral", "radius": 300, "spiral_length": 60, "delta": 40 }),
    );
    assert!(
        (c["theta_s"]["degrees"].as_f64().expect("θs") - 0.1f64.to_degrees()).abs() < 1e-6,
        "{c}"
    );
    let c = ok_json(
        &mut s,
        "solve_curve",
        json!({ "type": "compound", "r1": 200, "r2": 100, "delta1": 30, "delta2": 20 }),
    );
    assert!(
        (c["solutions"][0]["delta"]["degrees"].as_f64().expect("Δ") - 50.0).abs() < 1e-9,
        "{c}"
    );
    let c = ok_json(
        &mut s,
        "solve_curve",
        json!({ "type": "reverse", "radius": 100, "offset": 10 }),
    );
    assert!((c["offset"].as_f64().expect("p") - 10.0).abs() < 1e-9, "{c}");
    let (text, error) = call(
        &mut s,
        "solve_curve",
        json!({ "type": "compound", "r1": 200, "t1": 30, "t2": 40 }),
    );
    assert!(error, "{text}");
    let (text, error) = call(
        &mut s,
        "solve_curve",
        json!({ "type": "spiral", "radius": 100, "spiral_length": 80, "delta": 30 }),
    );
    assert!(error && text.contains("The spirals turn 45.8366°"), "{text}");

    // A building from taped walls.
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["bldg", "500,500", "512,500", "8", "12", "8", "c"] }),
    );
    assert!(
        out.contains("Building closed: 4 corners, misclose 0.000") && out.contains("area 96.000"),
        "{out}"
    );
    // An outline that stops short is closed with a warning; one with no area is refused.
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["bldg", "600,500", "610,500", "5", "c"] }),
    );
    assert!(out.contains("a closing side was added"), "{out}");
    let out = ok(
        &mut s,
        "run_commands",
        json!({ "lines": ["bldg", "700,500", "710,500", "s 3", "c", ""] }),
    );
    assert!(out.contains("enclose no area"), "{out}");
}

#[test]
fn images_add_place_and_plot() {
    let mut s = Server::new_for_tests();
    let dir = temp_dir("images");
    let png = |name: &str, colour: [u8; 3]| {
        let path = dir.join(name);
        image::RgbaImage::from_pixel(600, 400, image::Rgba([colour[0], colour[1], colour[2], 255]))
            .save(&path)
            .expect("png");
        path
    };
    let placed = png("ortho.png", [20, 150, 40]);
    std::fs::write(dir.join("ortho.pgw"), "0.25\n0\n0\n-0.25\n5000.125\n7099.875\n").expect("pgw");
    let loose = png("scan.png", [200, 60, 30]);
    ok(
        &mut s,
        "add_points",
        json!({ "points": [
        { "name": "P1", "e": 5010.0, "n": 7090.0 },
        { "name": "P2", "e": 5300.0, "n": 7000.0 },
    ] }),
    );

    let a = ok_json(&mut s, "image", json!({ "action": "add", "path": placed }));
    assert_eq!(a["placement"], "file", "{a}");
    assert_eq!(a["status"], "ready");
    assert_eq!(a["corners"][0], json!([5000.0, 7100.0]));
    assert_eq!(a["pixel_size"], 0.25);
    let b = ok_json(&mut s, "image", json!({ "action": "add", "path": loose }));
    assert_eq!(b["placement"], "not_placed", "{b}");

    // Place the scan by two corners, given where they are on it now and where they belong.
    let corner = |c: &Value, i: usize| json!([c["corners"][i][0], c["corners"][i][1]]);
    let fit = ok_json(
        &mut s,
        "image",
        json!({ "action": "control_points", "image": "scan", "add": [
            { "at": corner(&b, 0), "world": [5200.0, 7100.0] },
            { "pixel": [600, 400], "world": "P2" },
        ] }),
    );
    assert_eq!(fit["placement"], "control_points", "{fit}");
    assert_eq!(fit["rms"], 0.0);
    // A 3:2 image's diagonal onto a square's turns it: atan(2/3) − 45° = 11°18'36" clockwise.
    assert_eq!(fit["rotation"], "11°18'36\" clockwise", "{fit}");
    // A third, wrong by 0.4 m, shows in its residual; disabled, the fit is exact again.
    let fit = ok_json(
        &mut s,
        "image",
        json!({ "action": "control_points", "image": "scan", "add": [{ "pixel": [600, 0], "world": [5300.4, 7100.0] }] }),
    );
    let r3 = fit["control_points"][2]["residual"].as_f64().expect("residual");
    assert!(r3 > 0.2, "{fit}");
    let fit = ok_json(
        &mut s,
        "image",
        json!({ "action": "control_points", "image": "scan", "disable": [3] }),
    );
    assert_eq!(fit["rms"], 0.0, "{fit}");
    // Numbers in one call refer to the list as shown: enable 3, disable 4 and remove 3 together.
    let both = ok_json(
        &mut s,
        "image",
        json!({ "action": "control_points", "image": "scan", "add": [{ "pixel": [0, 400], "world": [5200.0, 7000.0] }] }),
    );
    assert_eq!(both["control_points"].as_array().expect("points").len(), 4);
    let both = ok_json(
        &mut s,
        "image",
        json!({ "action": "control_points", "image": "scan", "enable": [3], "disable": [4], "remove": [3] }),
    );
    let pts = both["control_points"].as_array().expect("points");
    assert_eq!(pts.len(), 3, "{both}");
    assert_eq!(pts[2]["enabled"], false, "the old point 4, disabled: {both}");
    // Unequal pixels: Helmert misfits and the summary suggests affine.
    let squash = png("squash.png", [90, 90, 90]);
    ok(&mut s, "image", json!({ "action": "add", "path": squash }));
    let h = ok_json(
        &mut s,
        "image",
        json!({ "action": "control_points", "image": "squash", "add": [
            { "pixel": [0, 0], "world": [6000.0, 8000.0] },
            { "pixel": [600, 0], "world": [6060.0, 8000.0] },
            { "pixel": [600, 400], "world": [6060.0, 7920.0] },
            { "pixel": [0, 400], "world": [6000.0, 7920.0] },
        ] }),
    );
    assert!(
        h["summary"]
            .as_str()
            .expect("summary")
            .contains("An affine fit gives RMS 0.000 m"),
        "{h}"
    );
    let a = ok_json(
        &mut s,
        "image",
        json!({ "action": "control_points", "image": "squash", "fit": "affine" }),
    );
    assert_eq!(a["rms"], 0.0, "{a}");
    let id = a["id"].as_i64().expect("id");
    ok(&mut s, "delete_record", json!({ "kind": "image", "id": id }));
    let (text, error) = call(
        &mut s,
        "image",
        json!({ "action": "control_points", "image": "scan", "add": [{ "pixel": [900, 0], "world": [0, 0] }] }),
    );
    assert!(error && text.contains("off the image"), "{text}");

    // Settings, world file, plots.
    let set = ok_json(
        &mut s,
        "image",
        json!({ "action": "set", "image": "ortho", "opacity": 0.6, "elevation": 12.5 }),
    );
    assert_eq!(
        (
            set["opacity"].as_f64().map(|v| (v * 10.0).round()),
            set["elevation"].as_f64()
        ),
        (Some(6.0), Some(12.5))
    );
    let wf = ok(
        &mut s,
        "image",
        json!({ "action": "save_world_file", "image": "scan" }),
    );
    assert!(wf.contains("scan.pgw") && dir.join("scan.pgw").exists(), "{wf}");
    let svg = dir.join("plan.svg");
    ok(&mut s, "export_file", json!({ "format": "svg", "path": svg }));
    let text = std::fs::read_to_string(&svg).expect("svg");
    assert_eq!(text.matches("<image").count(), 2);
    let pdf = dir.join("plan.pdf");
    ok(&mut s, "export_file", json!({ "format": "pdf", "path": pdf }));
    let bytes = std::fs::read(&pdf).expect("pdf");
    let images = bytes.windows(15).filter(|w| *w == b"/Subtype /Image").count();
    assert_eq!(images, 4, "two images, each with its soft mask");
    let (text, error) = call(&mut s, "render", json!({ "width": 400 }));
    assert!(!error, "{text}");
    let info = ok_json(&mut s, "project_info", json!({}));
    assert_eq!(info["records"]["image"].as_array().expect("images").len(), 2);
    let id = info["records"]["image"][1]["id"].as_i64().expect("id");
    ok(&mut s, "delete_record", json!({ "kind": "image", "id": id }));
    let (text, error) = call(
        &mut s,
        "image",
        json!({ "action": "add", "path": dir.join("nope.ecw") }),
    );
    assert!(error && text.contains("ECW"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
