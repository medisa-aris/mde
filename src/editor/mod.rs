//! A text editor that lays out only the lines on screen, so the cost of a
//! keystroke does not grow with the size of the document.

mod buffer;
mod highlight;

use std::ops::Range;
use std::sync::Arc;

use egui::text::CCursor;
use egui::{Event, EventFilter, Galley, ImeEvent, Key, Modifiers, Rect, pos2, vec2};

use crate::document::Document;
use buffer::{Buffer, EditKind, Selection};
use highlight::Style;

const PADDING: f32 = 8.0;

/// Keys the editor keeps for itself instead of letting them move focus.
const KEYS: EventFilter = EventFilter {
    tab: true,
    horizontal_arrows: true,
    vertical_arrows: true,
    escape: false,
};

pub struct EditorOutput {
    /// Character index at the top of the visible area.
    pub top_char: f32,
    pub hovered: bool,
}

#[derive(Default)]
pub struct Editor {
    buffer: Option<Buffer>,
    /// Document revision the buffer matches.
    revision: u64,
    /// Text being composed by an input method, not yet committed.
    preedit: Option<Range<usize>>,
    /// Column kept while moving up and down.
    desired_x: Option<f32>,
    /// At a wrap point, whether the caret is shown at the end of the row above.
    row_end: bool,
    /// Word or line first selected, which a drag then extends.
    drag_origin: Option<(Range<usize>, Unit)>,
    width: f32,
    blink_from: f64,
    /// Scroll so that the caret is visible.
    reveal: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Unit {
    Character,
    Word,
    Line,
}

/// What is needed to lay out lines during one frame.
struct Layout<'a> {
    ui: &'a egui::Ui,
    style: Style,
    width: f32,
    row_height: f32,
    columns: f32,
}

impl Layout<'_> {
    fn galley(&self, buffer: &Buffer, text: &str, line: usize) -> Arc<Galley> {
        let info = &buffer.lines()[line];
        let code = info.open.is_some() || info.is_fence();
        let source = &text[buffer.line_range(text, line)];
        let mut job = highlight::line(source, code, &self.style);
        job.wrap.max_width = self.width;
        self.ui.fonts_mut(|f| f.layout_job(job))
    }

    /// Measured height, or an estimate for lines that were never drawn.
    fn height(&self, buffer: &Buffer, line: usize) -> f32 {
        let measured = buffer.lines()[line].height;
        if measured > 0.0 {
            measured
        } else {
            let rows = (buffer.line_chars(line) as f32 / self.columns).ceil();
            rows.max(1.0) * self.row_height
        }
    }

    fn top_of(&self, buffer: &Buffer, line: usize) -> f32 {
        (0..line).map(|i| self.height(buffer, i)).sum()
    }

    /// The line at height `y`, and where that line starts.
    fn line_at(&self, buffer: &Buffer, y: f32) -> (usize, f32) {
        let mut top = 0.0;
        let last = buffer.lines().len() - 1;
        for line in 0..last {
            let height = self.height(buffer, line);
            if y < top + height {
                return (line, top);
            }
            top += height;
        }
        (last, top)
    }

    /// The caret rectangle for a byte offset, relative to its line.
    fn caret(&self, buffer: &Buffer, text: &str, at: usize, row_end: bool) -> (usize, Rect) {
        let line = buffer.line_of(at);
        let column = text[buffer.lines()[line].byte..at].chars().count();
        let mut cursor = CCursor::new(column);
        cursor.prefer_next_row = !row_end;
        let galley = self.galley(buffer, text, line);
        (line, galley.pos_from_cursor(cursor))
    }

    /// Byte offset closest to `pos` within a line, and whether that is the
    /// end of a wrapped row.
    fn hit(&self, buffer: &Buffer, text: &str, line: usize, pos: egui::Vec2) -> (usize, bool) {
        let galley = self.galley(buffer, text, line);
        let y = pos.y.clamp(0.0, (galley.size().y - 1.0).max(0.0));
        let cursor = galley.cursor_from_pos(vec2(pos.x, y));
        let range = buffer.line_range(text, line);
        let at = text[range.clone()]
            .char_indices()
            .nth(cursor.index.0)
            .map_or(range.end, |(i, _)| range.start + i);
        (at, !cursor.prefer_next_row && at != range.end)
    }
}

impl Editor {
    /// `scroll_to_char` moves the view so that character is at the top.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        scroll_to_char: Option<f32>,
        interactive: bool,
    ) -> EditorOutput {
        let id = egui::Id::unique(("editor", doc.id));
        let hovered = ui.rect_contains_pointer(ui.max_rect());
        let mut top_char = 0.0;

        if self.buffer.is_none() || self.revision != doc.revision() {
            self.buffer = Some(Buffer::new(&doc.text));
            self.revision = doc.revision();
            self.preedit = None;
        }
        let Some(mut buffer) = self.buffer.take() else {
            return EditorOutput { top_char, hovered };
        };

        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let style = Style::new(
            ui.visuals().dark_mode,
            font.clone(),
            ui.visuals().text_color(),
        );
        let (row_height, glyph_width) =
            ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, 'm')));
        let width = (ui.available_width() - 2.0 * PADDING).max(50.0);
        if (width - self.width).abs() > 0.5 {
            self.width = width;
            buffer.clear_heights();
        }
        let columns = (width / glyph_width.max(1.0)).floor().max(1.0);

        let mut area = egui::ScrollArea::vertical().id_salt(id).auto_shrink(false);
        if let Some(target) = scroll_to_char {
            let layout = Layout {
                ui,
                style: Style::new(false, font.clone(), egui::Color32::BLACK),
                width,
                row_height,
                columns,
            };
            area = area.vertical_scroll_offset(offset_of_char(&layout, &buffer, target));
        }

        area.show_viewport(ui, |ui, viewport| {
            let origin = ui.min_rect().min + vec2(PADDING, PADDING);
            let clip = ui.clip_rect();
            let response = ui.interact(clip, id, egui::Sense::click_and_drag());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, "Markdown editor")
            });
            if interactive && (response.clicked() || response.drag_started()) {
                response.request_focus();
            }
            if response.hovered() {
                ui.set_cursor_icon(egui::CursorIcon::Text);
            }
            let page = viewport.height();

            // Layout borrows the Ui, so input is handled before painting.
            let mut scroll_by = 0.0;
            let mut copied = None;
            let changed = {
                let layout = Layout {
                    ui,
                    style,
                    width,
                    row_height,
                    columns,
                };
                if interactive {
                    scroll_by =
                        self.pointer(&layout, &response, &mut buffer, &doc.text, origin, clip);
                }
                let focused = interactive && ui.memory(|m| m.has_focus(id));
                focused && self.keyboard(&layout, &mut buffer, &mut doc.text, page, &mut copied)
            };
            if changed {
                doc.mark_changed();
                self.revision = doc.revision();
                ui.ctx().request_repaint();
            }
            if let Some(copied) = copied {
                ui.ctx().copy_text(copied);
            }
            let focused = ui.memory(|m| m.has_focus(id));
            if focused {
                ui.memory_mut(|m| m.set_focus_lock_filter(id, KEYS));
            } else {
                self.preedit = None;
            }

            let style = Style::new(
                ui.visuals().dark_mode,
                font.clone(),
                ui.visuals().text_color(),
            );
            let layout = Layout {
                ui,
                style,
                width,
                row_height,
                columns,
            };
            let caret = self.paint(&layout, &mut buffer, &doc.text, viewport, origin, focused);

            let (line, top) = layout.line_at(&buffer, viewport.top());
            let part = ((viewport.top() - top) / layout.height(&buffer, line)).clamp(0.0, 1.0);
            top_char = buffer.lines()[line].chars as f32 + part * buffer.line_chars(line) as f32;

            let total = layout.top_of(&buffer, buffer.lines().len());
            let reveal = std::mem::take(&mut self.reveal).then(|| {
                let (line, rect) =
                    layout.caret(&buffer, &doc.text, buffer.selection.head, self.row_end);
                let top = layout.top_of(&buffer, line);
                rect.translate(origin.to_vec2() + vec2(0.0, top))
            });

            ui.set_height((total + 2.0 * PADDING).max(page));
            if let Some(rect) = reveal {
                ui.scroll_to_rect_animation(
                    rect.expand2(vec2(0.0, 2.0)),
                    None,
                    egui::style::ScrollAnimation::none(),
                );
                ui.ctx().request_repaint();
            }
            if scroll_by != 0.0 {
                ui.scroll_with_delta_animation(
                    vec2(0.0, scroll_by),
                    egui::style::ScrollAnimation::none(),
                );
                ui.ctx().request_repaint();
            }
            if focused && let Some(cursor_rect) = caret {
                let to_global = ui
                    .ctx()
                    .layer_transform_to_global(ui.layer_id())
                    .unwrap_or_default();
                ui.output_mut(|o| {
                    o.ime = Some(egui::output::IMEOutput {
                        purpose: egui::IMEPurpose::Normal,
                        rect: to_global * clip,
                        cursor_rect: to_global * cursor_rect,
                        should_interrupt_composition: false,
                    });
                });
            }
        });

        self.buffer = Some(buffer);
        EditorOutput { top_char, hovered }
    }

    /// Mouse selection. Returns how far to scroll while dragging past an edge.
    fn pointer(
        &mut self,
        layout: &Layout<'_>,
        response: &egui::Response,
        buffer: &mut Buffer,
        text: &str,
        origin: egui::Pos2,
        clip: Rect,
    ) -> f32 {
        let (pressed, shift, now) = layout
            .ui
            .input(|i| (i.pointer.primary_pressed(), i.modifiers.shift, i.time));
        let Some(pointer) = response.interact_pointer_pos() else {
            return 0.0;
        };
        let local = pointer - origin;
        let (line, top) = layout.line_at(buffer, local.y.max(0.0));
        let (at, row_end) = layout.hit(buffer, text, line, vec2(local.x, local.y - top));

        let unit = if response.triple_clicked() {
            Some(Unit::Line)
        } else if response.double_clicked() {
            Some(Unit::Word)
        } else {
            None
        };
        let around = |buffer: &Buffer, unit: Unit, at: usize| match unit {
            Unit::Character => at..at,
            Unit::Word => buffer.word_at(text, at),
            Unit::Line => {
                let line = buffer.line_range(text, buffer.line_of(at));
                line.start..(line.end + 1).min(text.len())
            }
        };

        if let Some(unit) = unit {
            let range = around(buffer, unit, at);
            buffer.selection = Selection {
                anchor: range.start,
                head: range.end,
            };
            self.drag_origin = Some((range, unit));
        } else if pressed && response.is_pointer_button_down_on() {
            if shift {
                buffer.selection.head = at;
            } else {
                buffer.selection = Selection::at(at);
            }
            let anchor = buffer.selection.anchor;
            self.drag_origin = Some((anchor..anchor, Unit::Character));
        } else if response.dragged()
            && let Some((first, unit)) = self.drag_origin.clone()
        {
            let range = around(buffer, unit, at);
            buffer.selection = if range.start < first.start {
                Selection {
                    anchor: first.end,
                    head: range.start,
                }
            } else {
                Selection {
                    anchor: first.start,
                    head: range.end,
                }
            };
        } else {
            return 0.0;
        }
        self.row_end = row_end;
        self.desired_x = None;
        self.blink_from = now;
        buffer.seal();

        if response.dragged() {
            let past = (clip.top() - pointer.y).max(0.0) - (pointer.y - clip.bottom()).max(0.0);
            past.clamp(-60.0, 60.0)
        } else {
            0.0
        }
    }

    /// Applies keyboard, clipboard and input method events. Returns true if
    /// the text changed.
    fn keyboard(
        &mut self,
        layout: &Layout<'_>,
        buffer: &mut Buffer,
        text: &mut String,
        page: f32,
        copied: &mut Option<String>,
    ) -> bool {
        let (events, now) = layout.ui.input(|i| (i.filtered_events(&KEYS), i.time));
        let mut changed = false;
        for event in events {
            let before = buffer.selection;
            let length = text.len();
            let mut edited = false;
            match event {
                Event::Text(insert) if insert != "\n" && insert != "\r" => {
                    let range = buffer.selection.range();
                    buffer.replace(text, range, &insert, EditKind::Typing);
                    edited = !insert.is_empty();
                }
                Event::Paste(insert) if !insert.is_empty() => {
                    let insert = insert.replace("\r\n", "\n").replace('\r', "\n");
                    let range = buffer.selection.range();
                    buffer.replace(text, range, &insert, EditKind::Other);
                    edited = true;
                }
                Event::Copy | Event::Cut if !buffer.selection.is_empty() => {
                    let range = buffer.selection.range();
                    *copied = Some(text[range.clone()].to_owned());
                    if matches!(event, Event::Cut) {
                        buffer.replace(text, range, "", EditKind::Other);
                        edited = true;
                    }
                }
                Event::Ime(ime) => edited = self.compose(buffer, text, ime),
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if self.preedit.is_none() => {
                    edited = self.key(layout, buffer, text, key, modifiers, page);
                }
                _ => continue,
            }
            changed |= edited;
            if edited || buffer.selection != before || text.len() != length {
                self.reveal = true;
                self.blink_from = now;
            }
        }
        changed
    }

    fn compose(&mut self, buffer: &mut Buffer, text: &mut String, event: ImeEvent) -> bool {
        let (insert, commit) = match event {
            ImeEvent::Preedit { text, .. } => (text, false),
            ImeEvent::Commit(text) => (text, true),
            _ => return false,
        };
        if insert == "\n" || insert == "\r" || (insert.is_empty() && self.preedit.is_none()) {
            return false;
        }
        // Composition replaces what was composed so far, or the selection.
        let start = match self.preedit.take() {
            Some(range) => {
                buffer.amend(text, range.clone(), &insert);
                range.start
            }
            None => {
                let range = buffer.selection.range();
                let start = range.start;
                buffer.replace(text, range, &insert, EditKind::Other);
                start
            }
        };
        if !commit && !insert.is_empty() {
            self.preedit = Some(start..start + insert.len());
        }
        true
    }

    /// Handles one key press. Returns true if the text changed.
    fn key(
        &mut self,
        layout: &Layout<'_>,
        buffer: &mut Buffer,
        text: &mut String,
        key: Key,
        modifiers: Modifiers,
        page: f32,
    ) -> bool {
        // Emacs-style Control keys, as in other macOS text fields.
        let (key, modifiers) = if cfg!(target_os = "macos") && modifiers.ctrl {
            let plain = Modifiers {
                shift: modifiers.shift,
                ..Modifiers::NONE
            };
            let line_wise = Modifiers {
                command: true,
                ..plain
            };
            match key {
                Key::A => (Key::ArrowLeft, line_wise),
                Key::E => (Key::ArrowRight, line_wise),
                Key::B => (Key::ArrowLeft, plain),
                Key::F => (Key::ArrowRight, plain),
                Key::P => (Key::ArrowUp, plain),
                Key::N => (Key::ArrowDown, plain),
                Key::H => (Key::Backspace, plain),
                Key::D => (Key::Delete, plain),
                Key::K => {
                    let head = buffer.selection.head;
                    let end = buffer.line_range(text, buffer.line_of(head)).end;
                    let end = if end == head {
                        buffer.next(text, head)
                    } else {
                        end
                    };
                    let changed = end != head;
                    buffer.replace(text, head..end, "", EditKind::Other);
                    return changed;
                }
                _ => return false,
            }
        } else {
            (key, modifiers)
        };

        let head = buffer.selection.head;
        let selected = buffer.selection.range();
        let extend = modifiers.shift;
        let move_to = |buffer: &mut Buffer, at: usize| {
            buffer.selection.head = at;
            if !extend {
                buffer.selection.anchor = at;
            }
            buffer.seal();
        };
        let vertical = matches!(
            key,
            Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown
        ) && !modifiers.command;
        if !vertical {
            self.desired_x = None;
        }
        let row_end = std::mem::take(&mut self.row_end);

        match key {
            Key::ArrowLeft | Key::ArrowRight => {
                let forward = key == Key::ArrowRight;
                let at = if modifiers.command {
                    self.row_edge(layout, buffer, text, row_end, forward)
                } else if modifiers.alt {
                    if forward {
                        buffer.next_word(text, head)
                    } else {
                        buffer.previous_word(text, head)
                    }
                } else if !extend && !selected.is_empty() {
                    if forward {
                        selected.end
                    } else {
                        selected.start
                    }
                } else if forward {
                    buffer.next(text, head)
                } else {
                    buffer.previous(text, head)
                };
                move_to(buffer, at);
            }
            Key::Home | Key::End => {
                let forward = key == Key::End;
                let at = if modifiers.command {
                    if forward { text.len() } else { 0 }
                } else {
                    self.row_edge(layout, buffer, text, row_end, forward)
                };
                move_to(buffer, at);
            }
            Key::ArrowUp | Key::ArrowDown if modifiers.command => {
                move_to(buffer, if key == Key::ArrowUp { 0 } else { text.len() });
            }
            Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown => {
                let down = matches!(key, Key::ArrowDown | Key::PageDown);
                let rows = if matches!(key, Key::PageUp | Key::PageDown) {
                    (page / layout.row_height).floor().max(1.0) as usize
                } else {
                    1
                };
                self.row_end = row_end;
                for _ in 0..rows {
                    let at = self.vertical(layout, buffer, text, down);
                    move_to(buffer, at);
                }
            }
            Key::Backspace | Key::Delete => {
                let forward = key == Key::Delete;
                let range = if !selected.is_empty() {
                    selected
                } else {
                    let line = buffer.line_range(text, buffer.line_of(head));
                    let to = if modifiers.command {
                        let edge = if forward { line.end } else { line.start };
                        match (edge == head, forward) {
                            (false, _) => edge,
                            (true, true) => buffer.next(text, head),
                            (true, false) => buffer.previous(text, head),
                        }
                    } else if modifiers.alt {
                        if forward {
                            buffer.next_word(text, head)
                        } else {
                            buffer.previous_word(text, head)
                        }
                    } else if forward {
                        buffer.next(text, head)
                    } else {
                        buffer.previous(text, head)
                    };
                    head.min(to)..head.max(to)
                };
                let changed = !range.is_empty();
                buffer.replace(text, range, "", EditKind::Deleting);
                return changed;
            }
            Key::Enter if !modifiers.command => {
                buffer.replace(text, selected, "\n", EditKind::Typing);
                return true;
            }
            Key::Tab => {
                let length = text.len();
                if modifiers.shift || text[selected.clone()].contains('\n') {
                    buffer.indent(text, modifiers.shift);
                } else {
                    buffer.replace(text, selected, "\t", EditKind::Typing);
                    return true;
                }
                return text.len() != length;
            }
            Key::A if modifiers.command => {
                buffer.selection = Selection {
                    anchor: 0,
                    head: text.len(),
                };
                buffer.seal();
            }
            Key::Z if modifiers.command && modifiers.shift => return buffer.redo(text),
            Key::Y if modifiers.command => return buffer.redo(text),
            Key::Z if modifiers.command => return buffer.undo(text),
            _ => self.row_end = row_end,
        }
        false
    }

    /// Start or end of the wrapped row the caret is on.
    fn row_edge(
        &mut self,
        layout: &Layout<'_>,
        buffer: &Buffer,
        text: &str,
        row_end: bool,
        forward: bool,
    ) -> usize {
        let (line, caret) = layout.caret(buffer, text, buffer.selection.head, row_end);
        let x = if forward { f32::MAX / 2.0 } else { 0.0 };
        let (at, ends_row) = layout.hit(buffer, text, line, vec2(x, caret.center().y));
        self.row_end = forward && ends_row;
        at
    }

    /// One row up or down, which may be within the same wrapped line.
    fn vertical(&mut self, layout: &Layout<'_>, buffer: &Buffer, text: &str, down: bool) -> usize {
        let (line, caret) = layout.caret(buffer, text, buffer.selection.head, self.row_end);
        let x = *self.desired_x.get_or_insert(caret.left());
        let height = layout.galley(buffer, text, line).size().y;
        let y = if down {
            caret.bottom() + caret.height() / 2.0
        } else {
            caret.top() - caret.height() / 2.0
        };
        let (line, y) = if y < 0.0 {
            if line == 0 {
                self.row_end = false;
                return 0;
            }
            let height = layout.galley(buffer, text, line - 1).size().y;
            (line - 1, height - 1.0)
        } else if y > height {
            if line + 1 == buffer.lines().len() {
                self.row_end = false;
                return text.len();
            }
            (line + 1, 0.0)
        } else {
            (line, y)
        };
        let (at, ends_row) = layout.hit(buffer, text, line, vec2(x, y));
        self.row_end = ends_row;
        at
    }

    /// Draws the visible lines. Returns where the caret is, if it is visible.
    fn paint(
        &self,
        layout: &Layout<'_>,
        buffer: &mut Buffer,
        text: &str,
        viewport: Rect,
        origin: egui::Pos2,
        focused: bool,
    ) -> Option<Rect> {
        let ui = layout.ui;
        let painter = ui.painter();
        let visuals = ui.visuals();
        let selection = buffer.selection.range();
        let head = buffer.selection.head;
        let head_line = buffer.line_of(head);
        let mut caret = None;
        let mut resized = false;

        let (mut line, mut top) = layout.line_at(buffer, (viewport.top() - PADDING).max(0.0));
        while line < buffer.lines().len() && top <= viewport.bottom() {
            let galley = layout.galley(buffer, text, line);
            let height = galley.size().y.max(layout.row_height);
            if (buffer.lines()[line].height - height).abs() > 0.5 {
                resized |= (layout.height(buffer, line) - height).abs() > 0.5;
                buffer.set_height(line, height);
            }
            let at = origin + vec2(0.0, top);
            let bytes = buffer.line_range(text, line);
            let column = |byte: usize| {
                text[bytes.start..byte.clamp(bytes.start, bytes.end)]
                    .chars()
                    .count()
            };

            if selection.start <= bytes.end && selection.end > bytes.start {
                let newline = selection.end > bytes.end;
                let range = column(selection.start)..column(selection.end);
                let color = visuals.selection.bg_fill;
                for rect in span_rects(&galley, range, newline) {
                    painter.rect_filled(rect.translate(at.to_vec2()), 0.0, color);
                }
            }
            if let Some(preedit) = &self.preedit
                && preedit.start <= bytes.end
                && preedit.end > bytes.start
            {
                let range = column(preedit.start)..column(preedit.end);
                for rect in span_rects(&galley, range, false) {
                    let rect = rect.translate(at.to_vec2());
                    painter.hline(
                        rect.x_range(),
                        rect.bottom() - 1.0,
                        egui::Stroke::new(1.0, layout.style.text),
                    );
                }
            }
            painter.galley(at, galley.clone(), layout.style.text);

            if line == head_line {
                let mut cursor = CCursor::new(column(head));
                cursor.prefer_next_row = !self.row_end;
                let rect = galley.pos_from_cursor(cursor).translate(at.to_vec2());
                caret = Some(Rect::from_min_max(
                    pos2(rect.left(), rect.top()),
                    pos2(rect.left() + 2.0, rect.bottom()),
                ));
            }
            top += height;
            line += 1;
        }
        if resized {
            ui.ctx().request_repaint();
        }

        if focused && let Some(rect) = caret {
            let (now, window_focused) = ui.input(|i| (i.time, i.focused));
            let phase = (now - self.blink_from).rem_euclid(1.0);
            if phase < 0.5 || !window_focused {
                painter.rect_filled(rect, 0.0, visuals.text_cursor.stroke.color);
            }
            if window_focused {
                let next = 0.5 - phase.rem_euclid(0.5);
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(next));
            }
        }
        caret
    }
}

/// Rectangles covering a character range of one laid-out line, one per row.
/// `newline` extends the last row to show that the line break is included.
fn span_rects(galley: &Galley, range: Range<usize>, newline: bool) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut start = 0;
    let rows = galley.rows.len();
    for (i, row) in galley.rows.iter().enumerate() {
        let len = row.glyphs.len();
        let from = range.start.max(start);
        let to = range.end.min(start + len);
        let trailing = newline && i + 1 == rows && range.start <= start + len;
        if from < to || trailing {
            let area = row.rect();
            let x = |column: usize| area.left() + row.x_offset((column - start).into());
            let left = x(from.min(start + len));
            let mut right = x(to.max(from).min(start + len));
            if trailing {
                right += area.height() / 3.0;
            }
            rects.push(Rect::from_x_y_ranges(left..=right, area.y_range()));
        }
        start += len;
    }
    rects
}

fn offset_of_char(layout: &Layout<'_>, buffer: &Buffer, target: f32) -> f32 {
    let line = buffer.line_of_char((target as usize).min(buffer.total_chars()));
    let len = buffer.line_chars(line).max(1) as f32;
    let part = ((target - buffer.lines()[line].chars as f32) / len).clamp(0.0, 1.0);
    layout.top_of(buffer, line) + part * layout.height(buffer, line)
}

#[cfg(test)]
mod tests;
