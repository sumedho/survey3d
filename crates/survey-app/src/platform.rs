//! File dialogs, downloads and browser storage, abstracted over native and web.
//!
//! Dialog results arrive asynchronously on the web, so everything is delivered
//! as [`FileEvent`]s polled once per frame.

use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Project,
    Points,
    Dxf,
}

impl FileKind {
    fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            FileKind::Project => ("survey3d project", &[survey_store::FILE_EXTENSION]),
            FileKind::Points => ("Point files", &["csv", "txt", "pnt", "xyz"]),
            FileKind::Dxf => ("DXF drawing", &["dxf"]),
        }
    }
}

pub enum FileEvent {
    /// A file chosen to open/import, as bytes.
    Picked {
        kind: FileKind,
        name: String,
        bytes: Vec<u8>,
    },
    /// A project chosen to open on native (opened in place, not copied).
    #[cfg(not(target_arch = "wasm32"))]
    ProjectPath(std::path::PathBuf),
    /// A crash-recovery snapshot found at startup (web).
    #[cfg(target_arch = "wasm32")]
    /// `None` when there was nothing to recover.
    Recovery { bytes: Option<Vec<u8>> },
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

impl Platform {
    pub fn poll(&self) -> Vec<FileEvent> {
        self.rx.try_iter().collect()
    }

    /// Shows an open dialog; the result arrives as a [`FileEvent`].
    pub fn pick_file(&self, ctx: &egui::Context, kind: FileKind) {
        let (label, exts) = kind.filter();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let Some(path) = rfd::FileDialog::new()
                .add_filter(label, exts)
                .add_filter("All files", &["*"])
                .pick_file()
            else {
                return;
            };
            if kind == FileKind::Project {
                let _ = self.tx.send(FileEvent::ProjectPath(path));
                return;
            }
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let name = file_name(&path);
                    let _ = self.tx.send(FileEvent::Picked { kind, name, bytes });
                }
                Err(e) => log::error!("reading {}: {e}", path.display()),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(file) = rfd::AsyncFileDialog::new()
                    .add_filter(label, exts)
                    .pick_file()
                    .await
                {
                    let bytes = file.read().await;
                    let _ = tx.send(FileEvent::Picked {
                        kind,
                        name: file.file_name(),
                        bytes,
                    });
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Asks where to save (native only; the web downloads instead).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pick_save_path(&self, kind: FileKind, default_stem: &str) -> Option<std::path::PathBuf> {
        let (label, exts) = kind.filter();
        let ext = exts[0];
        rfd::FileDialog::new()
            .add_filter(label, exts)
            .set_file_name(format!("{default_stem}.{ext}"))
            .save_file()
            .map(|p| {
                if p.extension().is_none() {
                    p.with_extension(ext)
                } else {
                    p
                }
            })
    }

    /// Looks for a crash-recovery snapshot (web); found snapshots arrive as
    /// [`FileEvent::Recovery`].
    #[cfg(target_arch = "wasm32")]
    pub fn check_recovery(&self, ctx: &egui::Context) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let bytes = opfs::read(opfs::AUTOSAVE).await.ok().flatten();
            let _ = tx.send(FileEvent::Recovery { bytes });
            ctx.request_repaint();
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

/// Writes `bytes` to a file chosen by the user (native) or downloads it (web).
pub fn export(
    platform: &Platform,
    kind: FileKind,
    default_stem: &str,
    bytes: &[u8],
) -> Result<Option<String>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = platform.pick_save_path(kind, default_stem) else {
            return Ok(None);
        };
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(Some(path.display().to_string()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = platform;
        let name = format!("{default_stem}.{}", kind.filter().1[0]);
        download(&name, bytes)?;
        Ok(Some(name))
    }
}

/// Origin-private file system storage for crash-recovery snapshots (web).
#[cfg(target_arch = "wasm32")]
pub mod opfs {
    use eframe::wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetFileOptions,
        FileSystemWritableFileStream,
    };

    pub const AUTOSAVE: &str = "autosave.s3d";

    async fn root() -> Result<FileSystemDirectoryHandle, JsValue> {
        let storage = web_sys::window().ok_or("no window")?.navigator().storage();
        JsFuture::from(storage.get_directory()).await?.dyn_into()
    }

    pub async fn write(name: &str, bytes: &[u8]) -> Result<(), JsValue> {
        let options = FileSystemGetFileOptions::new();
        options.set_create(true);
        let handle: FileSystemFileHandle =
            JsFuture::from(root().await?.get_file_handle_with_options(name, &options))
                .await?
                .dyn_into()?;
        let stream: FileSystemWritableFileStream =
            JsFuture::from(handle.create_writable()).await?.dyn_into()?;
        JsFuture::from(stream.write_with_u8_array(bytes)?).await?;
        JsFuture::from(stream.close()).await?;
        Ok(())
    }

    /// `Ok(None)` when the file does not exist.
    pub async fn read(name: &str) -> Result<Option<Vec<u8>>, JsValue> {
        let Ok(handle) = JsFuture::from(root().await?.get_file_handle(name)).await else {
            return Ok(None);
        };
        let handle: FileSystemFileHandle = handle.dyn_into()?;
        let file: web_sys::File = JsFuture::from(handle.get_file()).await?.dyn_into()?;
        let buffer = JsFuture::from(file.array_buffer()).await?;
        Ok(Some(js_sys::Uint8Array::new(&buffer).to_vec()))
    }

    pub async fn remove(name: &str) -> Result<(), JsValue> {
        JsFuture::from(root().await?.remove_entry(name)).await?;
        Ok(())
    }
}
