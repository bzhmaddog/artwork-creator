//! The widget gallery: one of each widget the app uses, drawn to `docs/widgets.png`.
//! `cargo test widget_gallery -- --ignored` draws it again.

use egui::{Ui, vec2};

use crate::document::gradient::Gradient;
use crate::document::variables::{Bindings, Field, Kind};
use crate::ui::callout::{self, Tone};
use crate::ui::{colors, dial, gradient, layers, text_links, variables};

/// A titled row of the gallery.
fn row(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            [150.0, 18.0],
            egui::Label::new(egui::RichText::new(title).weak()),
        );
        ui.vertical(add);
    });
    ui.add_space(6.0);
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(4.0);
    ui.strong(title);
    ui.separator();
}

#[test]
#[ignore = "draws docs/widgets.png"]
fn widget_gallery() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/widgets.png");
    crate::ui::preview::render(path, vec2(640.0, 1080.0), |ui| {
        layers::field_frames(ui);

        section(ui, "egui widgets, as the app uses them");
        row(ui, "Button, selectable", |ui| {
            ui.horizontal(|ui| {
                let _ = ui.button("Button");
                ui.add_enabled(false, egui::Button::new("Disabled"));
                let _ = ui.selectable_label(true, "Selected");
                let _ = ui.selectable_label(false, "Not selected");
            });
        });
        row(ui, "Checkbox, radio", |ui| {
            ui.horizontal(|ui| {
                let (mut on, mut off) = (true, false);
                ui.checkbox(&mut on, "On");
                ui.checkbox(&mut off, "Off");
                let _ = ui.radio(true, "Picked");
                let _ = ui.radio(false, "Not picked");
            });
        });
        row(ui, "Number, unit, slider", |ui| {
            ui.horizontal(|ui| {
                let (mut mm, mut k) = (30.0f32, 0.4f32);
                ui.add(egui::DragValue::new(&mut mm).fixed_decimals(1));
                layers::unit(ui, "mm");
                ui.add(egui::Slider::new(&mut k, 0.0..=1.0).fixed_decimals(2));
            });
        });
        row(ui, "Combo box, text", |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("gallery_combo")
                    .selected_text("Landscape")
                    .show_ui(ui, |_| {});
                let mut text = "Some text".to_owned();
                ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0));
            });
        });
        row(ui, "Collapsing section", |ui| {
            egui::CollapsingHeader::new("Section")
                .default_open(false)
                .show(ui, |_| {});
        });

        section(ui, "Numbers and angles");
        row(ui, "Field (takes formulas)", |ui| {
            ui.horizontal(|ui| {
                let mut bindings = Bindings::new();
                let mut v = 12.5;
                variables::field(ui, &mut bindings, Field::Width, &mut v, |d| {
                    d.fixed_decimals(1)
                });
                layers::unit(ui, "mm");
                let mut bound = Bindings::new();
                bound.insert(Field::Height, "buttonSize / 2".into());
                let mut h = 15.0;
                variables::field(ui, &mut bound, Field::Height, &mut h, |d| {
                    d.fixed_decimals(1)
                });
                layers::unit(ui, "mm");
            });
        });
        row(ui, "Prefix (input group)", |ui| {
            ui.horizontal(|ui| {
                let (mut x, mut y) = (148.5f32, 105.0f32);
                layers::prefixed(ui, "x", |ui| {
                    ui.add(egui::DragValue::new(&mut x).fixed_decimals(1))
                });
                let mut bindings = Bindings::new();
                layers::prefixed(ui, "y", |ui| {
                    variables::field(ui, &mut bindings, Field::Y, &mut y, |d| d.fixed_decimals(1))
                });
                layers::unit(ui, "mm");
            });
        });
        row(ui, "Angle dial", |ui| {
            ui.horizontal(|ui| {
                let (mut ccw, mut cw) = (135.0, 30.0);
                dial::angle_dial(ui, &mut ccw, dial::Turn::FromRight, 0.0);
                ui.weak("135° from the right, counterclockwise");
                dial::angle_dial(ui, &mut cw, dial::Turn::FromTop, 0.0);
                ui.weak("30° from the top, clockwise");
            });
        });

        section(ui, "Colors");
        row(ui, "Color button", |ui| {
            ui.horizontal(|ui| {
                let mut c = [220, 20, 60, 255];
                colors::button(ui, &mut c, true, None);
                let mut white = [255, 255, 255, 255];
                colors::button(ui, &mut white, true, None);
                let mut half = [0, 120, 255, 128];
                colors::button(ui, &mut half, true, None);
                let mut linked = Some("red".to_owned());
                colors::button(ui, &mut c, true, Some(&mut linked));
                ui.weak("plain, white, see-through, linked");
            });
        });
        row(ui, "Swatches", |ui| {
            ui.horizontal(|ui| {
                colors::swatch(ui, [255, 200, 0, 255], false, false);
                colors::swatch(ui, [255, 200, 0, 255], true, false);
                colors::swatch(ui, [255, 200, 0, 255], false, true);
                ui.weak("plain, selected, linked");
            });
        });
        row(ui, "Gradient", |ui| {
            let mut color = [220, 20, 60, 255];
            let mut link = None;
            let mut g = Some(Gradient {
                angle: 45.0,
                ..Gradient::from_color(color)
            });
            let mut fill = gradient::Fill {
                color: &mut color,
                link: &mut link,
                gradient: &mut g,
                pattern: None,
                generator: None,
                units_per_mm: 1.0,
                alpha: true,
                pick_pattern: false,
            };
            ui.horizontal(|ui| gradient::inline(ui, "gallery", &mut fill));
            ui.set_max_width(300.0);
            gradient::rows(ui, "gallery", &mut fill);
        });

        section(ui, "Variables and links");
        row(ui, "Variable kinds", |ui| {
            ui.horizontal(|ui| {
                for kind in Kind::ALL {
                    variables::kind_icon(ui, kind);
                }
            });
        });
        row(ui, "Links", |ui| {
            ui.horizontal(|ui| {
                let mut link = None;
                text_links::link_button(ui, &mut link);
                ui.weak("text link");
                variables::broken_links_badge(ui, &["gone".to_owned()]);
                ui.weak("broken link");
            });
        });

        section(ui, "Layers panel");
        row(ui, "Icons, tools, text style", |ui| {
            layers::sample_widgets(ui);
        });

        section(ui, "Messages");
        row(ui, "Callouts", |ui| {
            ui.set_max_width(440.0);
            callout::callout(ui, Tone::Info, "Info: an explanation or an empty state.");
            callout::callout(ui, Tone::Tip, "Tip: how to do something.");
            callout::callout(ui, Tone::Warning, "Warning: something to look out for.");
            callout::callout_small(ui, Tone::Warning, "A small one, under a field.");
        });
        row(ui, "Warning marks", |ui| {
            ui.horizontal(|ui| {
                callout::warning_icon(ui, ui.visuals().warn_fg_color);
                callout::warning_icon(ui, ui.visuals().error_fg_color);
                ui.weak("warning, error");
            });
        });
        row(ui, "Dialog buttons", |ui| {
            // At the right of a dialog's bottom row, as wide as what is above it.
            ui.weak("The dialog's contents, above its buttons.");
            crate::app::dialog_buttons(ui, "OK", true);
        });
    });
}
