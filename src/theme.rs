//! Light / Dark / System preference.

use egui::ThemePreference;

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

/// Larger type and stronger contrast than egui's defaults, for reading.
pub fn apply_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        use egui::{FontId, TextStyle};
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(28.0));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(13.5));
        let text = if style.visuals.dark_mode {
            egui::Color32::from_gray(205)
        } else {
            egui::Color32::from_gray(35)
        };
        style.visuals.widgets.noninteractive.fg_stroke.color = text;
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
    ui.menu_button(format!("Theme: {}", label(*preference)), |ui| {
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
    });
}
