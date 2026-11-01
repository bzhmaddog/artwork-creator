//! System light/dark theme detection where the window system doesn't report it.
//!
//! On Windows, macOS and some Wayland desktops, egui follows the system theme by itself.
//! On Linux under X11 (and WSLg) it gets no answer and falls back to dark, so we ask the
//! desktop instead: the XDG desktop portal first, then GNOME's settings.

use egui::{Theme, ThemePreference};

/// Asks the desktop for its colour scheme in the background, and makes it egui's fallback
/// theme (used only while the window system reports none).
pub fn detect_in_background(ctx: &egui::Context) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        if let Some(theme) = desktop_theme() {
            log::info!("desktop colour scheme: {theme:?}");
            ctx.options_mut(|o| o.fallback_theme = theme);
            ctx.request_repaint();
        }
    });
}

/// The desktop's preferred theme, or `None` if it has no preference or can't be asked.
fn desktop_theme() -> Option<Theme> {
    let portal = run(
        "busctl",
        &[
            "--user",
            "--timeout=2",
            "call",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            "ReadOne",
            "ss",
            "org.freedesktop.appearance",
            "color-scheme",
        ],
    );
    if let Some(theme) = portal.as_deref().and_then(parse_portal) {
        return Some(theme);
    }
    let scheme = run(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "color-scheme"],
    );
    if let Some(theme) = scheme.as_deref().and_then(parse_gsettings_scheme) {
        return Some(theme);
    }
    // Older desktops only have a theme name, e.g. "Adwaita-dark".
    let gtk = run(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "gtk-theme"],
    )?;
    gtk.to_lowercase().contains("dark").then_some(Theme::Dark)
}

/// Output of a command that succeeded, or `None`.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parses busctl's answer for the portal's `color-scheme`, e.g. `v u 1`:
/// 1 is dark, 2 is light, 0 is no preference.
fn parse_portal(out: &str) -> Option<Theme> {
    match out.split_whitespace().last()? {
        "1" => Some(Theme::Dark),
        "2" => Some(Theme::Light),
        _ => None,
    }
}

/// Parses GNOME's `color-scheme` setting, e.g. `'prefer-dark'`.
fn parse_gsettings_scheme(out: &str) -> Option<Theme> {
    match out.trim().trim_matches('\'') {
        "prefer-dark" => Some(Theme::Dark),
        "prefer-light" => Some(Theme::Light),
        _ => None,
    }
}

/// View menu entries to choose System, Light or Dark.
pub fn menu(ui: &mut egui::Ui) {
    let mut pref = ui.ctx().options(|o| o.theme_preference);
    let before = pref;
    ui.radio_value(&mut pref, ThemePreference::System, "System")
        .on_hover_text("Follow the operating system's light or dark setting");
    ui.radio_value(&mut pref, ThemePreference::Light, "Light");
    ui.radio_value(&mut pref, ThemePreference::Dark, "Dark");
    if pref != before {
        ui.ctx().set_theme(pref);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_desktop_answers() {
        assert_eq!(parse_portal("v u 1\n"), Some(Theme::Dark));
        assert_eq!(parse_portal("v u 2"), Some(Theme::Light));
        assert_eq!(parse_portal("v u 0"), None);
        assert_eq!(parse_portal(""), None);
        assert_eq!(parse_gsettings_scheme("'prefer-dark'\n"), Some(Theme::Dark));
        assert_eq!(parse_gsettings_scheme("'prefer-light'"), Some(Theme::Light));
        assert_eq!(parse_gsettings_scheme("'default'"), None);
    }
}
