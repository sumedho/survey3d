use super::*;
use survey_core::Conventions;
use survey_model::{ChangeSet, Command, CoordinateSystem, History};

fn point(name: &str, x: f64, z: Option<f64>) -> Entity {
    Entity::Point(SurveyPoint {
        name: name.into(),
        position: DVec3::new(500_000.0 + x, 7_000_000.123_456_789, z.unwrap_or(0.0)),
        height_known: z.is_some(),
        code: "CP".into(),
        description: "found".into(),
        layer: LayerId(1),
    })
}

fn sample() -> (Document, History) {
    let mut doc = Document::new();
    let mut history = History::new("tester");
    let mut meta = doc.meta().clone();
    meta.name = "Test job".into();
    meta.conventions = Conventions::north_american();
    meta.feature_codes = survey_core::field_to_finish::FeatureLibrary::example();
    meta.tolerances.min_precision = Some(10_000.0);
    meta.traverses.push(survey_core::traverse::TraverseRecord {
        name: "Loop 1".into(),
        start: "1".into(),
        legs: vec![survey_core::traverse::Leg::new(
            "2",
            survey_core::traverse::Direction::Angle(survey_core::Angle::from_degrees(90.0)),
            25.0,
        )],
        ..Default::default()
    });
    meta.parcels.push(survey_model::ParcelRecord {
        name: "Lot 1".into(),
        plan: "DP 1234".into(),
        boundary: EntityId(3),
        ..Default::default()
    });
    meta.alignments.push(survey_model::AlignmentRecord {
        name: "MC01".into(),
        polyline: EntityId(3),
        start_chainage: 1000.0,
        ..Default::default()
    });
    meta.coord_system = CoordinateSystem::Projected {
        name: "MGA2020 zone 56".into(),
        epsg: Some(7856),
        zone: Some(56),
    };
    history.execute(Box::new(ChangeSet::new("settings").set_meta(meta)), &mut doc);
    let line = Entity::Polyline(Polyline {
        name: "fence".into(),
        vertices: vec![DVec3::new(1.0, 2.0, 3.0), DVec3::new(4.0, 5.0, 6.0)],
        bulges: vec![0.25, 0.0],
        closed: false,
        code: "FE".into(),
        layer: LayerId(1),
    });
    history.execute(
        Box::new(
            ChangeSet::new("add")
                .insert_entity(point("1", 0.0, Some(10.5)))
                .insert_entity(point("2", 1.0, None))
                .insert_entity(line),
        ),
        &mut doc,
    );
    (doc, history)
}

fn save(store: &mut ProjectStore, doc: &mut Document, history: &mut History) {
    let audit = history.take_pending_audit();
    store.save(doc, &audit, "test").expect("save");
}

fn assert_same(a: &Document, b: &Document) {
    assert_eq!(a.meta(), b.meta());
    assert_eq!(a.layers(), b.layers());
    let ea: Vec<_> = a.entities().collect();
    let eb: Vec<_> = b.entities().collect();
    assert_eq!(ea, eb);
}

#[test]
fn round_trip_through_bytes() {
    let (mut doc, mut history) = sample();
    let mut store = ProjectStore::create_in_memory().expect("store");
    save(&mut store, &mut doc, &mut history);
    assert!(!doc.is_dirty());

    let bytes = store.to_bytes().expect("bytes");
    let reopened = ProjectStore::from_bytes(&bytes).expect("open");
    let loaded = reopened.load_document().expect("load");
    assert_same(&doc, &loaded);
    // Exact f64 round trip at MGA magnitudes.
    let (_, p) = loaded.find_point_by_name("1").expect("point");
    assert_eq!(p.position.y, 7_000_000.123_456_789);
    assert!(!loaded.find_point_by_name("2").expect("p2").1.height_known);
}

#[test]
fn incremental_saves_apply_updates_and_deletes() {
    let (mut doc, mut history) = sample();
    let mut store = ProjectStore::create_in_memory().expect("store");
    save(&mut store, &mut doc, &mut history);

    let (id1, _) = doc.find_point_by_name("1").expect("p1");
    let (id2, _) = doc.find_point_by_name("2").expect("p2");
    history.execute(
        Box::new(
            ChangeSet::new("edit")
                .update_entity(id1, point("1", 50.0, Some(1.0)))
                .delete_entity(id2),
        ),
        &mut doc,
    );
    assert_eq!(doc.dirty().entities.len(), 2);
    save(&mut store, &mut doc, &mut history);

    let loaded = store.load_document().expect("load");
    assert_same(&doc, &loaded);
    assert!(loaded.find_point_by_name("2").is_none());
}

#[test]
fn audit_chain_verifies_and_detects_tampering() {
    let (mut doc, mut history) = sample();
    let mut store = ProjectStore::create_in_memory().expect("store");
    save(&mut store, &mut doc, &mut history);
    history.undo(&mut doc);
    save(&mut store, &mut doc, &mut history);

    assert_eq!(store.audit_count().expect("count"), 3);
    let v = store.verify_audit().expect("verify");
    assert!(v.is_valid(), "{v:?}");
    let latest = store.audit_entries(1).expect("entries");
    assert_eq!(latest[0].record.action, AuditAction::Undo);

    // The log is append-only from SQL…
    assert!(store.conn.execute("UPDATE audit_log SET user = 'x'", []).is_err());
    assert!(store.conn.execute("DELETE FROM audit_log", []).is_err());

    // …and tampering by removing the guard is detected by the hash chain.
    store
        .conn
        .execute_batch(
            "DROP TRIGGER audit_log_no_update; UPDATE audit_log SET user = 'mallory' WHERE id = 2;",
        )
        .expect("tamper");
    let v = store.verify_audit().expect("verify");
    assert_eq!(v.first_invalid, Some(2));
}

#[test]
fn rejects_non_project_data() {
    assert!(matches!(
        ProjectStore::from_bytes(b"definitely not sqlite"),
        Err(StoreError::NotAProject)
    ));
    let conn = Connection::open_in_memory().expect("conn");
    conn.execute_batch("CREATE TABLE t(x);").expect("t");
    let bytes = conn.serialize(rusqlite::MAIN_DB).expect("ser").to_vec();
    assert!(matches!(
        ProjectStore::from_bytes(&bytes),
        Err(StoreError::NotAProject)
    ));
}

#[test]
fn file_save_as_and_reopen() {
    let dir = std::env::temp_dir().join(format!("survey3d-store-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("job.s3d");

    let (mut doc, mut history) = sample();
    let mut store = ProjectStore::create_in_memory().expect("store");
    save(&mut store, &mut doc, &mut history);
    store.save_as_file(&path).expect("save as");
    assert_eq!(store.file_path(), Some(path.as_path()));

    // Further saves go to the file.
    history.execute(
        Box::new(ChangeSet::new("more").insert_entity(point("3", 3.0, None))),
        &mut doc,
    );
    save(&mut store, &mut doc, &mut history);
    drop(store);

    let reopened = ProjectStore::open_file(&path).expect("open");
    assert_same(&doc, &reopened.load_document().expect("load"));
    assert!(reopened.verify_audit().expect("verify").is_valid());
    drop(reopened);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_includes_unsaved_changes_without_saving() {
    let (mut doc, mut history) = sample();
    let mut store = ProjectStore::create_in_memory().expect("store");
    save(&mut store, &mut doc, &mut history);
    history.execute(
        Box::new(ChangeSet::new("more").insert_entity(point("9", 9.0, None))),
        &mut doc,
    );
    let bytes = store
        .snapshot_bytes(&doc, history.pending_audit(), "test")
        .expect("snapshot");
    assert!(doc.is_dirty(), "snapshot must not clear dirty state");
    assert!(
        store
            .load_document()
            .expect("load")
            .find_point_by_name("9")
            .is_none()
    );
    let recovered = ProjectStore::from_bytes(&bytes).expect("open");
    assert!(
        recovered
            .load_document()
            .expect("load")
            .find_point_by_name("9")
            .is_some()
    );
    assert!(recovered.verify_audit().expect("verify").is_valid());
}

#[test]
fn view_state_and_migration_from_v1() {
    let store = ProjectStore::create_in_memory().expect("store");
    assert_eq!(store.read_view_state().expect("read"), None);
    store.write_view_state("{\"zoom\":1}").expect("write");
    assert_eq!(
        store.read_view_state().expect("read").as_deref(),
        Some("{\"zoom\":1}")
    );

    // A v1 file (no bulge column) opens and is upgraded.
    let conn = Connection::open_in_memory().expect("conn");
    conn.execute_batch(schema::MIGRATIONS[0]).expect("v1");
    conn.pragma_update(None, "user_version", 1).expect("version");
    conn.execute_batch(
        "INSERT INTO strings (id, name, closed, code, layer_id) VALUES (1, 'a', 0, '', 1);
         INSERT INTO string_vertices (string_id, seq, e, n, z) VALUES (1, 0, 0, 0, 0), (1, 1, 1, 0, 0);",
    )
    .expect("data");
    let bytes = conn.serialize(rusqlite::MAIN_DB).expect("ser").to_vec();
    let upgraded = ProjectStore::from_bytes(&bytes).expect("migrates");
    let doc = upgraded.load_document().expect("load");
    match doc.entity(EntityId(1)) {
        Some(Entity::Polyline(l)) => assert!(l.bulges.is_empty() && l.vertices.len() == 2),
        other => panic!("{other:?}"),
    }
}

#[test]
fn surfaces_round_trip() {
    let (mut doc, mut history) = sample();
    let mut store = ProjectStore::create_in_memory().expect("store");
    let tin = Tin {
        vertices: vec![
            DVec3::new(512_000.123_456, 6_912_000.5, 10.0),
            DVec3::new(512_010.0, 6_912_000.0, 11.0),
            DVec3::new(512_000.0, 6_912_010.0, 12.0),
        ],
        triangles: vec![[0, 1, 2]],
    };
    let surface = Surface {
        name: "Existing".into(),
        definition: Some(survey_model::SurfaceDefinition {
            point_codes: vec!["NS*".into()],
            max_edge: Some(50.0),
            ..Default::default()
        }),
        tin,
        style: survey_model::SurfaceStyle::default(),
    };
    let mut cmd = ChangeSet::new("surface").insert_surface(surface.clone());
    cmd.apply(&mut doc);
    let id = cmd.inserted_surface_ids()[0];
    save(&mut store, &mut doc, &mut history);
    let loaded = ProjectStore::from_bytes(&store.to_bytes().expect("bytes"))
        .expect("open")
        .load_document()
        .expect("load");
    assert_eq!(loaded.surface(id), Some(&surface));

    let mut del = ChangeSet::new("delete").delete_surface(id);
    del.apply(&mut doc);
    save(&mut store, &mut doc, &mut history);
    assert!(store.load_document().expect("load").surface(id).is_none());
}

#[test]
fn point_clouds_round_trip() {
    let (mut doc, mut history) = sample();
    let mut store = ProjectStore::create_in_memory().expect("store");
    let pts = [
        DVec3::new(512_000.25, 6_912_000.5, 101.0),
        DVec3::new(512_010.75, 6_912_003.0, 99.5),
    ];
    let cloud = PointCloud::from_points("scan", &pts, Some(vec![[1, 2, 3], [4, 5, 6]]));
    let mut cmd = ChangeSet::new("cloud").insert_cloud(cloud.clone());
    cmd.apply(&mut doc);
    save(&mut store, &mut doc, &mut history);
    let loaded = ProjectStore::from_bytes(&store.to_bytes().expect("bytes"))
        .expect("open")
        .load_document()
        .expect("load");
    let (_, c) = loaded.clouds().next().expect("cloud");
    assert_eq!(c, &cloud);
    assert!((c.point(1) - pts[1]).length() < 1e-3);
}
