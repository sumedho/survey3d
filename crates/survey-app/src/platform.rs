//! File dialogs and downloads, abstracted over native and web.
//!
//! Dialog results arrive asynchronously on the web, so everything is delivered
//! as [`FileEvent`]s polled once per frame.

use std::sync::mpsc::{Receiver, Sender, channel};

pub enum FileEvent {
    /// A project chosen to open, as bytes (web) …
    #[cfg(target_arch = "wasm32")]
    ProjectBytes { name: String, bytes: Vec<u8> },
    /// … or as a path (native).
    #[cfg(not(target_arch = "wasm32"))]
    ProjectPath(std::path::PathBuf),
    /// A delimited text file chosen for import.
    CsvBytes { name: String, bytes: Vec<u8> },
}

pub struct Platform {
    tx: Sender<FileEvent>,
    rx: Receiver<FileEvent>,
}

impl Default for Platform {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }
}

const PROJECT_FILTER: (&str, &[&str]) = ("survey3d project", &[survey_store::FILE_EXTENSION]);
const CSV_FILTER: (&str, &[&str]) = ("Point files", &["csv", "txt", "pnt", "xyz"]);

impl Platform {
    pub fn poll(&self) -> Vec<FileEvent> {
        self.rx.try_iter().collect()
    }

    pub fn pick_project(&self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(PROJECT_FILTER.0, PROJECT_FILTER.1)
                .pick_file()
            {
                let _ = self.tx.send(FileEvent::ProjectPath(path));
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.pick_bytes(ctx, PROJECT_FILTER, |name, bytes| FileEvent::ProjectBytes {
            name,
            bytes,
        });
    }

    pub fn pick_csv(&self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(CSV_FILTER.0, CSV_FILTER.1)
                .add_filter("All files", &["*"])
                .pick_file()
            {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let name = file_name(&path);
                        let _ = self.tx.send(FileEvent::CsvBytes { name, bytes });
                    }
                    Err(e) => log::error!("reading {}: {e}", path.display()),
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.pick_bytes(ctx, CSV_FILTER, |name, bytes| FileEvent::CsvBytes { name, bytes });
    }

    /// Asks where to save a project (native only; the web downloads instead).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pick_save_path(&self, default_name: &str) -> Option<std::path::PathBuf> {
        rfd::FileDialog::new()
            .add_filter(PROJECT_FILTER.0, PROJECT_FILTER.1)
            .set_file_name(format!("{default_name}.{}", survey_store::FILE_EXTENSION))
            .save_file()
            .map(|p| p.with_extension(survey_store::FILE_EXTENSION))
    }

    #[cfg(target_arch = "wasm32")]
    fn pick_bytes(
        &self,
        ctx: &egui::Context,
        filter: (&'static str, &'static [&'static str]),
        make: fn(String, Vec<u8>) -> FileEvent,
    ) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter(filter.0, filter.1)
                .pick_file()
                .await
            {
                let bytes = file.read().await;
                let _ = tx.send(make(file.file_name(), bytes));
                ctx.request_repaint();
            }
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Triggers a browser download of `bytes`.
#[cfg(target_arch = "wasm32")]
pub fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    use eframe::wasm_bindgen::JsCast as _;
    let err = |e: eframe::wasm_bindgen::JsValue| format!("{e:?}");
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&array);
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(err)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let anchor: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(err)?
        .dyn_into()
        .map_err(|_| "not an anchor".to_string())?;
    anchor.set_href(&url);
    anchor.set_download(name);
    anchor.click();
    web_sys::Url::revoke_object_url(&url).map_err(err)?;
    Ok(())
}
