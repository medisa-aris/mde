//! The macOS menu bar.

use std::sync::mpsc::{Receiver, channel};

use egui::ThemePreference;
use muda::accelerator::Accelerator;
use muda::{AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::app::View;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Command {
    New,
    Open,
    Save,
    SaveAs,
    CloseTab,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    View(View),
    Theme(ThemePreference),
    Shortcuts,
}

/// Menu items are identified by their position in this table.
const COMMANDS: [(Command, &str, &str); 18] = [
    (Command::New, "New", "Cmd+N"),
    (Command::Open, "Open…", "Cmd+O"),
    (Command::Save, "Save", "Cmd+S"),
    (Command::SaveAs, "Save As…", "Cmd+Shift+S"),
    (Command::CloseTab, "Close Tab", "Cmd+W"),
    (Command::Quit, "Quit mde", "Cmd+Q"),
    (Command::Undo, "Undo", "Cmd+Z"),
    (Command::Redo, "Redo", "Cmd+Shift+Z"),
    (Command::Cut, "Cut", "Cmd+X"),
    (Command::Copy, "Copy", "Cmd+C"),
    (Command::Paste, "Paste", "Cmd+V"),
    (Command::SelectAll, "Select All", "Cmd+A"),
    (Command::View(View::Editor), "Editor", "Cmd+1"),
    (Command::View(View::Split), "Split", "Cmd+2"),
    (Command::View(View::Preview), "Preview", "Cmd+3"),
    (Command::Theme(ThemePreference::Light), "Light", ""),
    (Command::Theme(ThemePreference::Dark), "Dark", ""),
    (Command::Theme(ThemePreference::System), "System", ""),
];

fn entry(command: Command) -> (String, &'static str, Option<Accelerator>) {
    let (index, (_, title, keys)) = COMMANDS
        .iter()
        .enumerate()
        .find(|(_, (c, _, _))| *c == command)
        .unwrap_or((
            COMMANDS.len(),
            &(Command::Shortcuts, "Keyboard Shortcuts", ""),
        ));
    (index.to_string(), title, keys.parse().ok())
}

fn command(id: &str) -> Option<Command> {
    let index: usize = id.parse().ok()?;
    match COMMANDS.get(index) {
        Some((command, _, _)) => Some(*command),
        None => (index == COMMANDS.len()).then_some(Command::Shortcuts),
    }
}

fn item(command: Command) -> MenuItem {
    let (id, title, keys) = entry(command);
    MenuItem::with_id(id, title, true, keys)
}

fn check(command: Command) -> CheckMenuItem {
    let (id, title, keys) = entry(command);
    CheckMenuItem::with_id(id, title, true, false, keys)
}

fn separator() -> PredefinedMenuItem {
    PredefinedMenuItem::separator()
}

pub struct MenuBar {
    main: Menu,
    /// Shown while a system file dialog is open, whose text fields need the
    /// standard editing commands rather than ours.
    dialog: Menu,
    views: [CheckMenuItem; 3],
    themes: [CheckMenuItem; 3],
    checked: Option<(View, ThemePreference)>,
    commands: Receiver<Command>,
}

impl MenuBar {
    /// Builds and installs the menu bar. Must run on the main thread.
    pub fn install(ctx: &egui::Context) -> muda::Result<Self> {
        let (sender, commands) = channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(std::sync::Arc::new(move |event: MenuEvent| {
            if let Some(command) = command(event.id.as_ref()) {
                // The receiver only goes away when the app is closing.
                let _ = sender.send(command);
                ctx.request_repaint();
            }
        })));

        let about = AboutMetadata {
            name: Some("mde".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            ..Default::default()
        };
        let app = Submenu::with_items(
            "mde",
            true,
            &[
                &PredefinedMenuItem::about(Some("About mde"), Some(about)),
                &separator(),
                &PredefinedMenuItem::services(None),
                &separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &separator(),
                &item(Command::Quit),
            ],
        )?;
        let file = Submenu::with_items(
            "File",
            true,
            &[
                &item(Command::New),
                &item(Command::Open),
                &separator(),
                &item(Command::CloseTab),
                &item(Command::Save),
                &item(Command::SaveAs),
            ],
        )?;
        let edit = Submenu::with_items(
            "Edit",
            true,
            &[
                &item(Command::Undo),
                &item(Command::Redo),
                &separator(),
                &item(Command::Cut),
                &item(Command::Copy),
                &item(Command::Paste),
                &item(Command::SelectAll),
            ],
        )?;
        let views = [View::Editor, View::Split, View::Preview].map(|v| check(Command::View(v)));
        let themes = [
            ThemePreference::Light,
            ThemePreference::Dark,
            ThemePreference::System,
        ]
        .map(|t| check(Command::Theme(t)));
        let theme = Submenu::with_items("Theme", true, &[&themes[0], &themes[1], &themes[2]])?;
        let view = Submenu::with_items(
            "View",
            true,
            &[
                &views[0],
                &views[1],
                &views[2],
                &separator(),
                &theme,
                &separator(),
                &PredefinedMenuItem::fullscreen(None),
            ],
        )?;
        let window = Submenu::with_items(
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(Some("Zoom")),
                &separator(),
                &PredefinedMenuItem::bring_all_to_front(None),
            ],
        )?;
        let help = Submenu::with_items("Help", true, &[&item(Command::Shortcuts)])?;

        let main = Menu::new();
        main.append_items(&[&app, &file, &edit, &view, &window, &help])?;

        let dialog = Menu::new();
        dialog.append_items(&[
            &Submenu::with_items("mde", true, &[&PredefinedMenuItem::hide(None)])?,
            &Submenu::with_items(
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(None),
                    &PredefinedMenuItem::redo(None),
                    &separator(),
                    &PredefinedMenuItem::cut(None),
                    &PredefinedMenuItem::copy(None),
                    &PredefinedMenuItem::paste(None),
                    &PredefinedMenuItem::select_all(None),
                ],
            )?,
        ])?;

        main.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
        help.set_as_help_menu_for_nsapp();

        Ok(Self {
            main,
            dialog,
            views,
            themes,
            checked: None,
            commands,
        })
    }

    pub fn commands(&self) -> impl Iterator<Item = Command> + '_ {
        self.commands.try_iter()
    }

    /// Keeps the check marks in step with the app. A click toggles a check
    /// mark by itself, so `force` rewrites them after any menu command.
    pub fn sync(&mut self, view: View, theme: ThemePreference, force: bool) {
        if !force && self.checked == Some((view, theme)) {
            return;
        }
        self.checked = Some((view, theme));
        for (item, v) in self
            .views
            .iter()
            .zip([View::Editor, View::Split, View::Preview])
        {
            item.set_checked(v == view);
        }
        for (item, t) in self.themes.iter().zip([
            ThemePreference::Light,
            ThemePreference::Dark,
            ThemePreference::System,
        ]) {
            item.set_checked(t == theme);
        }
    }

    pub fn use_dialog_menu(&self, dialog: bool) {
        if dialog {
            self.dialog.init_for_nsapp();
        } else {
            self.main.init_for_nsapp();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_maps_back_to_its_command_and_has_a_valid_shortcut() {
        for (command_in, _, keys) in COMMANDS {
            let (id, _, accelerator) = entry(command_in);
            assert_eq!(command(&id), Some(command_in));
            assert_eq!(accelerator.is_some(), !keys.is_empty(), "{keys}");
        }
        let (id, _, _) = entry(Command::Shortcuts);
        assert_eq!(command(&id), Some(Command::Shortcuts));
    }
}
