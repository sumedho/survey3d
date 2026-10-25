//! Reading files into the project and building exports. File choice and saving
//! are the caller's: these take and return bytes and text.

use glam::DVec2;
use survey_core::style::LabelSet;
use survey_io::plot::Plot;
use survey_io::{CsvLayout, CsvPointImporter, ImportedPoint};
use survey_model::{ChangeSet, Entity, SheetRecord, Surface, SurfaceStyle, SurveyPoint};

use super::{Event, Panel, Session};
use crate::import_command::ImportEntities;
use crate::interchange::{DxfExportOptions, dxf_export, dxf_import};
use crate::settings::LabelSettings;
use crate::ui::sheets::{SheetsWindow, ViewInfo};

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

    /// Raw instrument data: kept with the project for the network adjustment;
    /// coordinates in the file add any points the project does not have. A file
    /// of the same name replaces the earlier import.
    pub fn import_raw(&mut self, name: &str, bytes: &[u8]) {
        let stem = stem(name);
        let data = match survey_io::rawdata::read_raw(stem, bytes) {
            Ok(data) => data,
            Err(e) => {
                self.log.error(format!("Could not read {name}: {e}"));
                return;
            }
        };
        let doc = &self.project.doc;
        let mut cs = ChangeSet::new(format!("Import raw data {stem}"));
        let mut added = 0;
        for (pname, p) in &data.points {
            if doc.find_point_by_name(pname).is_none() {
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
        let source = format!("f:{}", data.name);
        let existing = doc
            .field_data()
            .iter()
            .find(|f| f.name == data.name)
            .map(|f| f.id);
        let cs = match existing {
            Some(id) => cs.update_record(id, data),
            None => cs.insert_record(data),
        };
        self.execute(Box::new(cs));
        self.emit(Event::Open(Panel::Network { source }));
        if added > 0 {
            self.emit(Event::ZoomExtents);
        }
        self.success(summary);
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
        // A light overview to show immediately; the view-dependent streamer
        // refines it (and reads only what the camera needs) from here on.
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
        let coarse = (loaded as u64) < summary.total;
        if coarse {
            self.log.warn(format!(
                "{stem} has {} points; showing a coarse preview of {loaded} ({} LOD level{})",
                summary.total,
                summary.levels_read,
                if summary.levels_read == 1 { "" } else { "s" },
            ));
        }
        self.execute(Box::new(
            ChangeSet::new(format!("Load COPC {stem}")).insert_cloud(cloud),
        ));
        if let Some((id, _)) = self.project.doc.clouds().last() {
            self.emit(Event::SelectCloud(id));
        }
        self.emit(Event::ZoomExtents);
        self.success(format!(
            "Loaded COPC {stem}: {loaded} points{}",
            if coarse { " (coarse preview)" } else { "" }
        ));
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
        let xml = survey_io::landxml::write_surfaces(&doc.meta().name, &chosen);
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
        Some(survey_io::landxml::write_parcels(&doc.meta().name, &parcels))
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
        let view = ViewInfo {
            centre: (min + max) * 0.5,
            up_bearing: 0.0,
            // The sheet takes 90% of this, and fits the drawing in 90% of its frame.
            extent: (max - min).max(DVec2::splat(1.0)) / 0.9,
            bounds: Some((min, max)),
        };
        let mut sheet = SheetsWindow::sheet_from_view(doc, &view, doc.sheets().len() + 1);
        sheet.name = "Overview".into();
        sheet
    }

    /// The drawing as DXF, with symbols and text sized for `sheet` (else 1:500).
    pub fn dxf(&self, sheet: Option<&SheetRecord>) -> (String, DxfExportOptions) {
        let opts = sheet.map_or_else(DxfExportOptions::default, DxfExportOptions::from_sheet);
        let text = survey_io::dxf::write_dxf(&dxf_export(&self.project.doc, &opts));
        (text, opts)
    }
}
