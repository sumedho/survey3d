//! The project's geoid model: found by file name in the user's geoid folder,
//! loaded once and handed to the document; and converting point heights
//! between the ellipsoid and the geoid.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use survey_core::geoid::GeoidModel;
use survey_model::{ChangeSet, Entity, EntityId};

use super::Session;
use super::points::{from_raw_data, some_names};

/// The geoid file loaded, and the last one that failed (not retried until
/// the folder or the project's model changes).
#[derive(Default)]
pub(super) struct GeoidState {
    loaded: Option<(PathBuf, Arc<GeoidModel>)>,
    failed: Option<PathBuf>,
}

pub const GEOID_HELP: &str = "geoid — the separation N at the selected points; geoid ortho — their heights from \
                              the ellipsoid to the geoid (H = h − N); geoid ellip — back to the ellipsoid (h = H + N)";

/// Geoid model files in a folder, by name.
pub fn geoid_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| survey_io::geoid::is_geoid_file(n))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

impl Session {
    /// The folder geoid models are found in (native only).
    pub fn geoid_dir(&self) -> Option<&Path> {
        self.geoid_dir.as_deref()
    }

    pub fn set_geoid_dir(&mut self, dir: Option<PathBuf>) {
        if self.geoid_dir != dir {
            self.geoid_dir = dir;
            self.geoid.failed = None;
            self.sync_geoid();
        }
    }

    /// The file the project's geoid model would be read from.
    fn geoid_path(&self) -> Option<PathBuf> {
        let name = self.project.doc.meta().coord_system.geoid_model()?;
        Some(self.geoid_dir.as_ref()?.join(name))
    }

    /// Loads the project's geoid model if it is not already loaded (or drops
    /// it when the project no longer names one). Cheap when nothing changed,
    /// so it is called after every operation.
    pub fn sync_geoid(&mut self) {
        let Some(path) = self.geoid_path() else {
            if self.project.doc.geoid().is_some() {
                self.project.doc.set_geoid(None);
            }
            return;
        };
        if let Some((loaded, model)) = &self.geoid.loaded
            && *loaded == path
        {
            if !self.project.doc.geoid().is_some_and(|g| Arc::ptr_eq(g, model)) {
                self.project.doc.set_geoid(Some(model.clone()));
            }
            return;
        }
        if self.project.doc.geoid().is_some() {
            self.project.doc.set_geoid(None);
        }
        if self.geoid.failed.as_ref() == Some(&path) {
            return;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| survey_io::geoid::read_geoid(&name, &bytes).map_err(|e| e.to_string()));
        match read {
            Ok(model) => {
                let (s, w, n, e) = model.extent();
                self.log.info(format!(
                    "Geoid model {name} loaded ({:.1}′ grid, {:.0}° to {:.0}° latitude, {:.0}° to {:.0}° longitude)",
                    model.resolution_minutes(),
                    s,
                    n,
                    w,
                    e
                ));
                let model = Arc::new(model);
                self.project.doc.set_geoid(Some(model.clone()));
                self.geoid.loaded = Some((path, model));
            }
            Err(e) => {
                self.log.warn(format!(
                    "Geoid model {name} could not be read from {}: {e}. Heights use the project's constant separation",
                    path.parent().map(|p| p.display().to_string()).unwrap_or_default()
                ));
                self.geoid.failed = Some(path);
            }
        }
    }

    /// One line on the geoid in use, for settings and project info.
    pub fn geoid_status(&self) -> String {
        let doc = &self.project.doc;
        let crs = &doc.meta().coord_system;
        let constant = format!("constant N = {:.3} m", crs.geoid_separation());
        match (crs.geoid_model(), doc.geoid()) {
            (_, Some(m)) => format!(
                "{} ({:.1}′ grid); outside it, {constant}",
                m.name,
                m.resolution_minutes()
            ),
            (Some(name), None) if self.geoid_dir.is_none() => {
                format!("{name} is not loaded: set the geoid folder. Using {constant}")
            }
            (Some(name), None) => format!("{name} is not loaded (see the log). Using {constant}"),
            (None, None) => format!("No geoid model: {constant}"),
        }
    }

    /// Converts the heights of the points among `ids` between the ellipsoid
    /// and the geoid with the separation at each point.
    pub fn convert_heights(&mut self, ids: &[EntityId], to_orthometric: bool) -> Result<String, String> {
        let doc = &self.project.doc;
        if doc.meta().coord_system.projection().is_none() {
            return Err(
                "Set a projected coordinate system first: the geoid is found by latitude and longitude"
                    .into(),
            );
        }
        if doc.geoid().is_none() && doc.meta().coord_system.geoid_separation() == 0.0 {
            return Err(format!(
                "No geoid model is loaded and the project's separation is 0, so the heights would not change ({})",
                self.geoid_status()
            ));
        }
        let conv = doc.conventions();
        let (mut cs, mut done, mut skipped) = (ChangeSet::new(""), 0, 0);
        let (mut from_model, mut from_constant) = (0, 0);
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut from_raw = Vec::new();
        for &id in ids {
            match doc.entity(id) {
                Some(Entity::Point(p)) if p.height_known && doc.is_entity_editable(id) => {
                    let s = doc.separation_at(p.position.x, p.position.y);
                    if s.model.is_some() {
                        from_model += 1;
                    } else {
                        from_constant += 1;
                    }
                    lo = lo.min(s.n);
                    hi = hi.max(s.n);
                    if from_raw_data(doc, &p.name) {
                        from_raw.push(p.name.as_str());
                    }
                    let mut q = p.clone();
                    q.position.z += if to_orthometric { -s.n } else { s.n };
                    cs.push_update_entity(id, Entity::Point(q));
                    done += 1;
                }
                Some(Entity::Point(_)) => skipped += 1,
                _ => {}
            }
        }
        if done == 0 {
            return Err("Select points with heights to convert".into());
        }
        let (from, to) = if to_orthometric {
            ("ellipsoidal", "orthometric")
        } else {
            ("orthometric", "ellipsoidal")
        };
        let separation = if hi - lo < 0.0005 {
            format!("N {}", conv.format_length(lo))
        } else {
            format!("N {} to {}", conv.format_length(lo), conv.format_length(hi))
        };
        let mut summary = format!(
            "Heights of {done} point{} from {from} to {to} ({separation})",
            if done == 1 { "" } else { "s" }
        );
        if from_constant > 0 {
            summary.push_str(&format!(
                "; {from_constant} with the constant N = {}",
                conv.format_length(doc.meta().coord_system.geoid_separation())
            ));
            if from_model > 0 {
                summary.push_str(" (outside the geoid model)");
            }
        }
        if skipped > 0 {
            summary.push_str(&format!("; {skipped} without heights or locked left as they are"));
        }
        let warning = (!from_raw.is_empty()).then(|| {
            format!(
                "{} of these points come from raw data ({}): reducing it again puts their heights back",
                from_raw.len(),
                some_names(&from_raw)
            )
        });
        let label = format!("Heights {from} to {to} ({done} points)");
        self.execute(Box::new(cs.with_label(label)));
        self.log.result(summary.clone());
        if let Some(w) = warning {
            self.log.warn(w.clone());
            summary = format!("{summary}. Warning: {w}");
        }
        Ok(summary)
    }

    /// The `geoid` command on the selection.
    pub(super) fn geoid_command(&mut self, args: &str) -> Result<String, String> {
        let ids: Vec<EntityId> = self.project.selection.ids().iter().copied().collect();
        match args.trim().to_ascii_lowercase().as_str() {
            "ortho" | "orthometric" | "h2h" => self.convert_heights(&ids, true),
            "ellip" | "ellipsoidal" => self.convert_heights(&ids, false),
            "" => {
                let doc = &self.project.doc;
                let mut lines = vec![self.geoid_status()];
                for id in &ids {
                    if let Some(Entity::Point(p)) = doc.entity(*id) {
                        let s = doc.separation_at(p.position.x, p.position.y);
                        lines.push(format!(
                            "{}  N {}{}",
                            p.name,
                            doc.conventions().format_length(s.n),
                            if s.model.is_some() { "" } else { " (constant)" }
                        ));
                    }
                }
                let text = lines.join("\n");
                for l in &lines {
                    self.log.result(l.clone());
                }
                Ok(text)
            }
            _ => Err(format!("Usage: {GEOID_HELP}")),
        }
    }
}
