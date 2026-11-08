// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas_points;
mod canvas_text;
mod clipboard;
mod colors;
mod document;
mod effects;
mod effects_ui;
mod export;
mod file_dialog;
mod history;
mod i18n;
mod layers_ui;
mod print;
mod printing;
mod project;
mod recent;
mod render;
mod shape;
mod template;
mod text;
mod theme;
mod variables;
mod view;

/// Starts the app, opening the project file given on the command line, if any.
fn main() -> eframe::Result {
    // RUST_LOG overrides this. The title bar drawn on Wayland asks the desktop for its
    // color scheme and logs an error where no one answers (e.g. WSLg); it then uses its
    // default, so that error is hidden.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("error,sctk_adwaita=off"),
    )
    .init();
    // A project file can be passed on the command line (or by double-clicking an associated file).
    let project = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    // Plugins are loaded once, before any effect is used.
    effects::init(effects::plugin::plugins_dir().as_deref());

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
