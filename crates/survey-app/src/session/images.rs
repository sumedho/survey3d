//! Georeferenced images: importing (placed by GeoTIFF tags or a world file,
//! else roughly until control points place it), control points and their
//! fit, and world files.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::DVec2;
use survey_core::georef::{ControlPoint, ImageTransform, fit};
use survey_model::{ChangeSet, Document, ImageRecord, Placement, RecordId};

use super::Session;

/// The control points and their residuals as text.
pub fn control_report(doc: &Document, rec: &ImageRecord) -> String {
    let conv = doc.conventions();
    let mut s = String::new();
    let _ = writeln!(s, "IMAGE  {}", rec.name);
    let _ = writeln!(
        s,
        "{} × {} pixels, {} per pixel",
        rec.width,
        rec.height,
        conv.format_length_with_unit(rec.transform.pixel_size())
    );
    let fitted = fit(&rec.control_points, rec.fit).ok();
    let _ = writeln!(
        s,
        "{}: {}",
        rec.fit.label(),
        match &fitted {
            Some(f) => format!(
                "RMS {} from {} points",
                conv.format_length_with_unit(f.rms),
                f.used
            ),
            None => format!("needs {} points", rec.fit.min_points()),
        }
    );
    let _ = writeln!(s);
    let (ea, nb) = conv.horizontal_labels();
    let _ = writeln!(
        s,
        "{:>3} {:>10} {:>10} {:>14} {:>14} {:>10}  Use",
        "#", "Column", "Row", ea, nb, "Residual"
    );
    for (i, p) in rec.control_points.iter().enumerate() {
        let r = fitted
            .as_ref()
            .and_then(|f| f.residuals.get(i))
            .map(|r| conv.format_length(r.length()));
        let (a, b) = conv.horizontal_values(DVec2::from(p.world).extend(0.0));
        let _ = writeln!(
            s,
            "{:>3} {:>10.1} {:>10.1} {:>14} {:>14} {:>10}  {}",
            i + 1,
            p.pixel[0],
            p.pixel[1],
            conv.format_length(a),
            conv.format_length(b),
            r.unwrap_or_default(),
            if p.enabled { "yes" } else { "check" }
        );
    }
    s
}

impl Session {
    /// Adds an image, placed by its GeoTIFF tags or world file, else roughly
    /// over the drawing until control points place it. Its tile cache builds
    /// in the background.
    pub fn import_image(&mut self, path: &Path) -> Result<(RecordId, String), String> {
        let info = survey_io::imagery::inspect(path).map_err(|e| e.to_string())?;
        let doc = &self.project.doc;
        if let Some(r) = doc.images().iter().find(|r| r.path == path) {
            return Err(format!(
                "{} is already in the project as {}",
                path.display(),
                r.name
            ));
        }
        let mut notes = Vec::new();
        let project_epsg = doc.meta().coord_system.epsg();
        let (transform, placement) = match info.transform {
            Some(t) if !info.geographic => {
                if let (Some(a), Some(b)) = (info.epsg, project_epsg)
                    && a != b
                {
                    notes.push(format!(
                        "the image is in EPSG:{a} but the project in EPSG:{b}; it is placed by its own coordinates \
                         (reproject it with gdalwarp -t_srs EPSG:{b} to match)"
                    ));
                }
                (t, Placement::File)
            }
            _ => {
                notes.push(if info.geographic {
                    "the GeoTIFF is in latitude and longitude, not a grid: placed roughly; reproject it to the \
                     project's grid with gdalwarp, or place it with control points (imgcp)"
                        .into()
                } else {
                    "it has no placement (no GeoTIFF tags or world file): placed roughly; add control points (imgcp) to place it"
                        .to_string()
                });
                // Over the drawing (or the origin), as wide as the drawing, until placed.
                let b = doc.bounds();
                let (centre, span) = if b.is_empty() {
                    (DVec2::ZERO, f64::from(info.width))
                } else {
                    (
                        b.center().truncate(),
                        (b.max - b.min).truncate().max_element().max(1.0),
                    )
                };
                let size = span / f64::from(info.width.max(1));
                let top_left =
                    centre + DVec2::new(-f64::from(info.width), f64::from(info.height)) * size * 0.5;
                (ImageTransform::north_up(top_left, size), Placement::Rough)
            }
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Image")
            .to_string();
        // The lowest surveyed height (lines and points without heights sit at 0 and do not count).
        let elevation = doc
            .entities()
            .filter_map(|(_, e)| match e {
                survey_model::Entity::Point(p) if p.height_known => Some(p.position.z),
                survey_model::Entity::Polyline(l) if l.heights_known => {
                    l.vertices.iter().map(|v| v.z).reduce(f64::min)
                }
                _ => None,
            })
            .reduce(f64::min)
            .unwrap_or(0.0);
        let rec = ImageRecord {
            name: name.clone(),
            path: path.to_path_buf(),
            width: info.width,
            height: info.height,
            transform,
            placement,
            epsg: info.epsg,
            elevation,
            original: Some((transform, placement)),
            ..Default::default()
        };
        let conv = doc.conventions();
        let mut summary = format!(
            "Image {name}: {} × {} pixels, {} per pixel, {}",
            info.width,
            info.height,
            conv.format_length_with_unit(transform.pixel_size()),
            match &info.placed_by {
                Some(by) if placement == Placement::File => format!("placed by {by}"),
                _ => "not placed".into(),
            }
        );
        for n in &notes {
            summary.push_str(&format!("; {n}"));
        }
        self.execute(Box::new(
            ChangeSet::new(format!("Add image {name}")).insert_record(rec),
        ));
        let id = self
            .project
            .doc
            .images()
            .last()
            .map(|r| r.id)
            .ok_or("the image was not added")?;
        self.log.result(summary.clone());
        Ok((id, summary))
    }

    /// Adds a control point (a pixel of the image and its true position) and
    /// refits; one undoable step.
    pub fn add_control_point(&mut self, id: RecordId, pixel: DVec2, world: DVec2) -> Result<String, String> {
        let mut rec = self.image(id)?.clone();
        rec.control_points.push(ControlPoint {
            pixel: pixel.to_array(),
            world: world.to_array(),
            enabled: true,
        });
        let n = rec.control_points.len();
        self.refit_image(id, rec, format!("Control point {n}"))
    }

    /// Stores changed control points or fit and refits; one undoable step.
    pub fn refit_image(
        &mut self,
        id: RecordId,
        mut rec: ImageRecord,
        what: String,
    ) -> Result<String, String> {
        let fitted = rec.refit()?;
        let conv = self.project.doc.conventions();
        let summary = match &fitted {
            Some(f) => {
                let worst = f
                    .residuals
                    .iter()
                    .zip(&rec.control_points)
                    .enumerate()
                    .filter(|(_, (_, p))| p.enabled)
                    .max_by(|a, b| a.1.0.length().total_cmp(&b.1.0.length()));
                format!(
                    "{} on {}: {} RMS {} from {} points, {} per pixel{}{}",
                    what,
                    rec.name,
                    rec.fit.label(),
                    conv.format_length_with_unit(f.rms),
                    f.used,
                    conv.format_length_with_unit(rec.transform.pixel_size()),
                    worst
                        .map(|(i, (r, _))| format!(
                            ", largest residual {} at point {}",
                            conv.format_length(r.length()),
                            i + 1
                        ))
                        .unwrap_or_default(),
                    affine_hint(conv, &rec, f.rms)
                )
            }
            None => format!(
                "{what} on {}: {} of {} points for a {} fit",
                rec.name,
                rec.control_points.iter().filter(|p| p.enabled).count(),
                rec.fit.min_points(),
                rec.fit.label()
            ),
        };
        let label = format!("{what} on image {}", rec.name);
        self.execute(Box::new(ChangeSet::new(label).update_record(id, rec)));
        self.log.result(summary.clone());
        Ok(summary)
    }

    pub fn image(&self, id: RecordId) -> Result<&ImageRecord, String> {
        self.project
            .doc
            .record::<ImageRecord>(id)
            .ok_or_else(|| "No such image".to_string())
    }

    /// Writes the image's placement as a world file beside it (`.tfw`,
    /// `.jgw` or `.pgw`).
    pub fn save_world_file(&mut self, id: RecordId) -> Result<PathBuf, String> {
        let rec = self.image(id)?;
        let ext = match rec
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("tif" | "tiff") => "tfw",
            Some("jpg" | "jpeg") => "jgw",
            Some("png") => "pgw",
            _ => "wld",
        };
        let out = rec.path.with_extension(ext);
        std::fs::write(&out, rec.transform.to_world_file())
            .map_err(|e| format!("Could not write {}: {e}", out.display()))?;
        self.log.result(format!("World file written: {}", out.display()));
        Ok(out)
    }

    /// Points the image at another file (moved or renamed) and rebuilds its tiles.
    pub fn relink_image(&mut self, id: RecordId, path: &Path) -> Result<String, String> {
        let info = survey_io::imagery::inspect(path).map_err(|e| e.to_string())?;
        let mut rec = self.image(id)?.clone();
        if let Some(other) = self
            .project
            .doc
            .images()
            .iter()
            .find(|r| r.id != id && r.path == path)
        {
            return Err(format!(
                "{} is already in the project as {}",
                path.display(),
                other.name
            ));
        }
        let ext = |p: &Path| match p
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("tiff") => "tif".to_string(),
            Some("jpeg") => "jpg".to_string(),
            e => e.unwrap_or_default().to_string(),
        };
        if ext(path) != ext(&rec.path) {
            return Err(format!(
                "{} is a .{} file, not .{} like {}: not the same image",
                path.display(),
                ext(path),
                ext(&rec.path),
                rec.name
            ));
        }
        // A file that places itself must place itself where this one did.
        let (original, how) = rec.original.unwrap_or((rec.transform, rec.placement));
        if let (Some(t), survey_model::Placement::File) = (info.transform, how) {
            let (w, h) = (f64::from(rec.width), f64::from(rec.height));
            let moved = t
                .corners(w, h)
                .iter()
                .zip(original.corners(w, h))
                .map(|(a, b)| a.distance(b))
                .fold(0.0, f64::max);
            if moved > original.pixel_size() {
                return Err(format!(
                    "{} places itself {:.2} m from where {} was: not the same image",
                    path.display(),
                    moved,
                    rec.name
                ));
            }
        }
        if (info.width, info.height) != (rec.width, rec.height) {
            return Err(format!(
                "{} is {} × {} pixels, not {} × {}: not the same image",
                path.display(),
                info.width,
                info.height,
                rec.width,
                rec.height
            ));
        }
        rec.path = path.to_path_buf();
        let name = rec.name.clone();
        self.execute(Box::new(
            ChangeSet::new(format!("Relink image {name}")).update_record(id, rec),
        ));
        let msg = format!("Image {name} now reads {}", path.display());
        self.log.result(msg.clone());
        Ok(msg)
    }

    /// Starts or finishes image tile caches (and logs what finished).
    pub fn sync_imagery(&mut self) {
        for m in self.imagery.sync(&self.project.doc) {
            if m.contains("could not") {
                self.log.warn(m);
            } else {
                self.log.info(m);
            }
        }
    }
}

/// When a Helmert fit leaves large residuals that an affine fit would not
/// (an image with unequal or sheared pixels), says so.
fn affine_hint(conv: &survey_core::Conventions, rec: &ImageRecord, rms: f64) -> String {
    if rec.fit != survey_core::georef::FitKind::Helmert
        || rec.control_points.iter().filter(|p| p.enabled).count() < 3
        || rms < 3.0 * rec.transform.pixel_size()
    {
        return String::new();
    }
    match fit(&rec.control_points, survey_core::georef::FitKind::Affine) {
        Ok(a) if a.rms < rms * 0.25 => format!(
            ". An affine fit gives RMS {}: the image's pixels are not square or are sheared (use fit affine)",
            conv.format_length_with_unit(a.rms)
        ),
        _ => String::new(),
    }
}
