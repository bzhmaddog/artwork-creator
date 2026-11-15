// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod document;
mod export;
mod files;
mod i18n;
mod render;
mod template;
mod ui;

/// The app's icon, for its window's title bar and the taskbar or dock (the executable's
/// own on Windows is built in by `build.rs`).
fn window_icon() -> std::sync::Arc<egui::IconData> {
    const PNG: &[u8] = include_bytes!("../assets/icon/png/artwork-creator-256.png");
    let icon = image::load_from_memory_with_format(PNG, image::ImageFormat::Png)
        .map(|i| i.into_rgba8())
        .map(|i| egui::IconData {
            width: i.width(),
            height: i.height(),
            rgba: i.into_raw(),
        })
        .unwrap_or_default();
    std::sync::Arc::new(icon)
}

/// The app's id: names its data folder, where eframe keeps the settings, and the
/// `.desktop` file's name.
const APP_ID: &str = "artwork-creator";

/// The app's data folder, the one eframe keeps the settings in; it may not exist yet.
pub fn data_dir() -> Option<std::path::PathBuf> {
    eframe::storage_dir(APP_ID)
}

/// Finds a folder the app ships with (`templates`, `plugins`): next to the executable
/// first, then in `../share/artwork-creator/` from it (where the Linux packages install
/// it), then in the working directory.
pub fn shipped_dir(name: &str) -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(name));
        candidates.push(dir.join("../share").join(APP_ID).join(name));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(name));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

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
    render::effects::init(render::effects::plugin::plugins_dir().as_deref());

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Artwork Creator")
            // Matches the `.desktop` file's name, for Linux desktops to show its icon.
            .with_app_id(APP_ID)
            .with_icon(window_icon())
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
