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
