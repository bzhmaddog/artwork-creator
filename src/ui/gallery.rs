//! The widget gallery: one of each widget the app uses, drawn to `docs/widgets.png`,
//! and the design tokens they are made with (`ui::tokens`), drawn to `docs/tokens.png`.
//! `cargo test widget_gallery -- --ignored` and `cargo test ui_tokens -- --ignored`
//! draw them again.

use egui::{Ui, vec2};

use crate::document::gradient::Gradient;
use crate::document::variables::{Bindings, Field, Kind};
use crate::ui::callout::{self, Tone};
use crate::ui::{colors, dial, gradient, layers, text_links, tokens, variables};

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

/// The theme's colors the widgets use, by name, as `docs/tokens.png` shows them.
fn theme_colors(v: &egui::Visuals) -> [(&'static str, egui::Color32); 10] {
    [
        ("text", v.text_color()),
        ("weak text", v.weak_text_color()),
        ("panel", v.panel_fill),
        ("field", v.extreme_bg_color),
        ("widget", v.widgets.inactive.weak_bg_fill),
        ("hover", v.widgets.hovered.weak_bg_fill),
        ("selection", v.selection.bg_fill),
        ("info / link", v.hyperlink_color),
        ("warning", v.warn_fg_color),
        ("error", v.error_fg_color),
    ]
}

/// A filled square with a frame, `side` wide.
fn sample(ui: &mut Ui, side: f32, fill: egui::Color32, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), egui::Sense::hover());
    let frame = ui.visuals().widgets.noninteractive.bg_stroke;
    ui.painter()
        .rect(rect, radius, fill, frame, egui::StrokeKind::Inside);
}

/// The rows of a tokens section, slightly indented.
fn block(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.add_space(12.0);
        ui.vertical(add);
    });
    ui.add_space(6.0);
}

/// A color token: its swatch, name and hex value (with its opacity when not opaque).
fn color_row(ui: &mut Ui, name: &str, c: egui::Color32) {
    ui.horizontal(|ui| {
        sample(ui, 18.0, c, tokens::RADIUS);
        ui.label(name);
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        let alpha = if a < 255 {
            format!(", {:.0}% opacity", f32::from(a) / 2.55)
        } else {
            String::new()
        };
        ui.weak(format!("#{r:02X}{g:02X}{b:02X}{alpha}"));
    });
}

/// A length token: a bar that long, its name and value.
fn length_row(ui: &mut Ui, name: &str, len: f32) {
    ui.horizontal(|ui| {
        // At its true length, in a column as wide as the longest.
        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 12.0), egui::Sense::hover());
        let bar = egui::Rect::from_min_size(rect.min, vec2(len, rect.height()));
        ui.painter()
            .rect_filled(bar, 0.0, ui.visuals().selection.bg_fill);
        ui.label(name);
        ui.weak(format!("{len} pt"));
    });
}

#[test]
#[ignore = "draws docs/tokens.png"]
fn ui_tokens() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/tokens.png");
    crate::ui::preview::render(path, vec2(400.0, 1000.0), |ui| {
        section(ui, "Theme colors (egui Visuals)");
        block(ui, |ui| {
            for (name, c) in theme_colors(ui.visuals()) {
                color_row(ui, name, c);
            }
        });

        section(ui, "Fixed colors");
        block(ui, |ui| {
            color_row(ui, "DANGER", tokens::DANGER);
            color_row(ui, "ACCENT", tokens::ACCENT);
            color_row(ui, "GUIDE", tokens::GUIDE);
            color_row(ui, "SAFE_AREA", tokens::SAFE_AREA);
            color_row(ui, "tip (TIP_DARK / TIP_LIGHT)", tokens::tip(ui.visuals()));
        });

        section(ui, "Sizes");
        block(ui, |ui| {
            ui.horizontal(|ui| {
                let fill = ui.visuals().widgets.inactive.weak_bg_fill;
                for (name, side) in [
                    ("ICON_TOOL", tokens::ICON_TOOL),
                    ("ICON_PANEL", tokens::ICON_PANEL),
                    ("ICON_ROW", tokens::ICON_ROW),
                    ("DIAL", tokens::DIAL),
                    ("THUMB", tokens::THUMB),
                ] {
                    ui.vertical(|ui| {
                        sample(ui, side, fill, 0.0);
                        ui.weak(format!("{name}\n{side} pt"));
                    });
                }
            });
        });

        section(ui, "Corner radii");
        block(ui, |ui| {
            ui.horizontal(|ui| {
                let fill = ui.visuals().widgets.inactive.weak_bg_fill;
                for (name, r) in [
                    ("RADIUS", tokens::RADIUS),
                    ("RADIUS_CALLOUT", f32::from(tokens::RADIUS_CALLOUT)),
                ] {
                    sample(ui, 32.0, fill, r);
                    ui.weak(format!("{name}, {r} pt"));
                }
            });
        });

        section(ui, "Spacing");
        block(ui, |ui| {
            length_row(ui, "DIALOG_MARGIN", f32::from(tokens::DIALOG_MARGIN));
            length_row(ui, "LAYER_INDENT", tokens::LAYER_INDENT);
            length_row(ui, "COLUMN_GAP", tokens::COLUMN_GAP);
            length_row(ui, "INDENT", tokens::INDENT);
            length_row(ui, "PROP_ROW_GAP", tokens::PROP_ROW_GAP);
            length_row(ui, "PANEL_ROW_GAP", tokens::PANEL_ROW_GAP);
        });

        section(ui, "Strokes");
        block(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(60.0, 12.0), egui::Sense::hover());
                let c = ui.visuals().text_color();
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    egui::Stroke::new(tokens::STROKE, c),
                );
                ui.label("STROKE");
                ui.weak(format!("{} pt", tokens::STROKE));
            });
        });

        section(ui, "Text styles (egui)");
        block(ui, |ui| {
            for style in [
                egui::TextStyle::Heading,
                egui::TextStyle::Body,
                egui::TextStyle::Button,
                egui::TextStyle::Monospace,
                egui::TextStyle::Small,
            ] {
                let size = style.resolve(ui.style()).size;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{style:?}")).text_style(style));
                    ui.weak(format!("{size} pt"));
                });
            }
        });
    });
}

#[test]
#[ignore = "draws docs/widgets.png"]
fn widget_gallery() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/widgets.png");
    crate::ui::preview::render(path, vec2(640.0, 1480.0), |ui| {
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
        row(ui, "Delete or discard", |ui| {
            ui.horizontal(|ui| {
                let _ = ui.add(crate::app::danger_button("Delete"));
                ui.add_enabled(false, crate::app::danger_button("Disabled"));
            });
        });
        row(ui, "Icon buttons (flat)", |ui| {
            ui.horizontal(|ui| {
                let _ = layers::tool_icon(ui, layers::Tool::Add, true, "Add");
                let _ = layers::tool_icon(ui, layers::Tool::Delete, true, "Delete");
                let _ = layers::tool_icon(ui, layers::Tool::Delete, false, "Disabled");
                if let Some(g) = crate::render::effects::registry().generator("builtin.noise") {
                    let mut inst = crate::render::effects::EffectInstance::generator(g);
                    crate::ui::effects::regenerate_button(ui, &mut inst);
                }
                ui.weak("toolbar, disabled, next to a field");
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
        row(ui, "Toasts", |ui| {
            use crate::ui::toast::{Intent, Toast, card};
            for (intent, title, text, actions) in [
                (
                    Intent::Info,
                    "Plugin updates",
                    "A new version of 1 plugin is available. Review it in the plugin manager?",
                    vec!["Review".to_owned()],
                ),
                (Intent::Success, "Success", "Something done.", vec![]),
                (
                    Intent::Warning,
                    "Warning",
                    "Something to look out for.",
                    vec![],
                ),
                (Intent::Error, "Error", "Something that failed.", vec![]),
            ] {
                let toast = Toast {
                    key: "gallery",
                    intent,
                    title: title.into(),
                    text: text.into(),
                    actions,
                };
                card(ui, &toast);
            }
        });
        row(ui, "Callouts", |ui| {
            ui.set_max_width(440.0);
            callout::callout(ui, Tone::Info, "Info: an explanation or an empty state.");
            callout::callout(ui, Tone::Tip, "Tip: how to do something.");
            callout::callout(ui, Tone::Warning, "Warning: something to look out for.");
            callout::callout(ui, Tone::Error, "Error: something wrong.");
            callout::callout_with(ui, Tone::Error, "With a button at its end.", |ui| {
                let _ = ui.button("Install");
            });
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
            crate::app::dialog_button_row(ui, |ui| {
                let _ = ui.button("OK");
                let _ = ui.button("Cancel");
                crate::app::at_left_edge(ui, |ui| ui.add(crate::app::danger_button("Don't save")));
            });
        });
    });
}
