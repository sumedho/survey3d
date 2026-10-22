//! GeoPackage (OGC 1.4) export: points, linework and parcels as feature
//! tables with 3D geometry, in the project's coordinate system.

use glam::DVec3;
use rusqlite::{Connection, params};
use survey_core::projection::TransverseMercator;
use survey_model::{CoordinateSystem, Document, Entity};

use crate::Result;

/// Plan chord tolerance for arcs (metres).
const ARC_TOLERANCE: f64 = 0.005;

/// WKT (OGC 1) for a Transverse Mercator projection.
fn tm_wkt(name: &str, tm: &TransverseMercator) -> String {
    let e = tm.ellipsoid;
    format!(
        "PROJCS[\"{name}\",GEOGCS[\"{0}\",DATUM[\"{0}\",SPHEROID[\"{0}\",{1},{2}]],PRIMEM[\"Greenwich\",0],\
         UNIT[\"degree\",0.0174532925199433]],PROJECTION[\"Transverse_Mercator\"],\
         PARAMETER[\"latitude_of_origin\",{3}],PARAMETER[\"central_meridian\",{4}],\
         PARAMETER[\"scale_factor\",{5}],PARAMETER[\"false_easting\",{6}],PARAMETER[\"false_northing\",{7}],\
         UNIT[\"metre\",1],AXIS[\"Easting\",EAST],AXIS[\"Northing\",NORTH]]",
        e.name,
        e.a,
        e.inv_f,
        tm.latitude_of_origin,
        tm.central_meridian,
        tm.scale,
        tm.false_easting,
        tm.false_northing
    )
}

/// The spatial reference row for the project: (srs_id, name, organisation, code, definition).
fn project_srs(cs: &CoordinateSystem) -> (i32, String, String, i32, String) {
    match cs {
        CoordinateSystem::Projected { epsg, projection, .. } => {
            let name = cs.display_name();
            let definition = projection
                .as_ref()
                .map_or_else(|| "undefined".to_string(), |tm| tm_wkt(&name, tm));
            match epsg {
                Some(code) => (*code as i32, name, "EPSG".into(), *code as i32, definition),
                None => (100_000, name, "NONE".into(), 100_000, definition),
            }
        }
        _ => (
            -1,
            "Undefined Cartesian SRS".into(),
            "NONE".into(),
            -1,
            "undefined".into(),
        ),
    }
}

/// GeoPackage geometry blob: header with an XY envelope, then little-endian WKB.
fn blob(srs: i32, wkb_type: u32, rings: &[Vec<DVec3>]) -> Vec<u8> {
    let all = rings.iter().flatten();
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for p in all {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let mut b = Vec::with_capacity(48 + rings.iter().map(Vec::len).sum::<usize>() * 24);
    b.extend_from_slice(b"GP");
    b.push(0); // version 1
    b.push(0b0000_0011); // XY envelope, little endian
    b.extend_from_slice(&srs.to_le_bytes());
    for v in [lo.x, hi.x, lo.y, hi.y] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.push(1);
    b.extend_from_slice(&wkb_type.to_le_bytes());
    let coords = |b: &mut Vec<u8>, pts: &[DVec3]| {
        for p in pts {
            for v in [p.x, p.y, p.z] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
    };
    match wkb_type {
        1001 => coords(&mut b, &rings[0][..1]),
        1002 => {
            b.extend_from_slice(&(rings[0].len() as u32).to_le_bytes());
            coords(&mut b, &rings[0]);
        }
        _ => {
            b.extend_from_slice(&(rings.len() as u32).to_le_bytes());
            for r in rings {
                b.extend_from_slice(&(r.len() as u32).to_le_bytes());
                coords(&mut b, r);
            }
        }
    }
    b
}

/// What was written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpkgSummary {
    pub points: usize,
    pub lines: usize,
    pub parcels: usize,
}

/// Writes the document's points, polylines and parcels to a new GeoPackage.
pub fn export_geopackage(doc: &Document) -> Result<(Vec<u8>, GpkgSummary)> {
    let mut conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "PRAGMA application_id = 1196444487;
         PRAGMA user_version = 10400;
         CREATE TABLE gpkg_spatial_ref_sys (
           srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL,
           organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
         CREATE TABLE gpkg_contents (
           table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE,
           description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
           min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER,
           CONSTRAINT fk_gc_r_srs_id FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id));
         CREATE TABLE gpkg_geometry_columns (
           table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL,
           srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL,
           CONSTRAINT pk_geom_cols PRIMARY KEY (table_name, column_name),
           CONSTRAINT fk_gc_tn FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name),
           CONSTRAINT fk_gc_srs FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys (srs_id));
         INSERT INTO gpkg_spatial_ref_sys VALUES
           ('Undefined cartesian SRS', -1, 'NONE', -1, 'undefined', 'undefined cartesian coordinate reference system'),
           ('Undefined geographic SRS', 0, 'NONE', 0, 'undefined', 'undefined geographic coordinate reference system'),
           ('WGS 84 geodetic', 4326, 'EPSG', 4326,
            'GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]',
            'longitude/latitude coordinates in decimal degrees on the WGS 84 spheroid');",
    )?;
    let (srs, srs_name, org, code, definition) = project_srs(&doc.meta().coord_system);
    if srs != -1 && srs != 4326 {
        conn.execute(
            "INSERT OR REPLACE INTO gpkg_spatial_ref_sys VALUES (?1, ?2, ?3, ?4, ?5, 'survey3d project')",
            params![srs_name, srs, org, code, definition],
        )?;
    }
    let layer_name = |id| doc.layer(id).map(|l| l.name.clone()).unwrap_or_default();
    let tx = conn.transaction()?;
    let table = |tx: &rusqlite::Transaction<'_>,
                 name: &str,
                 geom: &str,
                 columns: &str,
                 description: &str|
     -> Result<()> {
        tx.execute_batch(&format!(
            "CREATE TABLE \"{name}\" (fid INTEGER PRIMARY KEY AUTOINCREMENT, geom {geom}, {columns});"
        ))?;
        tx.execute(
            "INSERT INTO gpkg_contents (table_name, data_type, identifier, description, srs_id) VALUES (?1, 'features', ?1, ?2, ?3)",
            params![name, description, srs],
        )?;
        tx.execute(
            "INSERT INTO gpkg_geometry_columns VALUES (?1, 'geom', ?2, ?3, 1, 0)",
            params![name, geom, srs],
        )?;
        Ok(())
    };
    let mut summary = GpkgSummary::default();
    let mut extent = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    let mut grow = |p: DVec3| {
        extent.0 = extent.0.min(p);
        extent.1 = extent.1.max(p);
    };

    table(
        &tx,
        "points",
        "POINT",
        "name TEXT, code TEXT, description TEXT, layer TEXT, height_known BOOLEAN",
        "Survey points",
    )?;
    table(
        &tx,
        "lines",
        "LINESTRING",
        "name TEXT, code TEXT, layer TEXT, closed BOOLEAN, length DOUBLE",
        "Linework",
    )?;
    {
        let mut pts = tx.prepare(
            "INSERT INTO points (geom, name, code, description, layer, height_known) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut lines = tx.prepare(
            "INSERT INTO lines (geom, name, code, layer, closed, length) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (_, e) in doc.entities() {
            match e {
                Entity::Point(p) => {
                    grow(p.position);
                    pts.execute(params![
                        blob(srs, 1001, &[vec![p.position]]),
                        p.name,
                        p.code,
                        p.description,
                        layer_name(p.layer),
                        p.height_known
                    ])?;
                    summary.points += 1;
                }
                Entity::Polyline(l) if l.vertices.len() >= 2 => {
                    let mut v: Vec<DVec3> = Vec::new();
                    for seg in l.tessellate(ARC_TOLERANCE) {
                        let skip = usize::from(!v.is_empty());
                        v.extend(seg.into_iter().skip(skip));
                    }
                    v.iter().for_each(|p| grow(*p));
                    let (length, _, _) = survey_core::arc::polyline_metrics(&l.vertices, &l.bulges, l.closed);
                    lines.execute(params![
                        blob(srs, 1002, &[v]),
                        l.name,
                        l.code,
                        layer_name(l.layer),
                        l.closed,
                        length
                    ])?;
                    summary.lines += 1;
                }
                Entity::Polyline(_) => {}
            }
        }
    }
    if !doc.meta().parcels.is_empty() {
        table(
            &tx,
            "parcels",
            "POLYGON",
            "name TEXT, plan TEXT, description TEXT, area DOUBLE",
            "Parcels",
        )?;
        let mut stmt = tx.prepare(
            "INSERT INTO parcels (geom, name, plan, description, area) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for p in &doc.meta().parcels {
            let Some(Entity::Polyline(l)) = doc.entity(p.boundary) else {
                continue;
            };
            let mut ring: Vec<DVec3> = Vec::new();
            for seg in l.tessellate(ARC_TOLERANCE) {
                let skip = usize::from(!ring.is_empty());
                ring.extend(seg.into_iter().skip(skip));
            }
            if ring.len() < 3 {
                continue;
            }
            let (_, _, signed) = survey_core::arc::polyline_metrics(&l.vertices, &l.bulges, true);
            // OGC exterior rings run anticlockwise.
            if signed < 0.0 {
                ring.reverse();
            }
            if ring.first().map(|p| p.truncate()) != ring.last().map(|p| p.truncate()) {
                ring.push(ring[0]);
            }
            stmt.execute(params![
                blob(srs, 1003, &[ring]),
                p.name,
                p.plan,
                p.description,
                signed.abs()
            ])?;
            summary.parcels += 1;
        }
    }
    if extent.0.x.is_finite() {
        tx.execute(
            "UPDATE gpkg_contents SET min_x = ?1, min_y = ?2, max_x = ?3, max_y = ?4",
            params![extent.0.x, extent.0.y, extent.1.x, extent.1.y],
        )?;
    }
    tx.commit()?;
    Ok((conn.serialize(rusqlite::MAIN_DB)?.to_vec(), summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use survey_core::projection::Ellipsoid;
    use survey_model::{ChangeSet, Command, Polyline, SurveyPoint};

    #[test]
    fn writes_a_valid_geopackage() {
        let mut doc = Document::new();
        let layer = doc.current_layer();
        let o = DVec3::new(512_000.0, 6_912_000.0, 10.0);
        let mut meta = doc.meta().clone();
        meta.coord_system = CoordinateSystem::Projected {
            name: "MGA2020 zone 56".into(),
            epsg: Some(7856),
            zone: Some(56),
            projection: Some(TransverseMercator::utm(56, true, Ellipsoid::GRS80)),
            geoid_separation: 0.0,
        };
        let square = Polyline {
            name: "Lot".into(),
            vertices: vec![
                o,
                o + DVec3::X * 20.0,
                o + DVec3::new(20.0, 30.0, 0.0),
                o + DVec3::Y * 30.0,
            ],
            bulges: Vec::new(),
            closed: true,
            code: "BDY".into(),
            layer,
        };
        let mut cs = ChangeSet::new("test")
            .insert_entity(Entity::Point(SurveyPoint {
                name: "1".into(),
                position: o,
                height_known: true,
                code: "CP".into(),
                description: String::new(),
                layer,
            }))
            .insert_entity(Entity::Polyline(square));
        cs.apply(&mut doc);
        let boundary = doc
            .entities()
            .find(|(_, e)| matches!(e, Entity::Polyline(_)))
            .map(|(id, _)| id)
            .expect("line");
        meta.parcels.push(survey_model::parcel::ParcelRecord {
            name: "Lot 1".into(),
            boundary,
            ..Default::default()
        });
        ChangeSet::new("meta").set_meta(meta).apply(&mut doc);

        let (bytes, summary) = export_geopackage(&doc).expect("gpkg");
        assert_eq!(
            summary,
            GpkgSummary {
                points: 1,
                lines: 1,
                parcels: 1
            }
        );
        let mut conn = Connection::open_in_memory().expect("db");
        conn.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), true)
            .expect("load");
        let app: i64 = conn
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .expect("id");
        assert_eq!(app, 0x4750_4B47);
        let srs: String = conn
            .query_row(
                "SELECT definition FROM gpkg_spatial_ref_sys WHERE srs_id = 7856",
                [],
                |r| r.get(0),
            )
            .expect("srs");
        assert!(srs.contains("PARAMETER[\"central_meridian\",153]"), "{srs}");
        let (area, geom): (f64, Vec<u8>) = conn
            .query_row("SELECT area, geom FROM parcels", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("parcel");
        assert!((area - 600.0).abs() < 1e-9);
        assert_eq!(&geom[0..2], b"GP");
        // Header 8 + envelope 32, then WKB: order, type 1003, 1 ring of 5 points.
        assert_eq!(u32::from_le_bytes(geom[41..45].try_into().expect("u32")), 1003);
        assert_eq!(u32::from_le_bytes(geom[49..53].try_into().expect("u32")), 5);
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM gpkg_geometry_columns WHERE srs_id = 7856 AND z = 1",
                [],
                |r| r.get(0),
            )
            .expect("cols");
        assert_eq!(n, 3);
    }
}
