mod app;
mod canvas;
mod graph;
mod project;

use app::OneNoteApp;
use std::path::PathBuf;

fn main() -> eframe::Result {
    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "OneNote Linux",
        options,
        Box::new(move |context| Ok(Box::new(OneNoteApp::new(context, initial_path.clone())))),
    )
}
