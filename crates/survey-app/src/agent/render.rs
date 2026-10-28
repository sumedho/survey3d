//! SVG to PNG, with the bundled Inter font standing in for the plot's
//! Helvetica/Arial so renders look the same on every machine.

use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};

fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(egui_cad::fonts::INTER.to_vec());
        let family = db
            .faces()
            .next()
            .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
            .unwrap_or_else(|| "Inter".into());
        db.set_sans_serif_family(family);
        Arc::new(db)
    })
    .clone()
}

/// Renders `svg` `width` pixels wide (height to scale) on its own background.
pub fn svg_to_png(svg: &str, width: u32) -> Result<Vec<u8>, String> {
    let options = usvg::Options {
        fontdb: fonts(),
        font_family: "sans-serif".into(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| format!("Render failed: {e}"))?;
    let size = tree.size();
    let scale = width as f32 / size.width();
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("Render failed: image too large")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| format!("Render failed: {e}"))
}
