//! Light / Dark / System preference.

use egui::{Color32, CornerRadius, FontFamily, FontId, TextStyle, ThemePreference};

const KEY: &str = "theme";

pub fn load(storage: Option<&dyn eframe::Storage>) -> ThemePreference {
    let name = storage.and_then(|s| s.get_string(KEY));
    match name.as_deref() {
        Some("light") => ThemePreference::Light,
        Some("dark") => ThemePreference::Dark,
        _ => ThemePreference::System,
    }
}

pub fn save(storage: &mut dyn eframe::Storage, preference: ThemePreference) {
    storage.set_string(KEY, label(preference).to_lowercase());
}

/// Colors of the header and tab bar.
pub struct Chrome {
    pub header: Color32,
    /// Behind the segmented control and the tabs.
    pub track: Color32,
    /// The selected segment, and hovered buttons.
    pub raised: Color32,
    pub line: Color32,
    pub text: Color32,
    pub dim: Color32,
}

impl Chrome {
    pub fn new(dark: bool) -> Self {
        let gray = Color32::from_gray;
        if dark {
            Self {
                header: gray(50),
                track: gray(36),
                raised: gray(94),
                line: gray(18),
                text: gray(230),
                dim: gray(160),
            }
        } else {
            Self {
                header: gray(246),
                track: gray(225),
                raised: gray(255),
                line: gray(205),
                text: gray(30),
                dim: gray(100),
            }
        }
    }
}

/// The system fonts macOS apps use, ahead of egui's bundled ones, which stay
/// as fallbacks. A missing file just leaves the bundled font in place.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (path, family) in [
        ("/System/Library/Fonts/SFNS.ttf", FontFamily::Proportional),
        ("/System/Library/Fonts/SFNSMono.ttf", FontFamily::Monospace),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(path.into(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, path.into());
        }
    }
    ctx.set_fonts(fonts);
}

/// macOS-like colors and shapes, with type sized for reading.
pub fn apply_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        for (name, font) in [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Heading, FontId::proportional(28.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ] {
            style.text_styles.insert(name, font);
        }
        let dark = style.visuals.dark_mode;
        let gray = Color32::from_gray;
        let visuals = &mut style.visuals;
        visuals.panel_fill = if dark { gray(30) } else { gray(255) };
        visuals.window_fill = if dark { gray(44) } else { gray(246) };
        visuals.widgets.noninteractive.fg_stroke.color = if dark { gray(220) } else { gray(30) };
        visuals.selection.bg_fill = if dark {
            Color32::from_rgb(63, 99, 139)
        } else {
            Color32::from_rgb(179, 215, 255)
        };
        visuals.selection.stroke.color = if dark { gray(255) } else { gray(0) };
        visuals.hyperlink_color = if dark {
            Color32::from_rgb(65, 156, 255)
        } else {
            Color32::from_rgb(0, 104, 218)
        };
        visuals.text_cursor.stroke.color = Color32::from_rgb(0, 122, 255);
        let radius = CornerRadius::same(6);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = radius;
        }
        visuals.window_corner_radius = CornerRadius::same(10);
        visuals.menu_corner_radius = CornerRadius::same(8);
    });
}

pub fn label(preference: ThemePreference) -> &'static str {
    match preference {
        ThemePreference::Light => "Light",
        ThemePreference::Dark => "Dark",
        ThemePreference::System => "System",
    }
}

pub fn menu(ui: &mut egui::Ui, preference: &mut ThemePreference) {
    ui.menu_button(label(*preference), |ui| {
        for choice in [
            ThemePreference::Light,
            ThemePreference::Dark,
            ThemePreference::System,
        ] {
            if ui.radio_value(preference, choice, label(choice)).clicked() {
                ui.ctx().set_theme(choice);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Theme");
}
