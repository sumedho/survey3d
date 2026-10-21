//! Database schema and migrations.
//!
//! The schema version is stored in `PRAGMA user_version`. To change the schema,
//! append a migration to [`MIGRATIONS`]; never edit an existing one.

use rusqlite::Connection;

use crate::{Result, StoreError};

/// Each entry upgrades the schema from version `index` to `index + 1`.
const MIGRATIONS: &[&str] = &[
    // v1: initial schema.
    r#"
    CREATE TABLE project_meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE coord_system (
        id              INTEGER PRIMARY KEY CHECK (id = 1),
        definition_json TEXT NOT NULL
    );
    CREATE TABLE conventions (
        id           INTEGER PRIMARY KEY CHECK (id = 1),
        profile_json TEXT NOT NULL
    );
    CREATE TABLE layers (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL,
        color      INTEGER NOT NULL,
        visible    INTEGER NOT NULL,
        locked     INTEGER NOT NULL,
        sort_order INTEGER NOT NULL
    );
    CREATE TABLE points (
        id          INTEGER PRIMARY KEY,
        name        TEXT NOT NULL,
        e           REAL NOT NULL,
        n           REAL NOT NULL,
        z           REAL,
        code        TEXT NOT NULL DEFAULT '',
        description TEXT NOT NULL DEFAULT '',
        layer_id    INTEGER NOT NULL,
        attrs_json  TEXT
    );
    CREATE INDEX points_name ON points(name);
    CREATE INDEX points_code ON points(code);
    CREATE TABLE strings (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL,
        closed     INTEGER NOT NULL,
        code       TEXT NOT NULL DEFAULT '',
        layer_id   INTEGER NOT NULL,
        attrs_json TEXT
    );
    CREATE TABLE string_vertices (
        string_id INTEGER NOT NULL,
        seq       INTEGER NOT NULL,
        e         REAL NOT NULL,
        n         REAL NOT NULL,
        z         REAL NOT NULL,
        PRIMARY KEY (string_id, seq)
    ) WITHOUT ROWID;
    CREATE TABLE audit_log (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        ts_utc_ms   INTEGER NOT NULL,
        user        TEXT NOT NULL,
        app_version TEXT NOT NULL,
        action      TEXT NOT NULL,
        label       TEXT NOT NULL,
        detail_json TEXT NOT NULL,
        prev_hash   TEXT NOT NULL,
        hash        TEXT NOT NULL
    );
    CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
        BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
    CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
        BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
    INSERT INTO project_meta (key, value) VALUES ('format', 'survey3d');
    "#,
];

pub const SCHEMA_VERSION: i32 = MIGRATIONS.len() as i32;

fn user_version(conn: &Connection) -> Result<i32> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

fn migrate_from(conn: &Connection, from: i32) -> Result<()> {
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(from as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// Creates the schema in an empty database.
pub fn initialise(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate_from(conn, 0)
}

/// Validates that the database is a project file and upgrades it if needed.
pub fn check_and_migrate(conn: &Connection) -> Result<()> {
    let version = user_version(conn).map_err(|_| StoreError::NotAProject)?;
    if version == 0 {
        return Err(StoreError::NotAProject);
    }
    if version > SCHEMA_VERSION {
        return Err(StoreError::TooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    let format: Option<String> = conn
        .query_row("SELECT value FROM project_meta WHERE key = 'format'", [], |r| {
            r.get(0)
        })
        .ok();
    if format.as_deref() != Some("survey3d") {
        return Err(StoreError::NotAProject);
    }
    migrate_from(conn, version)
}
