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
}

impl Project {
    pub fn new(settings: &AppSettings) -> Result<Self, StoreError> {
        let mut doc = Document::new();
        let mut meta = doc.meta().clone();
        meta.conventions = settings.default_conventions.clone();
        // Not an edit: initial settings of a brand-new project.
        doc.set_meta(meta);
        Ok(Self {
            doc,
            history: History::new(settings.user_name.clone()),
            selection: Selection::default(),
            store: ProjectStore::create_in_memory()?,
            display_name: "Untitled".into(),
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
        })
    }

    /// Unsaved edits exist (document changes or audit entries not yet written).
    pub fn is_dirty(&self) -> bool {
        self.doc.is_dirty() || !self.history.pending_audit().is_empty()
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
        self.save()?;
        self.store.save_as_file(path)?;
        self.display_name = crate::platform::file_name(path);
        Ok(())
    }

    /// Saves and returns the whole project file (web download).
    #[cfg(target_arch = "wasm32")]
    pub fn save_to_bytes(&mut self) -> Result<Vec<u8>, StoreError> {
        self.save()?;
        self.store.to_bytes()
    }
}
