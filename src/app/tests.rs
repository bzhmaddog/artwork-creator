use super::dialogs::DIALOG_MARGIN;
use super::file::project_save_path;
use super::project_template::{
    compare_bar_body, problem_color, problems_tooltip, template_differs_body,
};
use super::*;

/// Draws the template selector with its list open, to look at it:
/// `TEMPLATE_SELECTOR_PREVIEW=out.png cargo test template_selector_preview -- --ignored`.
#[test]
#[ignore]
fn template_selector_preview() {
    let Ok(path) = std::env::var("TEMPLATE_SELECTOR_PREVIEW") else {
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut templates = template::scan(&dir);
    // One of each origin, to see both marks.
    if let Some(last) = templates.last_mut() {
        last.origin = template::Origin::User;
    }
    let chosen = templates.first().map(|t| t.id.clone());
    crate::ui::preview::render(&path, egui::vec2(320.0, 200.0), |ui| {
        ui.set_width(300.0);
        let (_, field) = template_selector(ui, "Cosmox - T16 (1.0)", &templates, &chosen);
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&field));
    });
}

/// Draws a template's problems as the Template check panel and the template list's
/// tooltip show them, to look at them:
/// `TEMPLATE_CHECK_PREVIEW=out.png cargo test template_check_preview -- --ignored`.
#[test]
#[ignore]
fn template_check_preview() {
    let Ok(path) = std::env::var("TEMPLATE_CHECK_PREVIEW") else {
        return;
    };
    let problem = |severity, text: &str, layer| Problem {
        severity,
        text: text.to_owned(),
        layer,
    };
    let problems = [
        problem(Severity::Warning, "Menu buttons: no shapes", None),
        problem(
            Severity::Error,
            "Action buttons: Circle 30.0 mm crosses the outline",
            Some(1),
        ),
        problem(
            Severity::Error,
            "Circle 24.0 mm (Action buttons) and Circle 30.0 mm (Action buttons) overlap",
            Some(2),
        ),
    ];
    crate::ui::preview::render(&path, egui::vec2(560.0, 260.0), |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(260.0);
                problem_list(ui, &problems, Some(1));
                ui.add_space(12.0);
                problem_list(ui, &[], None);
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(260.0);
                    problems_tooltip(ui, &problems);
                });
                ui.add_space(12.0);
                let mark = crate::ui::callout::warning_mark(problem_color(ui, Severity::Error));
                let _ = ui.add(egui::Button::selectable(
                    false,
                    (
                        "T16",
                        mark,
                        egui::Atom::grow(),
                        egui::RichText::new("1.2").weak(),
                    ),
                ));
                let mark = crate::ui::callout::warning_mark(problem_color(ui, Severity::Error));
                let _ = ui.add(egui::Button::new((mark, "Check")));
            });
        });
    });
}

/// Draws the bar shown while comparing templates, to look at it:
/// `COMPARE_BAR_PREVIEW=out.png cargo test compare_bar_preview -- --ignored`.
#[test]
#[ignore]
fn compare_bar_preview() {
    let Ok(path) = std::env::var("COMPARE_BAR_PREVIEW") else {
        return;
    };
    crate::ui::preview::render(&path, egui::vec2(520.0, 110.0), |ui| {
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            compare_bar_body(ui);
        });
    });
}

/// Draws the popup about a changed installed template, to look at it:
/// `TEMPLATE_DIFFERS_PREVIEW=out.png cargo test template_differs_preview -- --ignored`.
#[test]
#[ignore]
fn template_differs_preview() {
    let Ok(path) = std::env::var("TEMPLATE_DIFFERS_PREVIEW") else {
        return;
    };
    crate::ui::preview::render(&path, egui::vec2(520.0, 260.0), |ui| {
        egui::Frame::window(ui.style())
            .inner_margin(egui::Margin::same(DIALOG_MARGIN))
            .show(ui, |ui| {
                template_differs_body(ui, "Cosmox - T16", "1.0", "1.1")
            });
    });
}

#[test]
fn dialog_buttons_sit_right_without_widening_the_window() {
    let ctx = egui::Context::default();
    let mut found = Vec::new();
    for _ in 0..4 {
        let mut window = egui::Rect::NOTHING;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let shown = egui::Window::new("w")
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    padded(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_min_width(250.0);
                            ui.label("A row of settings, longer than the minimum width");
                            ui.add_space(10.0);
                            dialog_buttons(ui, "Export…", true);
                        })
                    })
                });
            window = shown.unwrap().response.rect;
        });
        out.textures_delta.clear();
        // Where the button texts were drawn: Cancel then Export, at the right edge.
        let mut texts: Vec<(String, f32)> = Vec::new();
        for clipped in &out.shapes {
            if let egui::Shape::Text(t) = &clipped.shape {
                texts.push((t.galley.text().to_owned(), t.pos.x + t.galley.size().x));
            }
        }
        found.push((window.width(), texts));
    }
    let (width, texts) = found.last().unwrap();
    assert_eq!(found[1].0, *width, "the window keeps its width");
    let x = |name: &str| texts.iter().find(|(t, _)| t == name).unwrap().1;
    assert!(x("Cancel") < x("Export…"));
    let content_right = texts
        .iter()
        .find(|(t, _)| t.starts_with("A row"))
        .unwrap()
        .1;
    assert!((x("Export…") - content_right).abs() < 12.0, "{texts:?}");
}

#[test]
fn a_dialog_with_three_buttons_keeps_the_main_one_right_and_one_apart_left() {
    let ctx = egui::Context::default();
    let mut found = Vec::new();
    for _ in 0..4 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            dialog(ui.ctx(), "t").show(ui.ctx(), |ui| {
                ui.set_max_width(420.0);
                ui.label("Save the changes before closing?");
                dialog_button_row(ui, |ui| {
                    let _ = ui.button("Save and close");
                    let _ = ui.button("Cancel");
                    at_left_edge(ui, |ui| ui.button("Close without saving"));
                });
            });
        });
        out.textures_delta.clear();
        let mut texts: Vec<(String, f32, f32)> = Vec::new();
        for clipped in &out.shapes {
            if let egui::Shape::Text(t) = &clipped.shape {
                texts.push((
                    t.galley.text().to_owned(),
                    t.pos.x,
                    t.pos.x + t.galley.size().x,
                ));
            }
        }
        found.push(texts);
    }
    let texts = found.last().unwrap();
    let at = |name: &str| {
        texts
            .iter()
            .find(|(t, ..)| t == name)
            .map(|(_, a, b)| ((), *a, *b))
            .unwrap()
    };
    let (_, label_left, label_right) = at("Save the changes before closing?");
    assert!(
        at("Save and close").1 > at("Cancel").1 && at("Cancel").1 > at("Close without saving").1
    );
    assert!(
        (at("Save and close").2 - label_right).abs() < 12.0,
        "Save at the right: {texts:?}"
    );
    assert!(
        (at("Close without saving").1 - label_left).abs() < 12.0,
        "Close without saving at the left"
    );
}

#[test]
fn project_save_path_adds_archive_extension() {
    let p = |s: &str| project_save_path(PathBuf::from(s));
    let ours = format!("/a/stick.{}", project::EXTENSION);
    assert_eq!(p(&ours), PathBuf::from(&ours));
    // A .json file is no longer a project: saved as an archive beside it.
    assert_eq!(
        p("/a/stick.json"),
        PathBuf::from(format!("/a/stick.json.{}", project::EXTENSION))
    );
    assert_eq!(
        p("/a/stick"),
        PathBuf::from(format!("/a/stick.{}", project::EXTENSION))
    );
    // A dot in the name is not an extension of ours.
    assert_eq!(
        p("/a/v1.2"),
        PathBuf::from(format!("/a/v1.2.{}", project::EXTENSION))
    );
}
