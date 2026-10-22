#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! survey3d — surveying CAD for cadastral, construction and mining work.

mod app;
mod cloud_view;
mod field_to_finish;
mod import_command;
mod interchange;
mod listing;
mod platform;
mod project;
mod scene_build;
mod settings;
mod surface_view;
mod tools;
mod ui;

use app::SurveyApp;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("survey3d")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "survey3d",
        options,
        Box::new(|cc| Ok(Box::new(SurveyApp::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().and_then(|w| w.document()).expect("no document");
        let canvas = document
            .get_element_by_id("survey3d_canvas")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("missing canvas #survey3d_canvas");
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(SurveyApp::new(cc)))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(()) => loading.remove(),
                Err(e) => {
                    loading.set_inner_html("<p>survey3d failed to start. See the developer console.</p>");
                    panic!("failed to start survey3d: {e:?}");
                }
            }
        }
    });
}
