//! Reading files into the project and building exports. File choice and saving
//! are the caller's: these take and return bytes and text.

use std::fmt::Write as _;

use glam::DVec2;
use survey_core::network::{FieldData, RawEdit};
use survey_core::style::LabelSet;
use survey_io::plot::Plot;
use survey_io::stakeout::{StakeFormat, StakeLine};
use survey_io::{CsvLayout, CsvPointImporter, ImportedPoint};
use survey_model::{ChangeSet, Entity, EntityId, SheetRecord, Surface, SurfaceStyle, SurveyPoint};

use super::{Event, Panel, Session};
use crate::import_command::ImportEntities;
use crate::interchange::{DxfExportOptions, dxf_export, dxf_import};
use crate::settings::LabelSettings;
use crate::ui::sheets::{SheetsWindow, ViewInfo};

/// For a north-up sheet: the part of its map frame clear of the title block,
/// as (its centre's offset from the sheet centre, its size), in metres.
fn clear_of_title(sheet: &SheetRecord) -> (DVec2, DVec2) {
    let lay = crate::plan::layout(sheet);
    let (f, t) = (lay.frame, lay.title);
    // An inset title block sits in the frame's bottom; a strip is already outside it.
    let bottom = if lay.strip { f.min[1] } else { t.max[1] };
    let k = sheet.mm_per_metre();
    let offset = DVec2::new(0.0, (bottom - f.min[1]) * 0.5) / k;
    (offset, DVec2::new(f.max[0] - f.min[0], f.max[1] - bottom) / k)
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

impl Session {
    /// Adds points (one undo step) on the layer named `layer` (created if
    /// needed), else the current layer. Unnamed points are numbered on.
    pub fn add_points(&mut self, points: &[ImportedPoint], layer: Option<&str>, source: &str) -> usize {
        let label = format!("Import {} points from {source}", points.len());
        let cmd = ImportEntities::points(&self.project.doc, label, points, layer);
        let n = cmd.len();
        self.execute(Box::new(cmd));
        n
    }

    /// Points from delimited text laid out as `layout` (column letters such as `PENZC`).
    pub fn import_points_csv(&mut self, name: &str, bytes: &[u8], layout: CsvLayout, layer: Option<&str>) {
        let Ok(text) = std::str::from_utf8(bytes) else {
            self.log.error(format!("Could not read {name}: not UTF-8 text"));
            return;
        };
        match CsvPointImporter::new(layout).import_text(text.strip_prefix('\u{feff}').unwrap_or(text)) {
            Ok(import) => {
                let n = self.add_points(&import.points, layer, name);
                self.emit(Event::ZoomExtents);
                self.success(format!("Imported {n} points from {name}"));
                for w in import.warnings {
                    self.log.warn(w);
                }
            }
            Err(e) => self.log.error(format!("Could not read {name}: {e}")),
        }
    }

    /// Raw instrument data: kept with the project for reduction and the network
    /// adjustment; coordinates in the file add any points the project does not
    /// have (those the instrument computed for its shots belong to the file's
    /// reduction, which recomputes them). A file of the same name replaces the
    /// earlier import, keeping its corrections where the setups and shots
    /// still match.
    pub fn import_raw(&mut self, name: &str, bytes: &[u8]) {
        let stem = stem(name);
        let mut data = match survey_io::rawdata::read_raw(stem, bytes) {
            Ok(data) => data,
            Err(e) => {
                self.log.error(format!("Could not read {name}: {e}"));
                return;
            }
        };
        let doc = &self.project.doc;
        let mut cs = ChangeSet::new(format!("Import raw data {stem}"));
        let mut added = 0;
        let existing = doc.field_data().iter().find(|f| f.name == data.name);
        let mut kept_edits = 0;
        if let Some(old) = existing {
            data.reduced.clone_from(&old.reduced);
            data.scale_factor = data.scale_factor.or(old.scale_factor);
            let station = |f: &FieldData, i: usize| f.setups.get(i).map(|s| s.station.clone());
            let target = |f: &FieldData, i: usize, k: usize| {
                f.setups
                    .get(i)
                    .and_then(|s| s.observations.get(k))
                    .map(|o| o.target.clone())
            };
            for e in &old.edits {
                let same = station(old, e.setup).is_some_and(|s| Some(s) == station(&data, e.setup))
                    && e.observation.is_none_or(|k| {
                        target(old, e.setup, k).is_some_and(|t| Some(t) == target(&data, e.setup, k))
                    });
                if same {
                    data.edits.push(e.clone());
                    kept_edits += 1;
                }
            }
            if kept_edits < old.edits.len() {
                self.log.warn(format!(
                    "{} of {} corrections to {stem} no longer match its setups and shots and were dropped",
                    old.edits.len() - kept_edits,
                    old.edits.len()
                ));
            }
        }
        let computed: Vec<(String, glam::DVec3)> = data.computed.clone();
        for (pname, p, own) in data
            .points
            .iter()
            .map(|(n, p)| (n, p, false))
            .chain(computed.iter().map(|(n, p)| (n, p, true)))
        {
            if doc.find_point_by_name(pname).is_none() {
                if own && !data.reduced.contains(pname) {
                    data.reduced.push(pname.clone());
                }
                cs.push_insert_entity(Entity::Point(SurveyPoint {
                    name: pname.clone(),
                    position: *p,
                    height_known: p.z != 0.0,
                    code: String::new(),
                    description: format!("From {}", data.format),
                    layer: doc.current_layer(),
                }));
                added += 1;
            }
        }
        let pointings: usize = data.setups.iter().map(|s| s.observations.len()).sum();
        let summary = format!(
            "Imported {} {stem}: {} setups, {pointings} observations, {added} new points",
            data.format,
            data.setups.len()
        );
        let warnings = data.warnings.clone();
        let cs = match existing.map(|f| f.id) {
            Some(id) => cs.update_record(id, data),
            None => cs.insert_record(data),
        };
        self.execute(Box::new(cs));
        self.emit(Event::Open(Panel::RawData {
            name: stem.to_string(),
        }));
        if added > 0 {
            self.emit(Event::ZoomExtents);
        }
        self.success(summary);
        for w in warnings {
            self.log.warn(w);
        }
        if pointings > 0 {
            self.log.info(format!(
                "Type 'reduce {stem}' to turn the shots into coded points, or adjust the control first (netadj)"
            ));
        }
    }

    /// Reduces a raw data file's shots to coded points (the only file when
    /// `name` is empty), shows the reduction report, and returns the summary.
    pub fn reduce_raw(&mut self, name: &str) -> Result<String, String> {
        let r = crate::raw::reduce_file(&self.project.doc, name)?;
        for w in &r.reduction.warnings {
            self.log.warn(w.clone());
        }
        let summary = r.summary.clone();
        let added = r.command.len();
        self.execute(Box::new(r.command));
        self.log.result(summary.clone());
        if let Some((title, text)) = self.raw_report(name) {
            self.report(title, text);
        }
        if added > 0 {
            self.emit(Event::ZoomExtents);
        }
        Ok(summary)
    }

    /// Corrects a setup or shot (re-reducing a reduced file in the same step).
    pub fn edit_raw(&mut self, name: &str, edit: RawEdit, revert: bool) -> Result<String, String> {
        let r = crate::raw::edit_file(&self.project.doc, name, edit, revert)?;
        let summary = r.summary.clone();
        self.execute(Box::new(r.command));
        self.log.result(summary.clone());
        Ok(summary)
    }

    /// The reduction report for a raw data file, as it reduces now.
    pub fn raw_report(&self, name: &str) -> Option<(String, String)> {
        let doc = &self.project.doc;
        let f = crate::raw::find(doc, name).ok()?;
        let r = crate::raw::reduce_command(doc, f.id, f.record.clone(), String::new()).reduction;
        Some((format!("Reduction {}", f.name), crate::raw::report(doc, f, &r)))
    }

    pub fn import_cloud(&mut self, name: &str, bytes: &[u8]) {
        #[cfg(target_arch = "wasm32")]
        const MAX_CLOUD_POINTS: usize = 15_000_000;
        #[cfg(not(target_arch = "wasm32"))]
        const MAX_CLOUD_POINTS: usize = 60_000_000;
        let raw = match survey_io::pointcloud::read_cloud(bytes) {
            Ok(raw) => raw,
            Err(e) => {
                self.log.error(format!("Could not read {name}: {e}"));
                return;
            }
        };
        let stem = stem(name);
        let full = survey_model::PointCloud::from_points(stem, &raw.points, raw.colors);
        drop(raw.points);
        // Very large clouds are thinned so they fit in memory (a browser
        // tab has a few GB at most).
        let (cloud, spacing) = full.thinned_to(MAX_CLOUD_POINTS);
        if let Some(s) = spacing {
            self.log.warn(format!(
                "{stem} has {} points; thinned to {} (one per {s:.3} m) to fit in memory",
                full.len(),
                cloud.len()
            ));
        }
        drop(full);
        let summary = format!(
            "Imported point cloud {stem}: {} points{}",
            cloud.len(),
            if cloud.colors.is_some() {
                " with colours"
            } else {
                ""
            }
        );
        self.execute(Box::new(
            ChangeSet::new(format!("Import point cloud {stem}")).insert_cloud(cloud),
        ));
        if let Some((id, _)) = self.project.doc.clouds().last() {
            self.emit(Event::SelectCloud(id));
        }
        self.emit(Event::ZoomExtents);
        self.success(summary);
    }

    /// Loads a `.copc.laz` from disk as a coarse, budget-bounded sample of its
    /// octree (native only). The full cloud stays on disk; this reads enough
    /// whole LOD levels to view it through the in-memory renderer.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_copc(&mut self, path: &std::path::Path) {
        // An overview kept in memory for snapping and surfaces; the view
        // streams its detail from the file (see `cloud_stream`).
        const INITIAL_PREVIEW: usize = 2_000_000;
        let (raw, summary) = match survey_io::copc::read_copc_coarse(path, INITIAL_PREVIEW) {
            Ok(v) => v,
            Err(e) => {
                self.log.error(format!("Could not read {}: {e}", path.display()));
                return;
            }
        };
        let name = crate::platform::file_name(path);
        let stem = stem(&name).trim_end_matches(".copc").to_string();
        let source = survey_model::CloudRef {
            path: path.to_path_buf(),
            total: summary.total,
            full_bounds: survey_core::Aabb3::new(summary.min, summary.max),
            has_color: summary.has_color,
        };
        let cloud = survey_model::PointCloud::from_source(&stem, source, &raw.points, raw.colors);
        drop(raw.points);
        let loaded = cloud.len();
        if (loaded as u64) < summary.total {
            self.log.info(format!(
                "{stem}: drawn in detail where you look; snapping and surfaces use an overview of {loaded} points"
            ));
        }
        self.execute(Box::new(
            ChangeSet::new(format!("Load COPC {stem}")).insert_cloud(cloud),
        ));
        if let Some((id, _)) = self.project.doc.clouds().last() {
            self.emit(Event::SelectCloud(id));
        }
        self.emit(Event::ZoomExtents);
        self.success(format!("Loaded COPC {stem}: {} points", summary.total));
    }

    pub fn import_landxml(&mut self, name: &str, bytes: &[u8]) {
        let (surfaces, warnings) = match survey_io::landxml::read_surfaces(bytes) {
            Ok(r) => r,
            Err(e) => {
                self.log.error(format!("Could not read {name}: {e}"));
                return;
            }
        };
        let n = surfaces.len();
        let mut cmd = ChangeSet::new(format!("Import LandXML {name}"));
        let mut taken: Vec<String> = Vec::new();
        for s in surfaces {
            // Keep names unique in the project.
            let mut surface_name = s.name.clone();
            let mut k = 2;
            while self.project.doc.surface_by_name(&surface_name).is_some()
                || taken.iter().any(|t| t.eq_ignore_ascii_case(&surface_name))
            {
                surface_name = format!("{} ({k})", s.name);
                k += 1;
            }
            taken.push(surface_name.clone());
            cmd = cmd.insert_surface(Surface {
                name: surface_name,
                definition: None,
                tin: s.tin,
                style: SurfaceStyle::default(),
            });
        }
        self.execute(Box::new(cmd));
        self.emit(Event::ZoomExtents);
        self.success(format!(
            "Imported {n} surface{} from {name}",
            if n == 1 { "" } else { "s" }
        ));
        for w in warnings {
            self.log.warn(w);
        }
    }

    pub fn import_dxf(&mut self, name: &str, bytes: &[u8]) {
        match survey_io::dxf::read_dxf(bytes) {
            Ok(drawing) => {
                let cmd = dxf_import(&self.project.doc, &drawing, name);
                let n = cmd.len();
                self.execute(Box::new(cmd));
                self.success(format!("Imported {n} objects from {name}"));
                for w in &drawing.warnings {
                    self.log.warn(w.clone());
                }
                self.emit(Event::ZoomExtents);
            }
            Err(e) => self.log.error(format!("Could not import {name}: {e}")),
        }
    }

    // ------------------------------------------------------------------ exports

    /// The active surface (or every surface when none is active) as LandXML:
    /// a file name stem and the document.
    pub fn surfaces_landxml(&mut self) -> Option<(String, String)> {
        let doc = &self.project.doc;
        let chosen: Vec<(&str, &survey_core::tin::Tin)> =
            match self.active_surface.and_then(|id| doc.surface(id)) {
                Some(s) => vec![(s.name.as_str(), &s.tin)],
                None => doc.surfaces().map(|(_, s)| (s.name.as_str(), &s.tin)).collect(),
            };
        if chosen.is_empty() {
            self.log.warn("There are no surfaces to export");
            return None;
        }
        let xml = survey_io::landxml::write_surfaces(&doc.meta().name, crs_info(doc).as_ref(), &chosen);
        let stem = if chosen.len() == 1 {
            chosen[0].0.to_string()
        } else {
            "surfaces".into()
        };
        Some((stem, xml))
    }

    /// The parcels as LandXML.
    pub fn parcels_landxml(&mut self) -> Option<String> {
        let doc = &self.project.doc;
        let geoms: Vec<(String, String, f64, &survey_model::Polyline)> = doc
            .parcels()
            .iter()
            .filter_map(|p| {
                let l = crate::ui::parcels::boundary(doc, p)?;
                let (_, _, area) = survey_core::arc::polyline_metrics(&l.vertices, &l.bulges, true);
                Some((p.name.clone(), p.plan.clone(), area.abs(), l))
            })
            .collect();
        if geoms.is_empty() {
            self.log.warn("There are no parcels to export");
            return None;
        }
        let parcels: Vec<survey_io::landxml::ParcelGeometry<'_>> = geoms
            .iter()
            .map(|(name, plan, area, l)| survey_io::landxml::ParcelGeometry {
                name,
                description: plan,
                area: *area,
                vertices: &l.vertices,
                bulges: &l.bulges,
            })
            .collect();
        Some(survey_io::landxml::write_parcels(
            &doc.meta().name,
            crs_info(doc).as_ref(),
            &parcels,
        ))
    }

    /// Every point as delimited text laid out as `layout`.
    pub fn points_csv(&self, layout: &CsvLayout) -> String {
        let points: Vec<ImportedPoint> = self
            .project
            .doc
            .entities()
            .filter_map(|(_, e)| match e {
                Entity::Point(p) => Some(ImportedPoint {
                    name: p.name.clone(),
                    easting: p.position.x,
                    northing: p.position.y,
                    height: p.height_known.then_some(p.position.z),
                    code: p.code.clone(),
                    description: p.description.clone(),
                }),
                _ => None,
            })
            .collect();
        survey_io::export_points_csv(&points, layout, 4)
    }

    /// A stake-out file for a controller: the points among `ids` (every point
    /// when `ids` has none) plus the vertices of the polylines among `ids`,
    /// which JobXML also carries as lines between them. A vertex at an exported
    /// point uses that point's name; others are named after their line.
    /// Returns the file text and a summary.
    pub fn stakeout_file(&self, format: StakeFormat, ids: &[EntityId]) -> Result<(String, String), String> {
        let doc = &self.project.doc;
        let as_point = |p: &SurveyPoint| ImportedPoint {
            name: p.name.clone(),
            easting: p.position.x,
            northing: p.position.y,
            height: p.height_known.then_some(p.position.z),
            code: p.code.clone(),
            description: p.description.clone(),
        };
        let chosen_points = ids
            .iter()
            .any(|id| matches!(doc.entity(*id), Some(Entity::Point(_))));
        let mut points: Vec<ImportedPoint> = doc
            .entities()
            .filter_map(|(id, e)| match e {
                Entity::Point(p) if !chosen_points || ids.contains(&id) => Some(as_point(p)),
                _ => None,
            })
            .collect();
        let polylines: Vec<&survey_model::Polyline> = ids
            .iter()
            .filter_map(|id| match doc.entity(*id) {
                Some(Entity::Polyline(l)) => Some(l),
                _ => None,
            })
            .collect();
        if !chosen_points && !polylines.is_empty() {
            points.clear();
        }
        let mut lines = Vec::new();
        let mut arcs = 0;
        for (k, l) in polylines.iter().enumerate() {
            let base = if !l.name.is_empty() {
                l.name.clone()
            } else if !l.code.is_empty() {
                format!("{}{}", l.code, k + 1)
            } else {
                format!("L{}", k + 1)
            };
            let mut names = Vec::new();
            for (i, v) in l.vertices.iter().enumerate() {
                let near = |p: &DVec2| p.distance(v.truncate()) < 0.0005;
                let existing = points
                    .iter()
                    .find(|p| near(&DVec2::new(p.easting, p.northing)))
                    .map(|p| p.name.clone())
                    .or_else(|| {
                        doc.entities().find_map(|(_, e)| match e {
                            Entity::Point(p) if near(&p.position.truncate()) => {
                                points.push(as_point(p));
                                Some(p.name.clone())
                            }
                            _ => None,
                        })
                    });
                let name = existing.unwrap_or_else(|| {
                    // Not a name already in the file or the project.
                    let taken =
                        |n: &str| points.iter().any(|p| p.name == n) || doc.find_point_by_name(n).is_some();
                    let mut name = format!("{base}-{}", i + 1);
                    let mut k = 2;
                    while taken(&name) {
                        name = format!("{base}-{}-{k}", i + 1);
                        k += 1;
                    }
                    points.push(ImportedPoint {
                        name: name.clone(),
                        easting: v.x,
                        northing: v.y,
                        height: l.heights_known.then_some(v.z),
                        code: l.code.clone(),
                        description: String::new(),
                    });
                    name
                });
                names.push(name);
            }
            let n = names.len();
            let segments = if l.closed { n } else { n.saturating_sub(1) };
            for i in 0..segments {
                if l.bulges.get(i).is_some_and(|b| b.abs() > 1e-12) {
                    arcs += 1;
                }
                lines.push(StakeLine {
                    name: format!("{base}.{}", i + 1),
                    code: l.code.clone(),
                    from: names[i].clone(),
                    to: names[(i + 1) % n].clone(),
                });
            }
        }
        if points.is_empty() {
            return Err("There are no points or lines to export".into());
        }
        let job = if doc.meta().name.is_empty() {
            "survey3d".to_string()
        } else {
            doc.meta().name.clone()
        };
        let mut notes = Vec::new();
        let text = match format {
            StakeFormat::JobXml => {
                if arcs > 0 {
                    notes.push(format!(
                        "{arcs} arcs written as straight lines between their ends"
                    ));
                }
                survey_io::stakeout::write_jobxml(&job, &points, &lines)
            }
            StakeFormat::Rw5 => survey_io::stakeout::write_rw5_points(&job, &points),
            StakeFormat::Gsi => {
                let (text, changed) = survey_io::stakeout::write_gsi16(&points);
                if !changed.is_empty() {
                    notes.push(format!(
                        "{} names or codes shortened to 16 characters or cleaned for GSI: {}",
                        changed.len(),
                        changed.join(", ")
                    ));
                }
                text
            }
            StakeFormat::Csv => survey_io::export_points_csv(&points, &CsvLayout::from_letters("PNEZC"), 4),
        };
        let mut summary = format!("Stake-out file ({}): {} points", format.label(), points.len());
        if format == StakeFormat::JobXml && !lines.is_empty() {
            let _ = write!(summary, ", {} lines", lines.len());
        }
        for n in notes {
            let _ = write!(summary, "; {n}");
        }
        Ok((text, summary))
    }

    /// A plan sheet drawn for printing (labels as the application's defaults).
    pub fn plot(&self, sheet: &SheetRecord) -> Plot {
        let l = LabelSettings::default();
        let labels = LabelSet {
            name: l.names,
            code: l.codes,
            height: l.heights,
            description: false,
        };
        crate::plan::build_plot(&self.project.doc, sheet, labels)
    }

    /// A north-up A3 sheet framing the whole drawing at a standard scale.
    pub fn overview_sheet(&self) -> SheetRecord {
        let doc = &self.project.doc;
        let b = doc.bounds();
        let (min, max) = if b.is_empty() {
            (DVec2::splat(-50.0), DVec2::splat(50.0))
        } else {
            (b.min.truncate(), b.max.truncate())
        };
        let centre = (min + max) * 0.5;
        let view = ViewInfo {
            centre,
            up_bearing: 0.0,
            extent: (max - min).max(DVec2::splat(1.0)),
            bounds: Some((min, max)),
        };
        let mut sheet = SheetsWindow::sheet_from_view(doc, &view, doc.sheets().len() + 1);
        sheet.name = "Overview".into();
        // Fit the drawing in 90% of the frame above the title block, not under it.
        let size = view.extent;
        for scale in survey_model::sheet::STANDARD_SCALES {
            sheet.scale = scale;
            let (_, clear) = clear_of_title(&sheet);
            if size.x <= clear.x * 0.9 && size.y <= clear.y * 0.9 {
                break;
            }
        }
        let (offset, _) = clear_of_title(&sheet);
        sheet.centre = (centre - offset).to_array();
        sheet
    }

    /// A new sheet as `set_record` would make it when no centre or scale is
    /// given: framing the whole drawing, named as the next sheet.
    pub fn new_sheet(&self) -> SheetRecord {
        let mut sheet = self.overview_sheet();
        sheet.name = format!("Sheet {}", self.project.doc.sheets().len() + 1);
        sheet
    }

    /// The drawing as DXF, with symbols and text sized for `sheet` (else 1:500).
    pub fn dxf(&self, sheet: Option<&SheetRecord>) -> (String, DxfExportOptions) {
        let opts = sheet.map_or_else(DxfExportOptions::default, DxfExportOptions::from_sheet);
        let text = survey_io::dxf::write_dxf(&dxf_export(&self.project.doc, &opts));
        (text, opts)
    }
}

/// The project's coordinate system for LandXML, unless it is unspecified.
fn crs_info(doc: &survey_model::Document) -> Option<survey_io::landxml::CrsInfo> {
    let crs = &doc.meta().coord_system;
    if matches!(crs, survey_model::CoordinateSystem::Unspecified) {
        return None;
    }
    Some(survey_io::landxml::CrsInfo {
        name: crs.display_name(),
        epsg: crs.epsg(),
        wkt: crs.wkt(),
        ellipsoid: crs.projection().map(|tm| tm.ellipsoid.name.to_string()),
        geoid: crs.geoid_model().map(str::to_string),
    })
}
