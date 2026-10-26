//! An open project: document, undo history, selection and its backing store.

use survey_model::{Document, History, Selection};
use survey_store::{ProjectStore, StoreError};

use crate::settings::AppSettings;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Project {
    pub doc: Document,
    pub history: History,
    pub selection: Selection,
    pub store: ProjectStore,
    /// File name shown in the title bar.
    pub display_name: String,
    /// For a brand-new project: the document revision at creation. Until it
    /// changes there is nothing worth saving, so the project is not "dirty".
    pristine_revision: Option<u64>,
}

impl Project {
    pub fn new(settings: &AppSettings) -> Result<Self, StoreError> {
        let mut doc = Document::new();
        let mut meta = doc.meta().clone();
        meta.conventions = settings.default_conventions.clone();
        // Not an edit: initial settings of a brand-new project.
        doc.set_meta(meta);
        let pristine_revision = Some(doc.revision());
        Ok(Self {
            doc,
            history: History::new(settings.user_name.clone()),
            selection: Selection::default(),
            store: ProjectStore::create_in_memory()?,
            display_name: "Untitled".into(),
            pristine_revision,
        })
    }

    pub fn from_store(
        store: ProjectStore,
        display_name: String,
        settings: &AppSettings,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            doc: store.load_document()?,
            history: History::new(settings.user_name.clone()),
            selection: Selection::default(),
            store,
            display_name,
            pristine_revision: None,
        })
    }

    /// Unsaved edits exist (document changes or audit entries not yet written).
    pub fn is_dirty(&self) -> bool {
        let untouched = self.pristine_revision == Some(self.doc.revision());
        (self.doc.is_dirty() && !untouched) || !self.history.pending_audit().is_empty()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn has_file(&self) -> bool {
        self.store.file_path().is_some()
    }

    /// Writes dirty records and pending audit entries to the store.
    pub fn save(&mut self) -> Result<(), StoreError> {
        let audit = self.history.take_pending_audit();
        match self.store.save(&mut self.doc, &audit, APP_VERSION) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.history.restore_pending_audit(audit);
                Err(e)
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_as(&mut self, path: &std::path::Path) -> Result<(), StoreError> {
        // Copy the file as last saved, then write the unsaved changes into the
        // copy: the original keeps what it had.
        self.store.save_as_file(path)?;
        self.display_name = crate::platform::file_name(path);
        self.save()
    }

    /// The project including unsaved changes, without saving (crash recovery).
    pub fn snapshot(&self) -> Result<Vec<u8>, StoreError> {
        self.store
            .snapshot_bytes(&self.doc, self.history.pending_audit(), APP_VERSION)
    }

    /// Saves and returns the whole project file (web download).
    #[cfg(target_arch = "wasm32")]
    pub fn save_to_bytes(&mut self) -> Result<Vec<u8>, StoreError> {
        self.save()?;
        self.store.to_bytes()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use glam::DVec3;
    use survey_model::{ChangeSet, Entity, SurveyPoint};

    fn add_point(p: &mut Project, name: &str) {
        let point = Entity::Point(SurveyPoint {
            name: name.into(),
            position: DVec3::new(1.0, 2.0, 3.0),
            height_known: true,
            code: String::new(),
            description: String::new(),
            layer: p.doc.current_layer(),
        });
        p.history
            .execute(Box::new(ChangeSet::new("add").insert_entity(point)), &mut p.doc);
    }

    fn names(path: &std::path::Path) -> Vec<String> {
        // A copy: an open project holds the file itself.
        let bytes = std::fs::read(path).expect("read");
        let doc = ProjectStore::from_bytes(&bytes)
            .expect("open")
            .load_document()
            .expect("load");
        let mut n: Vec<String> = doc.entities().map(|(_, e)| e.name().to_string()).collect();
        n.sort();
        n
    }

    #[test]
    fn save_as_leaves_the_original_alone() {
        let dir = std::env::temp_dir().join(format!("survey3d-saveas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let (a, b) = (dir.join("a.s3d"), dir.join("b.s3d"));
        let settings = AppSettings::default();

        // A new project saved as A with one point.
        let mut p = Project::new(&settings).expect("project");
        add_point(&mut p, "P1");
        p.save_as(&a).expect("save a");
        assert_eq!(names(&a), ["P1"]);

        // An older file already at B (to be replaced).
        let mut other = Project::new(&settings).expect("project");
        add_point(&mut other, "OLD");
        other.save_as(&b).expect("save b");
        drop(other);

        // Reopen A, add a point, save as B: A is untouched, B has both.
        drop(p);
        let store = ProjectStore::open_file(&a).expect("open a");
        let mut p = Project::from_store(store, "a".into(), &settings).expect("project");
        add_point(&mut p, "P2");
        p.save_as(&b).expect("save as b");
        assert_eq!(names(&a), ["P1"], "the original keeps what it had");
        assert_eq!(names(&b), ["P1", "P2"]);
        assert!(!p.is_dirty());
        // Saving as the same file is just a save.
        add_point(&mut p, "P3");
        p.save_as(&b).expect("save as itself");
        assert_eq!(names(&b), ["P1", "P2", "P3"]);
        assert!(!dir.join("b.s3d.saving").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
