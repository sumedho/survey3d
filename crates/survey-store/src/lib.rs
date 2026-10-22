//! SQLite project storage (`.s3d` files).
//!
//! The in-memory [`Document`] is the source of truth while editing; this crate
//! persists it. Saves are incremental (only records the document reports as dirty)
//! and happen in a single transaction together with the new audit-trail entries.
//!
//! The audit trail is append-only (enforced by triggers) and hash-chained:
//! each entry stores `sha256(prev_hash ‖ entry)`, so editing the file outside the
//! application is detectable with [`ProjectStore::verify_audit`].

pub mod geopackage;
mod schema;

use std::path::{Path, PathBuf};

use glam::DVec3;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use survey_core::tin::Tin;
use survey_model::{
    AuditAction, AuditRecord, Color, Document, Entity, EntityId, Layer, LayerId, PointCloud, PointCloudId,
    Polyline, ProjectMeta, RecordKind, Records, Surface, SurfaceId, SurveyPoint,
};
use thiserror::Error;

pub use schema::SCHEMA_VERSION;

/// File extension for project files.
pub const FILE_EXTENSION: &str = "s3d";

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid project data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a survey3d project file")]
    NotAProject,
    #[error("project was written by a newer version (schema {found}, supported {supported})")]
    TooNew { found: i32, supported: i32 },
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// An audit entry as stored, including its position in the hash chain.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredAuditEntry {
    pub id: i64,
    pub record: AuditRecord,
    pub app_version: String,
    pub prev_hash: String,
    pub hash: String,
}

/// Result of checking the audit hash chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditVerification {
    pub entries: usize,
    /// Id of the first entry whose hash or link does not match, if any.
    pub first_invalid: Option<i64>,
}

impl AuditVerification {
    pub fn is_valid(&self) -> bool {
        self.first_invalid.is_none()
    }
}

/// An open project database.
pub struct ProjectStore {
    conn: Connection,
    path: Option<PathBuf>,
}

impl ProjectStore {
    /// A new, empty project held in memory (saved later with `save_as_file` or `to_bytes`).
    pub fn create_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::initialise(&conn)?;
        Ok(Self { conn, path: None })
    }

    /// Loads a project from serialized bytes (e.g. an uploaded file on the web).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        conn.deserialize_read_exact(rusqlite::MAIN_DB, bytes, bytes.len(), false)
            .map_err(|_| StoreError::NotAProject)?;
        schema::check_and_migrate(&conn)?;
        Ok(Self { conn, path: None })
    }

    /// Serializes the whole database (e.g. to download on the web).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.conn.serialize(rusqlite::MAIN_DB)?.to_vec())
    }

    /// Opens an existing project file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        schema::check_and_migrate(&conn)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Ok(Self {
            conn,
            path: Some(path.to_path_buf()),
        })
    }

    /// Copies the database to `path` and continues working on that file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_as_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        self.conn.backup(rusqlite::MAIN_DB, path, None)?;
        *self = Self::open_file(path)?;
        Ok(())
    }

    /// The backing file, if any.
    pub fn file_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    // ------------------------------------------------------------------ load

    pub fn load_document(&self) -> Result<Document> {
        let meta = self.load_meta()?;
        let layers = self.load_layers()?;
        let mut entities = self.load_points()?;
        entities.extend(self.load_polylines()?);
        let mut doc = Document::from_parts(meta, layers, entities);
        doc.load_records(self.load_records()?);
        doc.load_surfaces(self.load_surfaces()?);
        doc.load_clouds(self.load_clouds()?);
        Ok(doc)
    }

    fn load_clouds(&self) -> Result<Vec<(PointCloudId, PointCloud)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, origin_json, style_json, points, colors FROM point_clouds ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Vec<u8>>(4)?,
                r.get::<_, Option<Vec<u8>>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, origin, style, points, colors) = row?;
            let points = points
                .as_chunks::<12>()
                .0
                .iter()
                .map(|c| {
                    let f = |i: usize| f32::from_le_bytes(c[i * 4..i * 4 + 4].try_into().unwrap_or_default());
                    [f(0), f(1), f(2)]
                })
                .collect();
            let colors = colors.map(|c| c.as_chunks::<3>().0.iter().map(|x| [x[0], x[1], x[2]]).collect());
            out.push((
                PointCloudId(id),
                PointCloud {
                    name,
                    origin: serde_json::from_str(&origin)?,
                    points,
                    colors,
                    style: serde_json::from_str(&style)?,
                },
            ));
        }
        Ok(out)
    }

    fn load_surfaces(&self) -> Result<Vec<(SurfaceId, Surface)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, definition_json, style_json, vertices, triangles FROM surfaces ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Vec<u8>>(4)?,
                r.get::<_, Vec<u8>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, definition, style, vertices, triangles) = row?;
            let tin = Tin {
                vertices: vertices
                    .as_chunks::<24>()
                    .0
                    .iter()
                    .map(|c| {
                        let f =
                            |i: usize| f64::from_le_bytes(c[i * 8..i * 8 + 8].try_into().unwrap_or_default());
                        DVec3::new(f(0), f(1), f(2))
                    })
                    .collect(),
                triangles: triangles
                    .as_chunks::<12>()
                    .0
                    .iter()
                    .map(|c| {
                        let u =
                            |i: usize| u32::from_le_bytes(c[i * 4..i * 4 + 4].try_into().unwrap_or_default());
                        [u(0), u(1), u(2)]
                    })
                    .collect(),
            };
            out.push((
                SurfaceId(id),
                Surface {
                    name,
                    definition: definition.map(|d| serde_json::from_str(&d)).transpose()?,
                    tin,
                    style: serde_json::from_str(&style)?,
                },
            ));
        }
        Ok(out)
    }

    fn load_meta(&self) -> Result<ProjectMeta> {
        let mut meta = ProjectMeta::default();
        if let Some(name) = self.meta_value("name")? {
            meta.name = name;
        }
        if let Some(desc) = self.meta_value("description")? {
            meta.description = desc;
        }
        let cs: Option<String> = self
            .conn
            .query_row("SELECT definition_json FROM coord_system WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(cs) = cs {
            meta.coord_system = serde_json::from_str(&cs)?;
        }
        let conv: Option<String> = self
            .conn
            .query_row("SELECT profile_json FROM conventions WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(conv) = conv {
            meta.conventions = serde_json::from_str(&conv)?;
        }
        if let Some(codes) = self.meta_value("feature_codes")? {
            meta.feature_codes = serde_json::from_str(&codes)?;
        }
        if let Some(t) = self.meta_value("tolerances")? {
            meta.tolerances = serde_json::from_str(&t)?;
        }
        if let Some(o) = self.meta_value("network_options")? {
            meta.network_options = serde_json::from_str(&o)?;
        }
        Ok(meta)
    }

    /// Record collections, each a JSON array under its own key in `project_meta`.
    fn load_records(&self) -> Result<Records> {
        fn list<T: serde::de::DeserializeOwned>(store: &ProjectStore, kind: RecordKind) -> Result<Vec<T>> {
            Ok(match store.meta_value(kind.key())? {
                Some(json) => serde_json::from_str(&json)?,
                None => Vec::new(),
            })
        }
        Ok(Records {
            traverses: list(self, RecordKind::Traverse)?,
            parcels: list(self, RecordKind::Parcel)?,
            alignments: list(self, RecordKind::Alignment)?,
            field_data: list(self, RecordKind::FieldData)?,
            sheets: list(self, RecordKind::Sheet)?,
        })
    }

    fn meta_value(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM project_meta WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    fn load_layers(&self) -> Result<Vec<Layer>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, color, visible, locked FROM layers ORDER BY sort_order, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Layer {
                id: LayerId(r.get(0)?),
                name: r.get(1)?,
                color: Color::from_u32(r.get::<_, i64>(2)? as u32),
                visible: r.get(3)?,
                locked: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn load_points(&self) -> Result<Vec<(EntityId, Entity)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, e, n, z, code, description, layer_id FROM points ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            let z: Option<f64> = r.get(4)?;
            Ok((
                EntityId(r.get(0)?),
                Entity::Point(SurveyPoint {
                    name: r.get(1)?,
                    position: DVec3::new(r.get(2)?, r.get(3)?, z.unwrap_or(0.0)),
                    height_known: z.is_some(),
                    code: r.get(5)?,
                    description: r.get(6)?,
                    layer: LayerId(r.get(7)?),
                }),
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn load_polylines(&self) -> Result<Vec<(EntityId, Entity)>> {
        let mut vstmt = self
            .conn
            .prepare("SELECT string_id, e, n, z, bulge FROM string_vertices ORDER BY string_id, seq")?;
        type Vertices = (Vec<DVec3>, Vec<f64>);
        let mut vertices: std::collections::HashMap<i64, Vertices> = Default::default();
        let mut rows = vstmt.query([])?;
        while let Some(r) = rows.next()? {
            let entry = vertices.entry(r.get(0)?).or_default();
            entry.0.push(DVec3::new(r.get(1)?, r.get(2)?, r.get(3)?));
            entry.1.push(r.get(4)?);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, closed, code, layer_id FROM strings ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            let id: i64 = r.get(0)?;
            Ok((
                EntityId(id),
                Entity::Polyline(Polyline {
                    name: r.get(1)?,
                    vertices: Vec::new(),
                    bulges: Vec::new(),
                    closed: r.get(2)?,
                    code: r.get(3)?,
                    layer: LayerId(r.get(4)?),
                }),
            ))
        })?;
        let mut out: Vec<(EntityId, Entity)> = rows.collect::<rusqlite::Result<_>>()?;
        for (id, e) in &mut out {
            if let Entity::Polyline(p) = e {
                let (verts, bulges) = vertices.remove(&id.0).unwrap_or_default();
                p.vertices = verts;
                // All-straight polylines keep an empty bulge list.
                p.bulges = if bulges.iter().any(|b| *b != 0.0) {
                    bulges
                } else {
                    Vec::new()
                };
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------ save

    /// Writes dirty records and appends `audit` in one transaction. On success the
    /// document's dirty set is cleared; on failure nothing is written.
    pub fn save(&mut self, doc: &mut Document, audit: &[AuditRecord], app_version: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        write_changes(&tx, doc, audit, app_version)?;
        tx.commit()?;
        doc.take_dirty();
        Ok(())
    }

    /// The project file as it *would* be after saving, without saving: unsaved
    /// changes are applied to a copy. Used for crash-recovery snapshots.
    pub fn snapshot_bytes(
        &self,
        doc: &Document,
        audit: &[AuditRecord],
        app_version: &str,
    ) -> Result<Vec<u8>> {
        let mut copy = Self::from_bytes(&self.to_bytes()?)?;
        let tx = copy.conn.transaction()?;
        write_changes(&tx, doc, audit, app_version)?;
        tx.commit()?;
        copy.to_bytes()
    }

    /// Stores view state (camera etc.) as opaque JSON. Display state, not audited.
    pub fn write_view_state(&self, json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO project_meta (key, value) VALUES ('view_state', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [json],
        )?;
        Ok(())
    }

    pub fn read_view_state(&self) -> Result<Option<String>> {
        self.meta_value("view_state")
    }

    // ------------------------------------------------------------------ audit

    /// Most recent audit entries first (up to `limit`).
    pub fn audit_entries(&self, limit: usize) -> Result<Vec<StoredAuditEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts_utc_ms, user, app_version, action, label, detail_json, prev_hash, hash
             FROM audit_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], read_audit_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn audit_count(&self) -> Result<usize> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get::<_, i64>(0))? as usize)
    }

    /// Recomputes every hash in order and checks the chain links.
    pub fn verify_audit(&self) -> Result<AuditVerification> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts_utc_ms, user, app_version, action, label, detail_json, prev_hash, hash
             FROM audit_log ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            ))
        })?;
        let mut expected_prev = GENESIS_HASH.to_string();
        let mut entries = 0;
        for row in rows {
            let (id, ts, user, version, action, label, detail, prev, hash) = row?;
            entries += 1;
            let recomputed = entry_hash(&prev, ts, &user, &version, &action, &label, &detail);
            if prev != expected_prev || recomputed != hash {
                return Ok(AuditVerification {
                    entries,
                    first_invalid: Some(id),
                });
            }
            expected_prev = hash;
        }
        Ok(AuditVerification {
            entries,
            first_invalid: None,
        })
    }
}

fn read_audit_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<StoredAuditEntry>> {
    let action: String = r.get(4)?;
    let detail: String = r.get(6)?;
    let entry = (|| -> Result<StoredAuditEntry> {
        Ok(StoredAuditEntry {
            id: r.get(0)?,
            record: AuditRecord {
                timestamp_ms: r.get(1)?,
                user: r.get(2)?,
                action: AuditAction::parse(&action).unwrap_or(AuditAction::Do),
                label: r.get(5)?,
                detail: serde_json::from_str(&detail)?,
            },
            app_version: r.get(3)?,
            prev_hash: r.get(7)?,
            hash: r.get(8)?,
        })
    })();
    Ok(entry)
}

fn entry_hash(
    prev: &str,
    ts: i64,
    user: &str,
    version: &str,
    action: &str,
    label: &str,
    detail: &str,
) -> String {
    let mut h = Sha256::new();
    for part in [prev, &ts.to_string(), user, version, action, label, detail] {
        // Length-prefix each field so concatenation is unambiguous.
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn write_changes(
    tx: &Transaction<'_>,
    doc: &Document,
    audit: &[AuditRecord],
    app_version: &str,
) -> Result<()> {
    let dirty = doc.dirty();
    if dirty.meta {
        write_meta(tx, doc.meta())?;
    }
    for kind in doc.records_to_save() {
        write_records(tx, doc.records(), kind)?;
    }
    if !dirty.layers.is_empty() {
        write_layers(tx, doc)?;
    }
    for id in &dirty.entities {
        write_entity(tx, *id, doc.entity(*id))?;
    }
    for id in &dirty.surfaces {
        write_surface(tx, *id, doc.surface(*id))?;
    }
    for id in &dirty.clouds {
        write_cloud(tx, *id, doc.cloud(*id))?;
    }
    append_audit(tx, audit, app_version)
}

fn write_cloud(tx: &Transaction<'_>, id: PointCloudId, cloud: Option<&PointCloud>) -> Result<()> {
    tx.execute("DELETE FROM point_clouds WHERE id = ?1", [id.0])?;
    let Some(c) = cloud else {
        return Ok(());
    };
    let mut points = Vec::with_capacity(c.points.len() * 12);
    for p in &c.points {
        for v in p {
            points.extend_from_slice(&v.to_le_bytes());
        }
    }
    let colors: Option<Vec<u8>> = c.colors.as_ref().map(|cs| cs.iter().flatten().copied().collect());
    tx.execute(
        "INSERT INTO point_clouds (id, name, origin_json, style_json, points, colors) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id.0,
            c.name,
            serde_json::to_string(&c.origin)?,
            serde_json::to_string(&c.style)?,
            points,
            colors
        ],
    )?;
    Ok(())
}

fn write_surface(tx: &Transaction<'_>, id: SurfaceId, surface: Option<&Surface>) -> Result<()> {
    tx.execute("DELETE FROM surfaces WHERE id = ?1", [id.0])?;
    let Some(s) = surface else {
        return Ok(());
    };
    let mut vertices = Vec::with_capacity(s.tin.vertices.len() * 24);
    for v in &s.tin.vertices {
        for c in [v.x, v.y, v.z] {
            vertices.extend_from_slice(&c.to_le_bytes());
        }
    }
    let mut triangles = Vec::with_capacity(s.tin.triangles.len() * 12);
    for t in &s.tin.triangles {
        for k in t {
            triangles.extend_from_slice(&k.to_le_bytes());
        }
    }
    let definition = s.definition.as_ref().map(serde_json::to_string).transpose()?;
    tx.execute(
        "INSERT INTO surfaces (id, name, definition_json, style_json, vertices, triangles) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id.0, s.name, definition, serde_json::to_string(&s.style)?, vertices, triangles],
    )?;
    Ok(())
}

fn write_records(tx: &Transaction<'_>, records: &Records, kind: RecordKind) -> Result<()> {
    let json = match kind {
        RecordKind::Traverse => serde_json::to_string(&records.traverses),
        RecordKind::Parcel => serde_json::to_string(&records.parcels),
        RecordKind::Alignment => serde_json::to_string(&records.alignments),
        RecordKind::FieldData => serde_json::to_string(&records.field_data),
        RecordKind::Sheet => serde_json::to_string(&records.sheets),
    }?;
    tx.execute(
        "INSERT INTO project_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![kind.key(), json],
    )?;
    Ok(())
}

fn write_meta(tx: &Transaction<'_>, meta: &ProjectMeta) -> Result<()> {
    let mut put = tx.prepare_cached(
        "INSERT INTO project_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )?;
    put.execute(params!["name", meta.name])?;
    put.execute(params!["description", meta.description])?;
    put.execute(params![
        "feature_codes",
        serde_json::to_string(&meta.feature_codes)?
    ])?;
    put.execute(params!["tolerances", serde_json::to_string(&meta.tolerances)?])?;
    put.execute(params![
        "network_options",
        serde_json::to_string(&meta.network_options)?
    ])?;
    tx.execute(
        "INSERT OR REPLACE INTO coord_system (id, definition_json) VALUES (1, ?1)",
        [serde_json::to_string(&meta.coord_system)?],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO conventions (id, profile_json) VALUES (1, ?1)",
        [serde_json::to_string(&meta.conventions)?],
    )?;
    Ok(())
}

/// Layers are few, so they are rewritten as a whole (which also captures ordering).
fn write_layers(tx: &Transaction<'_>, doc: &Document) -> Result<()> {
    tx.execute("DELETE FROM layers", [])?;
    let mut ins = tx.prepare_cached(
        "INSERT INTO layers (id, name, color, visible, locked, sort_order) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (i, l) in doc.layers().iter().enumerate() {
        ins.execute(params![
            l.id.0,
            l.name,
            i64::from(l.color.to_u32()),
            l.visible,
            l.locked,
            i as i64
        ])?;
    }
    Ok(())
}

fn write_entity(tx: &Transaction<'_>, id: EntityId, entity: Option<&Entity>) -> Result<()> {
    // An id can change type between saves, so clear it from every table first.
    tx.prepare_cached("DELETE FROM points WHERE id = ?1")?
        .execute([id.0])?;
    tx.prepare_cached("DELETE FROM string_vertices WHERE string_id = ?1")?
        .execute([id.0])?;
    tx.prepare_cached("DELETE FROM strings WHERE id = ?1")?
        .execute([id.0])?;
    match entity {
        None => {}
        Some(Entity::Point(p)) => {
            tx.prepare_cached(
                "INSERT INTO points (id, name, e, n, z, code, description, layer_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![
                id.0,
                p.name,
                p.position.x,
                p.position.y,
                p.height_known.then_some(p.position.z),
                p.code,
                p.description,
                p.layer.0
            ])?;
        }
        Some(Entity::Polyline(l)) => {
            tx.prepare_cached(
                "INSERT INTO strings (id, name, closed, code, layer_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![id.0, l.name, l.closed, l.code, l.layer.0])?;
            let mut ins = tx.prepare_cached(
                "INSERT INTO string_vertices (string_id, seq, e, n, z, bulge) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (seq, v) in l.vertices.iter().enumerate() {
                ins.execute(params![id.0, seq as i64, v.x, v.y, v.z, l.bulge(seq)])?;
            }
        }
    }
    Ok(())
}

fn append_audit(tx: &Transaction<'_>, records: &[AuditRecord], app_version: &str) -> Result<()> {
    let mut prev: String = tx
        .query_row("SELECT hash FROM audit_log ORDER BY id DESC LIMIT 1", [], |r| {
            r.get(0)
        })
        .optional()?
        .unwrap_or_else(|| GENESIS_HASH.to_string());
    let mut ins = tx.prepare_cached(
        "INSERT INTO audit_log (ts_utc_ms, user, app_version, action, label, detail_json, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for r in records {
        let detail = serde_json::to_string(&r.detail)?;
        let action = r.action.as_str();
        let hash = entry_hash(
            &prev,
            r.timestamp_ms,
            &r.user,
            app_version,
            action,
            &r.label,
            &detail,
        );
        ins.execute(params![
            r.timestamp_ms,
            r.user,
            app_version,
            action,
            r.label,
            detail,
            prev,
            hash
        ])?;
        prev = hash;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
