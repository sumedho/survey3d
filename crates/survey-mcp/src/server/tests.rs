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
