//! The window header and tab bar, drawn to match macOS. The system title bar
//! is hidden, so the header sits behind the window's traffic light buttons.

use egui::{Align2, CornerRadius, Rect, Sense, TextStyle, ThemePreference, pos2, vec2};

use crate::app::View;
use crate::theme;

pub const HEADER_HEIGHT: f32 = 38.0;
const TAB_HEIGHT: f32 = 28.0;
/// Room taken by the traffic light buttons.
const TRAFFIC_LIGHTS: f32 = 78.0;
const SEGMENT: egui::Vec2 = vec2(72.0, 22.0);

pub struct Tab {
    pub id: u64,
    pub title: String,
    pub dirty: bool,
    pub path: Option<String>,
}

pub enum TabAction {
    Select(usize),
    Close(u64),
    New,
}

/// Text on one line, cut short with an ellipsis to fit `width`.
fn label(
    ui: &egui::Ui,
    text: String,
    width: f32,
    color: egui::Color32,
) -> std::sync::Arc<egui::Galley> {
    let font = TextStyle::Button.resolve(ui.style());
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.fonts_mut(|f| f.layout_job(job))
}

pub fn header(
    ui: &mut egui::Ui,
    title: &str,
    dirty: bool,
    view: &mut View,
    preference: &mut ThemePreference,
) {
    let colors = theme::Chrome::new(ui.visuals().dark_mode);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, colors.header);

    // Empty header space moves the window, like a title bar. Added before the
    // controls so that they take precedence.
    let drag = ui.interact(
        rect,
        ui.scope_id().with("title-bar"),
        Sense::click_and_drag(),
    );
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }

    let segments = Rect::from_center_size(rect.center(), vec2(SEGMENT.x * 3.0, SEGMENT.y));
    ui.painter()
        .rect_filled(segments.expand(2.0), CornerRadius::same(7), colors.track);
    for (i, (choice, name, _)) in View::ALL.into_iter().enumerate() {
        let area = Rect::from_min_size(segments.min + vec2(SEGMENT.x * i as f32, 0.0), SEGMENT);
        let selected = *view == choice;
        let response = ui
            .interact(area, ui.scope_id().with(("segment", i)), Sense::click())
            .on_hover_text(format!("Cmd+{}", i + 1));
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, name)
        });
        if response.clicked() {
            *view = choice;
        }
        if selected {
            ui.painter()
                .rect_filled(area, CornerRadius::same(5), colors.raised);
        }
        let color = if selected { colors.text } else { colors.dim };
        let galley = label(ui, name.to_owned(), SEGMENT.x - 6.0, color);
        let at = area.center() - galley.size() / 2.0;
        ui.painter().galley(at, galley, color);
    }

    let fullscreen = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
    let left = rect.left() + if fullscreen { 12.0 } else { TRAFFIC_LIGHTS };
    let width = segments.left() - left - 12.0;
    if width > 40.0 {
        let text = if dirty {
            format!("{title} — Edited")
        } else {
            title.to_owned()
        };
        let galley = label(ui, text, width, colors.text);
        let at = pos2(left, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, colors.text);
    }

    let right = Rect::from_min_max(
        pos2(segments.right() + 8.0, rect.top()),
        pos2(rect.right() - 8.0, rect.bottom()),
    );
    let builder = egui::UiBuilder::new()
        .max_rect(right)
        .layout(egui::Layout::right_to_left(egui::Align::Center));
    theme::menu(&mut ui.new_child(builder), preference);

    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, colors.line),
    );
}

/// Equal-width tabs across the window, with a button for a new one.
pub fn tab_bar(ui: &mut egui::Ui, tabs: &[Tab], active: usize) -> Option<TabAction> {
    let colors = theme::Chrome::new(ui.visuals().dark_mode);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), TAB_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, colors.track);
    let mut action = None;

    let plus = Rect::from_min_max(pos2(rect.right() - TAB_HEIGHT, rect.top()), rect.max);
    let width = (plus.left() - rect.left()) / tabs.len().max(1) as f32;
    let line = egui::Stroke::new(1.0, colors.line);

    for (i, tab) in tabs.iter().enumerate() {
        let area = Rect::from_min_size(
            rect.min + vec2(width * i as f32, 0.0),
            vec2(width, TAB_HEIGHT),
        );
        let selected = i == active;
        let id = ui.scope_id().with(("tab", tab.id));
        let mut response = ui.interact(area, id, Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Tab, true, selected, &tab.title)
        });
        let hovered = ui.rect_contains_pointer(area);
        if selected {
            ui.painter().rect_filled(area, 0.0, colors.header);
        } else if hovered {
            ui.painter()
                .rect_filled(area, 0.0, colors.raised.gamma_multiply(0.4));
        }
        ui.painter().vline(area.right() - 0.5, area.y_range(), line);

        // Room is kept for the close button so the title does not shift.
        let color = if selected { colors.text } else { colors.dim };
        let title = if tab.dirty {
            format!("• {}", tab.title)
        } else {
            tab.title.clone()
        };
        let galley = label(ui, title, width - 2.0 * TAB_HEIGHT, color);
        let at = area.center() - galley.size() / 2.0;
        ui.painter().galley(at, galley, color);

        if let Some(path) = &tab.path {
            response = response.on_hover_text(path);
        }
        if response.clicked() {
            action = Some(TabAction::Select(i));
        }
        if response.middle_clicked() {
            action = Some(TabAction::Close(tab.id));
        }
        let close = Rect::from_center_size(
            pos2(area.left() + TAB_HEIGHT / 2.0 + 2.0, area.center().y),
            vec2(18.0, 18.0),
        );
        let button = ui.interact(close, id.with("close"), Sense::click());
        button.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Close {}", tab.title))
        });
        if hovered {
            if button.hovered() {
                ui.painter()
                    .rect_filled(close, CornerRadius::same(4), colors.raised);
            }
            ui.painter().text(
                close.center(),
                Align2::CENTER_CENTER,
                "×",
                TextStyle::Body.resolve(ui.style()),
                colors.text,
            );
        }
        if button.clicked() {
            action = Some(TabAction::Close(tab.id));
        }
    }

    let button = ui
        .interact(plus, ui.scope_id().with("new-tab"), Sense::click())
        .on_hover_text("New (Cmd+N)");
    button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "New tab"));
    if button.hovered() {
        ui.painter().rect_filled(plus, 0.0, colors.raised);
    }
    ui.painter().text(
        plus.center(),
        Align2::CENTER_CENTER,
        "+",
        egui::FontId::proportional(18.0),
        colors.dim,
    );
    if button.clicked() {
        action = Some(TabAction::New);
    }
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, line);
    action
}
