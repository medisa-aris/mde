//! Window layout, tabs, shortcuts and dialogs.

use std::path::PathBuf;
use std::time::Instant;

use egui::{Key, KeyboardShortcut, Modifiers, ThemePreference};
use egui_commonmark::CommonMarkCache;

use crate::chrome;
use crate::document::Document;
use crate::editor::Editor;
use crate::menu::{Command, MenuBar};
use crate::preview::mermaid::Diagrams;
use crate::preview::view::Preview;
use crate::theme;
use crate::worker::Worker;

/// Seconds after the last scroll input during which the panes keep syncing.
const SCROLL_SETTLE: f64 = 0.3;

const SHORTCUTS: &str = "\
Cmd+1, 2, 3 — Editor, Split, Preview
Cmd+N, O — New, Open
Cmd+S, Shift+Cmd+S — Save, Save As
Cmd+W, Q — Close tab, Quit
Cmd+Z, Shift+Cmd+Z — Undo, Redo
Tab, Shift+Tab — Indent, Outdent";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Editor,
    Split,
    Preview,
}

impl View {
    pub const ALL: [(View, &'static str, Key); 3] = [
        (View::Editor, "Editor", Key::Num1),
        (View::Split, "Split", Key::Num2),
        (View::Preview, "Preview", Key::Num3),
    ];
}

#[derive(Clone, Copy)]
enum Action {
    New,
    Open,
    Save,
    SaveAs,
    CloseTab(u64),
    Quit,
}

enum Dialog {
    ConfirmClose(u64),
    ConfirmQuit,
    ConfirmOverwrite(u64),
    Message(String),
}

struct Tab {
    doc: Document,
    editor: Editor,
    preview: Preview,
    /// Revision most recently sent to the parser.
    requested: Option<u64>,
    /// Source position shared by both panes in split view.
    position: f32,
    /// Set when the editor should jump to `position` on the next frame.
    sync_editor: bool,
    sync_preview: bool,
}

impl Tab {
    fn new(doc: Document) -> Self {
        Self {
            doc,
            editor: Editor::default(),
            preview: Preview::default(),
            requested: None,
            position: 0.0,
            sync_editor: false,
            sync_preview: false,
        }
    }
}

pub struct App {
    tabs: Vec<Tab>,
    active: usize,
    view: View,
    theme: ThemePreference,
    dialog: Option<Dialog>,
    allow_close: bool,
    worker: Worker,
    markdown_cache: Option<CommonMarkCache>,
    diagrams: Diagrams,
    perf: bool,
    /// When the user last scrolled or dragged, in egui time.
    last_scroll: f64,
    /// When the keyboard last moved the editor, in egui time.
    last_caret_move: f64,
    started: Option<Instant>,
    title: String,
    /// Absent in headless tests.
    menu: Option<MenuBar>,
    /// Actions chosen from the menu bar, run during the next frame.
    pending: Vec<Action>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>, started: Instant) -> Self {
        let theme = theme::load(cc.storage);
        cc.egui_ctx.set_theme(theme);
        theme::install_fonts(&cc.egui_ctx);
        theme::apply_style(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let view = match cc.storage.and_then(|s| s.get_string("view")).as_deref() {
            Some("Editor") => View::Editor,
            Some("Preview") => View::Preview,
            _ => View::Split,
        };
        let mut app = Self::with_context(&cc.egui_ctx, view, theme);
        app.started = Some(started);
        match MenuBar::install(&cc.egui_ctx) {
            Ok(menu) => app.menu = Some(menu),
            Err(e) => eprintln!("mde: could not create the menu bar: {e}"),
        }
        for path in files {
            app.open_path(path);
        }
        app
    }

    pub fn with_context(ctx: &egui::Context, view: View, theme: ThemePreference) -> Self {
        let repaint = ctx.clone();
        Self {
            tabs: vec![Tab::new(Document::untitled())],
            active: 0,
            view,
            theme,
            dialog: None,
            allow_close: false,
            worker: Worker::spawn(move || repaint.request_repaint()),
            markdown_cache: None,
            diagrams: Diagrams::new(ctx.clone()),
            perf: std::env::var_os("MDE_PERF").is_some(),
            last_scroll: f64::NEG_INFINITY,
            last_caret_move: f64::NEG_INFINITY,
            started: None,
            title: String::new(),
            menu: None,
            pending: Vec::new(),
        }
    }

    pub fn open_path(&mut self, path: PathBuf) {
        let doc = match Document::open(&path) {
            Ok(doc) => doc,
            Err(e) => {
                self.dialog = Some(Dialog::Message(format!(
                    "Could not open {}: {e}",
                    path.display()
                )));
                return;
            }
        };
        let same = |t: &Tab| match (&t.doc.path, &doc.path) {
            (Some(a), Some(b)) => {
                a == b
                    || matches!(
                        (std::fs::canonicalize(a), std::fs::canonicalize(b)),
                        (Ok(a), Ok(b)) if a == b
                    )
            }
            _ => false,
        };
        if let Some(i) = self.tabs.iter().position(same) {
            self.active = i;
        } else if self.tabs.len() == 1 && self.tabs[0].doc.is_pristine() {
            self.tabs[0] = Tab::new(doc);
            self.active = 0;
        } else {
            self.tabs.push(Tab::new(doc));
            self.active = self.tabs.len() - 1;
        }
    }

    fn index_of(&self, doc_id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc.id == doc_id)
    }

    fn remove_tab(&mut self, index: usize) {
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::new(Document::untitled()));
        }
        if self.active > index {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len() - 1);
    }

    /// Returns true when the document is on disk afterwards.
    fn save(&mut self, index: usize, choose_name: bool, overwrite: bool) -> bool {
        let doc = &self.tabs[index].doc;
        let result = if choose_name || doc.path.is_none() {
            let name = if doc.path.is_some() {
                doc.title()
            } else {
                "Untitled.md".to_owned()
            };
            let mut dialog = rfd::FileDialog::new()
                .add_filter("Markdown", &["md", "markdown"])
                .set_file_name(name);
            if let Some(dir) = doc.path.as_ref().and_then(|p| p.parent()) {
                dialog = dialog.set_directory(dir);
            }
            match self.system_dialog(|| dialog.save_file()) {
                Some(path) => self.tabs[index].doc.save_as(path),
                None => return false,
            }
        } else if !overwrite && doc.changed_on_disk() {
            self.dialog = Some(Dialog::ConfirmOverwrite(doc.id));
            return false;
        } else {
            self.tabs[index].doc.save()
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                self.dialog = Some(Dialog::Message(format!("Could not save: {e}")));
                false
            }
        }
    }

    fn run(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::New => {
                self.tabs.push(Tab::new(Document::untitled()));
                self.active = self.tabs.len() - 1;
            }
            Action::Open => {
                let dialog = rfd::FileDialog::new()
                    .add_filter("Markdown", &["md", "markdown", "mdown", "txt"])
                    .add_filter("All files", &["*"]);
                let picked = self.system_dialog(|| dialog.pick_files());
                for path in picked.into_iter().flatten() {
                    self.open_path(path);
                }
            }
            Action::Save => {
                self.save(self.active, false, false);
            }
            Action::SaveAs => {
                self.save(self.active, true, false);
            }
            Action::CloseTab(id) => {
                if let Some(i) = self.index_of(id) {
                    if self.tabs[i].doc.is_dirty() {
                        self.active = i;
                        self.dialog = Some(Dialog::ConfirmClose(id));
                    } else {
                        self.remove_tab(i);
                    }
                }
            }
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) -> Option<Action> {
        // Dialogs take all input until answered.
        if self.dialog.is_some() {
            return None;
        }
        let pressed = |modifiers: Modifiers, key: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        for (view, _, key) in View::ALL {
            if pressed(Modifiers::COMMAND, key) {
                self.view = view;
            }
        }
        let shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // The Shift variant must be tested first.
        if pressed(shift, Key::S) {
            Some(Action::SaveAs)
        } else if pressed(Modifiers::COMMAND, Key::S) {
            Some(Action::Save)
        } else if pressed(Modifiers::COMMAND, Key::N) {
            Some(Action::New)
        } else if pressed(Modifiers::COMMAND, Key::O) {
            Some(Action::Open)
        } else if pressed(Modifiers::COMMAND, Key::W) {
            Some(Action::CloseTab(self.tabs[self.active].doc.id))
        } else if pressed(Modifiers::COMMAND, Key::Q) {
            Some(Action::Quit)
        } else {
            None
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let doc = &self.tabs[self.active].doc;
        let (title, dirty) = (doc.title(), doc.is_dirty());
        chrome::header(ui, &title, dirty, &mut self.view, &mut self.theme);

        // As in other Mac apps, tabs appear once there is more than one.
        if self.tabs.len() < 2 {
            return None;
        }
        let tabs: Vec<chrome::Tab> = self
            .tabs
            .iter()
            .map(|t| chrome::Tab {
                id: t.doc.id,
                title: t.doc.title(),
                dirty: t.doc.is_dirty(),
                path: t.doc.path.as_ref().map(|p| p.display().to_string()),
            })
            .collect();
        match chrome::tab_bar(ui, &tabs, self.active)? {
            chrome::TabAction::Select(i) => {
                self.active = i;
                None
            }
            chrome::TabAction::Close(id) => Some(Action::CloseTab(id)),
            chrome::TabAction::New => Some(Action::New),
        }
    }

    /// Applies menu bar commands. Editing commands become the input events
    /// the editor already understands.
    fn menu_command(&mut self, ctx: &egui::Context, command: Command, input: &mut egui::RawInput) {
        // A dialog takes all input until answered.
        if self.dialog.is_some() {
            return;
        }
        let mut key = |modifiers: Modifiers, key: Key| {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        };
        match command {
            Command::Undo => key(Modifiers::COMMAND, Key::Z),
            Command::Redo => key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
            Command::SelectAll => key(Modifiers::COMMAND, Key::A),
            Command::Cut => input.events.push(egui::Event::Cut),
            Command::Copy => input.events.push(egui::Event::Copy),
            // The window system answers with a paste event.
            Command::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Command::View(view) => self.view = view,
            Command::Theme(theme) => {
                self.theme = theme;
                ctx.set_theme(theme);
            }
            Command::Shortcuts => self.dialog = Some(Dialog::Message(SHORTCUTS.into())),
            Command::New => self.pending.push(Action::New),
            Command::Open => self.pending.push(Action::Open),
            Command::Save => self.pending.push(Action::Save),
            Command::SaveAs => self.pending.push(Action::SaveAs),
            Command::CloseTab => {
                let id = self.tabs[self.active].doc.id;
                self.pending.push(Action::CloseTab(id));
            }
            Command::Quit => self.pending.push(Action::Quit),
        }
    }

    /// Runs a system file dialog with the menu bar it needs.
    fn system_dialog<T>(&self, show: impl FnOnce() -> T) -> T {
        if let Some(menu) = &self.menu {
            menu.use_dialog_menu(true);
        }
        let result = show();
        if let Some(menu) = &self.menu {
            menu.use_dialog_menu(false);
        }
        result
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        // Read before drawing: scroll areas consume the scroll delta. The window
        // stays open briefly because a pane moves a frame after its input.
        let (now, scroll_input) = ui.input(|i| {
            let input = i.smooth_scroll_delta().y != 0.0 || i.pointer.primary_down();
            (i.time, input)
        });
        if scroll_input {
            self.last_scroll = now;
            // Scrolling by hand takes over from the keyboard.
            self.last_caret_move = f64::NEG_INFINITY;
        }
        let scrolling = now - self.last_scroll < SCROLL_SETTLE;
        let view = self.view;
        let split = view == View::Split;
        // A dialog takes the keyboard and mouse until it is answered.
        let interactive = self.dialog.is_none();
        let tab = &mut self.tabs[self.active];
        let (mut editor_out, mut preview_out) = (None, None);

        if view != View::Preview {
            let sync = std::mem::take(&mut tab.sync_editor);
            let target = (sync && split).then_some(tab.position);
            let mut editor = |ui: &mut egui::Ui| {
                editor_out = Some(tab.editor.show(ui, &mut tab.doc, target, interactive));
            };
            if split {
                let half = ui.available_width() / 2.0;
                egui::Panel::left("editor-pane")
                    .resizable(true)
                    .default_size(half)
                    .size_range(150.0..=(ui.available_width() - 150.0).max(150.0))
                    .show(ui, editor);
            } else {
                egui::CentralPanel::default().show(ui, |ui| editor(ui));
            }
        }

        // Parse only when something is going to display the result.
        if view != View::Editor {
            if tab.requested != Some(tab.doc.revision()) {
                tab.requested = Some(tab.doc.revision());
                self.worker
                    .request(tab.doc.id, tab.doc.revision(), &tab.doc.text);
            }
            let cache = self.markdown_cache.get_or_insert_default();
            let diagrams = &mut self.diagrams;
            let sync = std::mem::take(&mut tab.sync_preview);
            let target = (sync && split).then_some(tab.position);
            egui::CentralPanel::default().show(ui, |ui| {
                let base_dir = tab.doc.path.as_ref().and_then(|p| p.parent());
                preview_out = Some(
                    tab.preview
                        .show(ui, tab.doc.id, base_dir, cache, diagrams, target),
                );
            });
        }

        // The pane under the pointer leads and the other follows.
        if let (Some(editor), Some(preview)) = (editor_out, preview_out) {
            if editor.followed_caret {
                self.last_caret_move = now;
            }
            // While typing, the editor leads wherever the pointer happens to be.
            let keyboard = now - self.last_caret_move < SCROLL_SETTLE;
            let scrolling = scrolling || keyboard;
            let leader = if keyboard || editor.hovered {
                Some((editor.top_char, true))
            } else if preview.hovered {
                Some((preview.top_char, false))
            } else {
                None
            };
            if let Some((position, from_editor)) = leader
                && scrolling
                && (position - tab.position).abs() >= 1.0
            {
                tab.position = position;
                tab.sync_preview = from_editor;
                tab.sync_editor = !from_editor;
            }
            if scrolling {
                ui.ctx().request_repaint();
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        let response = egui::Modal::new(egui::Id::unique("dialog")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            match &dialog {
                Dialog::ConfirmClose(id) => {
                    let Some(i) = self.index_of(*id) else {
                        keep = false;
                        return;
                    };
                    ui.label(format!("Save changes to “{}”?", self.tabs[i].doc.title()));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            keep = false;
                            if self.save(i, false, false) {
                                self.remove_tab(i);
                            }
                        }
                        if ui.button("Don't Save").clicked() {
                            keep = false;
                            self.remove_tab(i);
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::ConfirmQuit => {
                    let unsaved: Vec<String> = self
                        .tabs
                        .iter()
                        .filter(|t| t.doc.is_dirty())
                        .map(|t| t.doc.title())
                        .collect();
                    ui.label(format!("Unsaved changes in: {}", unsaved.join(", ")));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save All and Quit").clicked() {
                            keep = false;
                            let all_saved = (0..self.tabs.len()).all(|i| {
                                !self.tabs[i].doc.is_dirty() || {
                                    self.active = i;
                                    self.save(i, false, false)
                                }
                            });
                            if all_saved {
                                self.allow_close = true;
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        if ui.button("Discard and Quit").clicked() {
                            keep = false;
                            self.allow_close = true;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::ConfirmOverwrite(id) => {
                    ui.label("The file was changed by another program. Overwrite it?");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Overwrite").clicked() {
                            keep = false;
                            if let Some(i) = self.index_of(*id) {
                                self.save(i, false, true);
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::Message(message) => {
                    ui.label(message);
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() {
                        keep = false;
                    }
                }
            }
        });
        // A save from inside the dialog may have raised a new dialog.
        if keep && !response.should_close() && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    fn collect_background_results(&mut self) {
        for parsed in self.worker.results() {
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.doc.id == parsed.doc_id) {
                if self.perf {
                    eprintln!(
                        "perf: preview ready {:.1} ms after edit ({} blocks)",
                        parsed.sent.elapsed().as_secs_f64() * 1e3,
                        parsed.blocks.len()
                    );
                }
                tab.preview.set_blocks(parsed.blocks, parsed.revision);
            }
        }
        self.diagrams.poll();
    }

    /// Draws one frame. Separate from `eframe::App` so it can run headless.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let frame_start = Instant::now();
        let typed =
            self.perf && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(_))));
        self.collect_background_results();

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for path in dropped {
            self.open_path(path);
        }

        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.tabs.iter().any(|t| t.doc.is_dirty())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::ConfirmQuit);
        }

        let shortcut = self.shortcuts(&ctx);
        let header = egui::Panel::top("header")
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.header(ui)
            });
        let mut actions = std::mem::take(&mut self.pending);
        actions.extend(shortcut.or(header.inner));
        for action in actions {
            self.run(&ctx, action);
        }
        if let Some(menu) = &mut self.menu {
            menu.sync(self.view, self.theme, false);
        }
        self.body(ui);
        self.dialogs(&ctx);

        let doc = &self.tabs[self.active].doc;
        let title = format!("{} — mde", doc.title());
        if title != self.title {
            self.title.clone_from(&title);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        if let Some(started) = self.started.take()
            && self.perf
        {
            eprintln!(
                "perf: first frame built {:.0} ms after launch",
                started.elapsed().as_secs_f64() * 1e3
            );
        }
        if typed {
            eprintln!(
                "perf: keystroke frame {:.1} ms",
                frame_start.elapsed().as_secs_f64() * 1e3
            );
        }
    }

    #[cfg(test)]
    pub fn active_state(&mut self) -> (&mut Document, bool) {
        let tab = &mut self.tabs[self.active];
        let current = tab.preview.revision == Some(tab.doc.revision());
        (&mut tab.doc, current)
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let commands: Vec<Command> = match &self.menu {
            Some(menu) => menu.commands().collect(),
            None => return,
        };
        if commands.is_empty() {
            return;
        }
        for command in commands {
            self.menu_command(ctx, command, input);
        }
        if let Some(menu) = &mut self.menu {
            menu.sync(self.view, self.theme, true);
        }
        ctx.request_repaint();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        theme::save(storage, self.theme);
        storage.set_string("view", format!("{:?}", self.view));
    }

    // Widget state is keyed by per-run document ids, so it is not worth keeping.
    fn persist_egui_memory(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 600.0),
            )),
            events,
            focused: true,
            ..Default::default()
        };
        // There is no window system here, so the drawn output is discarded.
        ctx.run_ui(input, |ui| app.frame(ui)).textures_delta.clear();
    }

    fn press(ctx: &egui::Context, app: &mut App, modifiers: Modifiers, key: Key) {
        let event = egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(ctx, app, vec![event]);
    }

    fn app(view: View) -> (egui::Context, App) {
        let ctx = egui::Context::default();
        let app = App::with_context(&ctx, view, ThemePreference::Light);
        (ctx, app)
    }

    #[test]
    fn shortcuts_switch_views_and_manage_tabs() {
        let (ctx, mut app) = app(View::Split);
        let cmd = Modifiers::COMMAND;
        frame(&ctx, &mut app, vec![]);
        for (key, view) in [
            (Key::Num1, View::Editor),
            (Key::Num3, View::Preview),
            (Key::Num2, View::Split),
        ] {
            press(&ctx, &mut app, cmd, key);
            assert_eq!(app.view, view);
        }
        press(&ctx, &mut app, cmd, Key::N);
        assert_eq!(app.tabs.len(), 2);
        press(&ctx, &mut app, cmd, Key::W);
        assert_eq!(app.tabs.len(), 1);

        app.tabs[0].doc.mark_changed();
        press(&ctx, &mut app, cmd, Key::W);
        assert!(matches!(app.dialog, Some(Dialog::ConfirmClose(_))));
        assert_eq!(app.tabs.len(), 1, "an unsaved tab must stay open");
    }

    #[test]
    fn menu_commands_drive_the_app_and_the_editor() {
        let (ctx, mut app) = app(View::Split);
        frame(&ctx, &mut app, vec![]);
        let id = egui::Id::unique(("editor", app.tabs[0].doc.id));
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![egui::Event::Text("hello".into())]);

        let choose = |app: &mut App, command: Command| {
            let mut input = egui::RawInput::default();
            app.menu_command(&ctx, command, &mut input);
            frame(&ctx, app, input.events);
        };
        choose(&mut app, Command::SelectAll);
        choose(&mut app, Command::Cut);
        assert_eq!(app.tabs[0].doc.text, "");
        choose(&mut app, Command::Undo);
        assert_eq!(app.tabs[0].doc.text, "hello");
        choose(&mut app, Command::Redo);
        assert_eq!(app.tabs[0].doc.text, "");

        choose(&mut app, Command::View(View::Preview));
        assert_eq!(app.view, View::Preview);
        choose(&mut app, Command::New);
        assert_eq!(app.tabs.len(), 2);
        app.tabs[1].doc.mark_changed();
        choose(&mut app, Command::CloseTab);
        assert!(matches!(app.dialog, Some(Dialog::ConfirmClose(_))));
        // Nothing gets past an open dialog.
        choose(&mut app, Command::New);
        assert_eq!(app.tabs.len(), 2);
    }

    #[test]
    fn split_view_panes_scroll_together() {
        let (ctx, mut app) = app(View::Split);
        app.tabs[0].doc.text = (0..300).map(|i| format!("Paragraph {i}\n\n")).collect();
        app.tabs[0].doc.mark_changed();
        while app.tabs[0].preview.revision != Some(1) {
            frame(&ctx, &mut app, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Scroll with the pointer over one pane, then let the other catch up.
        let scroll = |app: &mut App, x: f32| {
            frame(
                &ctx,
                app,
                vec![egui::Event::PointerMoved(egui::pos2(x, 300.0))],
            );
            for _ in 0..10 {
                let wheel = egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -120.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: Modifiers::NONE,
                };
                frame(&ctx, app, vec![wheel]);
            }
            for _ in 0..30 {
                frame(&ctx, app, vec![]);
            }
            app.tabs[0].position
        };

        let from_editor = scroll(&mut app, 200.0);
        assert!(from_editor > 100.0, "editor led to {from_editor}");
        let from_preview = scroll(&mut app, 800.0);
        assert!(from_preview > from_editor, "preview led to {from_preview}");

        // Moving with the keyboard takes the preview along, even with the
        // pointer resting over the preview.
        let id = egui::Id::unique(("editor", app.tabs[0].doc.id));
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        press(&ctx, &mut app, Modifiers::COMMAND, Key::ArrowDown);
        for _ in 0..30 {
            frame(&ctx, &mut app, vec![]);
        }
        let end = app.tabs[0].position;
        assert!(end > from_preview, "keyboard led to {end}");
    }
}
