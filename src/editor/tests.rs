//! Drives the widget headlessly with real input events.

use egui::{Event, ImeEvent, Key, Modifiers, OutputCommand, PointerButton, pos2};

use super::*;

const CMD: Modifiers = Modifiers::COMMAND;
const ALT: Modifiers = Modifiers::ALT;
const SHIFT: Modifiers = Modifiers::SHIFT;
const NONE: Modifiers = Modifiers::NONE;

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    doc: Document,
    copied: Vec<String>,
    time: f64,
}

impl Harness {
    fn new(text: &str) -> Self {
        let mut doc = Document::untitled();
        doc.text = text.to_owned();
        let mut harness = Self {
            ctx: egui::Context::default(),
            editor: Editor::default(),
            doc,
            copied: Vec::new(),
            time: 0.0,
        };
        harness.frame(vec![]);
        let id = egui::Id::unique(("editor", harness.doc.id));
        harness.ctx.memory_mut(|m| m.request_focus(id));
        harness.frame(vec![]);
        harness.frame(vec![]);
        harness
    }

    fn frame(&mut self, events: Vec<Event>) {
        // Far enough apart that separate clicks are not double clicks.
        self.time += 0.05;
        let modifiers = events.iter().find_map(|e| match e {
            Event::PointerButton { modifiers, .. } => Some(*modifiers),
            _ => None,
        });
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0))),
            time: Some(self.time),
            focused: true,
            ..Default::default()
        };
        if let Some(modifiers) = modifiers {
            input.events.push(Event::ModifiersChanged(modifiers));
        }
        input.events.extend(events);
        let (editor, doc) = (&mut self.editor, &mut self.doc);
        let mut output = self.ctx.run_ui(input, |ui| {
            editor.show(ui, doc, None, true);
        });
        output.textures_delta.clear();
        for command in output.platform_output.commands {
            if let OutputCommand::CopyText(text) = command {
                self.copied.push(text);
            }
        }
    }

    fn key(&mut self, modifiers: Modifiers, key: Key) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    fn selection(&self) -> Selection {
        self.editor.buffer.as_ref().unwrap().selection
    }

    fn head(&self) -> usize {
        self.selection().head
    }

    fn selected(&self) -> &str {
        &self.doc.text[self.selection().range()]
    }

    fn button(&mut self, at: egui::Pos2, pressed: bool, modifiers: Modifiers) {
        self.frame(vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            },
        ]);
    }

    fn click(&mut self, at: egui::Pos2, modifiers: Modifiers) {
        self.button(at, true, modifiers);
        self.button(at, false, modifiers);
    }

    /// Screen position just right of the left edge of the character at `byte`.
    fn position_of(&mut self, byte: usize) -> egui::Pos2 {
        let buffer = self.editor.buffer.as_ref().unwrap();
        let line = buffer.line_of(byte);
        assert!(buffer.lines().iter().all(|l| l.height > 0.0));
        let top: f32 = buffer.lines()[..line].iter().map(|l| l.height).sum();
        let column = self.doc.text[buffer.lines()[line].byte..byte]
            .chars()
            .count();
        let font = egui::FontId::monospace(12.0);
        let width = self.ctx.fonts_mut(|f| f.glyph_width(&font, 'm'));
        pos2(
            PADDING + column as f32 * width + 1.0,
            PADDING + top + buffer.lines()[line].height / 2.0,
        )
    }
}

#[test]
fn typing_inserts_at_the_caret_and_marks_the_document() {
    let mut h = Harness::new("");
    assert!(!h.doc.is_dirty());
    h.type_text("héllo");
    h.key(NONE, Key::Enter);
    h.type_text("漢");
    assert_eq!(h.doc.text, "héllo\n漢");
    assert!(h.doc.is_dirty());
    h.key(NONE, Key::Backspace);
    h.key(NONE, Key::Backspace);
    h.key(NONE, Key::ArrowLeft);
    h.key(NONE, Key::Delete);
    assert_eq!(h.doc.text, "héll");
}

#[test]
fn arrows_move_by_character_word_line_and_document() {
    let mut h = Harness::new("one two\nthree");
    h.key(NONE, Key::ArrowRight);
    assert_eq!(h.head(), 1);
    h.key(ALT, Key::ArrowRight);
    assert_eq!(h.head(), 3);
    h.key(CMD, Key::ArrowRight);
    assert_eq!(h.head(), 7);
    h.key(NONE, Key::ArrowDown);
    assert_eq!(h.head(), 13);
    h.key(CMD, Key::ArrowLeft);
    assert_eq!(h.head(), 8);
    h.key(CMD, Key::ArrowUp);
    assert_eq!(h.head(), 0);
    h.key(CMD, Key::ArrowDown);
    assert_eq!(h.head(), 13);
}

#[test]
fn up_and_down_keep_the_column_across_short_lines() {
    let mut h = Harness::new("abcdef\nx\nabcdef");
    for _ in 0..4 {
        h.key(NONE, Key::ArrowRight);
    }
    h.key(NONE, Key::ArrowDown);
    assert_eq!(h.head(), 8);
    h.key(NONE, Key::ArrowDown);
    assert_eq!(h.head(), 13);
}

#[test]
fn a_wrapped_line_is_walked_row_by_row() {
    let text = "word ".repeat(60);
    let mut h = Harness::new(&text);
    h.key(NONE, Key::ArrowDown);
    let second_row = h.head();
    assert!(
        second_row > 10 && second_row < text.len() / 2,
        "{second_row}"
    );
    h.key(CMD, Key::ArrowRight);
    let end = h.head();
    assert!(end > second_row && end < text.len(), "{end}");
    // The end of a row must not behave like the start of the next one.
    h.key(CMD, Key::ArrowLeft);
    assert_eq!(h.head(), second_row);
    h.key(NONE, Key::ArrowUp);
    assert_eq!(h.head(), 0);
}

#[test]
fn selecting_copying_cutting_and_pasting() {
    let mut h = Harness::new("alpha beta");
    h.key(ALT | SHIFT, Key::ArrowRight);
    assert_eq!(h.selected(), "alpha");
    h.frame(vec![Event::Copy]);
    h.frame(vec![Event::Cut]);
    assert_eq!(h.doc.text, " beta");
    assert_eq!(h.copied, ["alpha", "alpha"]);
    h.frame(vec![Event::Paste("a\r\nb".into())]);
    assert_eq!(h.doc.text, "a\nb beta");
    h.key(CMD, Key::A);
    h.type_text("new");
    assert_eq!(h.doc.text, "new");
}

#[test]
fn undo_and_redo() {
    let mut h = Harness::new("");
    h.type_text("a");
    h.type_text("b");
    h.key(CMD, Key::Z);
    assert_eq!(h.doc.text, "");
    h.key(CMD | SHIFT, Key::Z);
    assert_eq!(h.doc.text, "ab");
}

#[test]
fn tab_indents_and_shift_tab_outdents_without_losing_focus() {
    let mut h = Harness::new("a\nb");
    h.key(NONE, Key::Tab);
    assert_eq!(h.doc.text, "\ta\nb");
    h.key(CMD, Key::A);
    h.key(NONE, Key::Tab);
    assert_eq!(h.doc.text, "\t\ta\n\tb");
    h.key(SHIFT, Key::Tab);
    assert_eq!(h.doc.text, "\ta\nb");
}

#[test]
fn input_method_composition_replaces_itself_then_commits() {
    let mut h = Harness::new("[]");
    h.key(NONE, Key::ArrowRight);
    for composing in ["n", "に", "にほん"] {
        h.frame(vec![Event::Ime(ImeEvent::Preedit {
            text: composing.into(),
            active_range_chars: None,
        })]);
        assert_eq!(h.doc.text, format!("[{composing}]"));
    }
    h.frame(vec![Event::Ime(ImeEvent::Commit("日本".into()))]);
    assert_eq!(h.doc.text, "[日本]");
    h.key(CMD, Key::Z);
    assert_eq!(h.doc.text, "[]");
}

#[test]
fn mouse_clicks_and_drags_select() {
    let mut h = Harness::new("first line\nsecond words here");
    let at = h.position_of(14);
    h.click(at, NONE);
    assert_eq!(h.selection(), Selection::at(14));

    let far = h.position_of(18);
    h.click(far, SHIFT);
    assert_eq!(h.selected(), "ond ");
    h.frame(vec![Event::ModifiersChanged(NONE)]);

    // Wait so the next press is not part of a double click.
    h.time += 1.0;
    let from = h.position_of(0);
    let to = h.position_of(13);
    h.button(from, true, NONE);
    h.frame(vec![Event::PointerMoved(to)]);
    h.frame(vec![Event::PointerMoved(to)]);
    h.button(to, false, NONE);
    assert_eq!(h.selected(), "first line\nse");

    h.time += 1.0;
    let word = h.position_of(20);
    h.click(word, NONE);
    h.click(word, NONE);
    assert_eq!(h.selected(), "words");
    h.click(word, NONE);
    assert_eq!(h.selected(), "second words here");
}

#[test]
fn the_view_follows_the_caret_in_a_long_document() {
    let text: String = (0..2000).map(|i| format!("line {i}\n")).collect();
    let mut h = Harness::new(&text);
    h.key(CMD, Key::ArrowDown);
    for _ in 0..4 {
        h.frame(vec![]);
    }
    let buffer = h.editor.buffer.as_ref().unwrap();
    let drawn = |line: usize| buffer.lines()[line].height > 0.0;
    assert!(
        drawn(buffer.lines().len() - 1),
        "end was scrolled into view"
    );
    assert!(!drawn(1000), "lines in between are never laid out");
}

#[test]
fn text_replaced_from_outside_is_picked_up() {
    let mut h = Harness::new("old");
    h.doc.text = "completely new\ntext".into();
    h.doc.mark_changed();
    h.frame(vec![]);
    h.key(CMD, Key::ArrowDown);
    h.type_text("!");
    assert_eq!(h.doc.text, "completely new\ntext!");
}
