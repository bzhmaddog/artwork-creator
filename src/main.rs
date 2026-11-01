// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod document;
mod export;
mod file_dialog;
mod history;
mod layers_ui;
mod recent;
mod render;
mod template;
mod theme;
mod view;

/// Starts the app, opening the project file given on the command line, if any.
fn main() -> eframe::Result {
    env_logger::init();
    // A project file can be passed on the command line (or by double-clicking an associated file).
    let project = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Artwork Creator")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "Artwork Creator",
        options,
        Box::new(|cc| Ok(Box::new(app::ArtworkApp::new(cc, project)))),
    )
}
