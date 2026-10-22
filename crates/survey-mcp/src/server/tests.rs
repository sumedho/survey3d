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
