//! Editing state for one document: line index, selection and undo history.
//! The text itself stays in the document; every change goes through here so
//! that the index stays in step.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;

const HISTORY_LIMIT: usize = 2000;

#[derive(Clone, Copy)]
pub struct Line {
    /// Byte offset of the first character.
    pub byte: usize,
    /// Character offset of the first character.
    pub chars: usize,
    /// Set when the line starts or ends a code fence: the fence character.
    fence: Option<u8>,
    /// The code fence the line starts inside of, if any.
    pub open: Option<u8>,
    /// Measured height on screen; zero until the line has been drawn.
    pub height: f32,
}

impl Line {
    fn new(byte: usize, chars: usize, content: &str) -> Self {
        let trimmed = content.trim_start();
        let fence = if trimmed.starts_with("```") {
            Some(b'`')
        } else if trimmed.starts_with("~~~") {
            Some(b'~')
        } else {
            None
        };
        Self {
            byte,
            chars,
            fence,
            open: None,
            height: 0.0,
        }
    }

    pub fn is_fence(&self) -> bool {
        self.fence.is_some()
    }
}

/// Byte offsets. `head` is the end that moves and shows the caret.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn at(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

/// Consecutive edits of the same kind are undone together.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

struct Edit {
    at: usize,
    removed: String,
    inserted: String,
    before: Selection,
    after: Selection,
    kind: EditKind,
}

pub struct Buffer {
    lines: Vec<Line>,
    total_chars: usize,
    pub selection: Selection,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    sealed: bool,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        let mut lines = Vec::new();
        let (mut byte, mut chars) = (0, 0);
        for line in text.split('\n') {
            lines.push(Line::new(byte, chars, line));
            byte += line.len() + 1;
            chars += line.chars().count() + 1;
        }
        let mut buffer = Self {
            lines,
            total_chars: chars - 1,
            selection: Selection::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            sealed: true,
        };
        buffer.update_fences(0);
        buffer
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn total_chars(&self) -> usize {
        self.total_chars
    }

    pub fn set_height(&mut self, line: usize, height: f32) {
        self.lines[line].height = height;
    }

    pub fn clear_heights(&mut self) {
        for line in &mut self.lines {
            line.height = 0.0;
        }
    }

    pub fn line_of(&self, byte: usize) -> usize {
        self.lines.partition_point(|l| l.byte <= byte) - 1
    }

    pub fn line_of_char(&self, chars: usize) -> usize {
        self.lines.partition_point(|l| l.chars <= chars) - 1
    }

    /// The line's bytes, without its newline.
    pub fn line_range(&self, text: &str, line: usize) -> Range<usize> {
        let end = self
            .lines
            .get(line + 1)
            .map_or(text.len(), |next| next.byte - 1);
        self.lines[line].byte..end
    }

    /// Number of characters on the line, without its newline.
    pub fn line_chars(&self, line: usize) -> usize {
        let end = self
            .lines
            .get(line + 1)
            .map_or(self.total_chars, |next| next.chars - 1);
        end - self.lines[line].chars
    }

    /// Stops the next edit from joining the previous undo step.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    /// Replaces `range` with `insert` and leaves the caret after it.
    pub fn replace(
        &mut self,
        text: &mut String,
        range: Range<usize>,
        insert: &str,
        kind: EditKind,
    ) {
        if range.is_empty() && insert.is_empty() {
            return;
        }
        let before = self.selection;
        let removed = text[range.clone()].to_owned();
        self.splice(text, range.clone(), insert);
        self.selection = Selection::at(range.start + insert.len());
        self.redo.clear();

        let joins = !self.sealed && kind != EditKind::Other;
        if joins
            && let Some(last) = self.undo.last_mut()
            && last.kind == kind
        {
            // Typing forward.
            if removed.is_empty() && range.start == last.at + last.inserted.len() {
                last.inserted.push_str(insert);
                last.after = self.selection;
                self.sealed = insert.contains('\n');
                return;
            }
            // Backspacing.
            if insert.is_empty() && last.inserted.is_empty() && range.end == last.at {
                last.removed.insert_str(0, &removed);
                last.at = range.start;
                last.after = self.selection;
                return;
            }
        }
        self.undo.push(Edit {
            at: range.start,
            removed,
            inserted: insert.to_owned(),
            before,
            after: self.selection,
            kind,
        });
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.sealed = insert.contains('\n');
    }

    /// Replaces `range`, the whole of what the previous edit inserted, and
    /// keeps both as a single undo step. Used while an input method composes.
    pub fn amend(&mut self, text: &mut String, range: Range<usize>, insert: &str) {
        self.splice(text, range.clone(), insert);
        self.selection = Selection::at(range.start + insert.len());
        if let Some(last) = self.undo.last_mut() {
            debug_assert_eq!(last.at, range.start);
            last.inserted = insert.to_owned();
            last.after = self.selection;
            // A cancelled composition leaves nothing to undo.
            if last.inserted == last.removed {
                self.undo.pop();
            }
        }
    }

    pub fn undo(&mut self, text: &mut String) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        self.splice(text, edit.at..edit.at + edit.inserted.len(), &edit.removed);
        self.selection = edit.before;
        self.redo.push(edit);
        self.sealed = true;
        true
    }

    pub fn redo(&mut self, text: &mut String) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        self.splice(text, edit.at..edit.at + edit.removed.len(), &edit.inserted);
        self.selection = edit.after;
        self.undo.push(edit);
        self.sealed = true;
        true
    }

    /// Changes the text and updates only the affected part of the index.
    fn splice(&mut self, text: &mut String, range: Range<usize>, insert: &str) {
        let first = self.line_of(range.start);
        let last = self.line_of(range.end);
        let removed_chars = text[range.clone()].chars().count();
        let inserted_chars = insert.chars().count();
        text.replace_range(range.clone(), insert);

        let byte_delta = insert.len() as isize - range.len() as isize;
        let char_delta = inserted_chars as isize - removed_chars as isize;
        for line in &mut self.lines[last + 1..] {
            line.byte = line.byte.wrapping_add_signed(byte_delta);
            line.chars = line.chars.wrapping_add_signed(char_delta);
        }
        self.total_chars = self.total_chars.wrapping_add_signed(char_delta);

        // Rebuild the lines the edit touched.
        let start = self.lines[first];
        let end = self
            .lines
            .get(last + 1)
            .map_or(text.len(), |next| next.byte - 1);
        let mut chars = start.chars;
        let mut byte = start.byte;
        let rebuilt: Vec<Line> = text[start.byte..end]
            .split('\n')
            .map(|content| {
                let line = Line::new(byte, chars, content);
                byte += content.len() + 1;
                chars += content.chars().count() + 1;
                line
            })
            .collect();
        self.lines.splice(first..=last, rebuilt);
        self.update_fences(first);
    }

    fn update_fences(&mut self, from: usize) {
        let mut open = if from == 0 {
            None
        } else {
            after(&self.lines[from - 1])
        };
        for line in &mut self.lines[from..] {
            line.open = open;
            open = after(line);
        }
    }

    /// The grapheme boundary before `at`.
    pub fn previous(&self, text: &str, at: usize) -> usize {
        let line = self.line_range(text, self.line_of(at));
        if at == line.start {
            return at.saturating_sub(1);
        }
        text[line.start..at]
            .grapheme_indices(true)
            .next_back()
            .map_or(line.start, |(i, _)| line.start + i)
    }

    /// The grapheme boundary after `at`.
    pub fn next(&self, text: &str, at: usize) -> usize {
        let line = self.line_range(text, self.line_of(at));
        if at == line.end {
            return (at + 1).min(text.len());
        }
        text[at..line.end]
            .graphemes(true)
            .next()
            .map_or(line.end, |g| at + g.len())
    }

    pub fn previous_word(&self, text: &str, at: usize) -> usize {
        let line = self.line_range(text, self.line_of(at));
        if at == line.start {
            return at.saturating_sub(1);
        }
        text[line.start..at]
            .split_word_bound_indices()
            .rev()
            .find(|(_, word)| is_word(word))
            .map_or(line.start, |(i, _)| line.start + i)
    }

    pub fn next_word(&self, text: &str, at: usize) -> usize {
        let line = self.line_range(text, self.line_of(at));
        if at == line.end {
            return (at + 1).min(text.len());
        }
        text[at..line.end]
            .split_word_bound_indices()
            .find(|(_, word)| is_word(word))
            .map_or(line.end, |(i, word)| at + i + word.len())
    }

    /// The word, or run of spaces or punctuation, containing `at`.
    pub fn word_at(&self, text: &str, at: usize) -> Range<usize> {
        let line = self.line_range(text, self.line_of(at));
        let mut found = line.start..line.start;
        for (i, word) in text[line.clone()].split_word_bound_indices() {
            if line.start + i > at {
                break;
            }
            found = line.start + i..line.start + i + word.len();
        }
        found
    }

    /// Adds or removes one level of indentation on every selected line.
    pub fn indent(&mut self, text: &mut String, outdent: bool) {
        let selection = self.selection;
        let range = selection.range();
        let first = self.line_of(range.start);
        let mut last = self.line_of(range.end);
        // A selection ending at the start of a line does not include it.
        if last > first && range.end == self.lines[last].byte {
            last -= 1;
        }
        let block = self.lines[first].byte..self.line_range(text, last).end;
        let changed: Vec<String> = text[block.clone()]
            .split('\n')
            .map(|line| {
                if !outdent {
                    return format!("\t{line}");
                }
                let spaces = line.bytes().take(4).take_while(|b| *b == b' ').count();
                let cut = if line.starts_with('\t') { 1 } else { spaces };
                line[cut..].to_owned()
            })
            .collect();
        let changed = changed.join("\n");
        if changed == text[block.clone()] {
            return;
        }
        let old_len = text.len();
        let old_first = self.line_range(text, first).len();
        let start = block.start;
        self.replace(text, block, &changed, EditKind::Other);

        let after = if selection.is_empty() {
            let delta = text.len() as isize - old_len as isize;
            let head = selection.head.saturating_add_signed(delta).max(start);
            Selection::at(floor_boundary(text, head))
        } else {
            debug_assert!(old_first <= old_len);
            Selection {
                anchor: start,
                head: start + changed.len(),
            }
        };
        self.selection = after;
        if let Some(edit) = self.undo.last_mut() {
            edit.after = after;
        }
    }
}

fn after(line: &Line) -> Option<u8> {
    match (line.open, line.fence) {
        (None, fence) => fence,
        (Some(open), Some(fence)) if open == fence => None,
        (open, _) => open,
    }
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The incrementally updated index must match one built from scratch.
    fn assert_index(buffer: &Buffer, text: &str) {
        let fresh = Buffer::new(text);
        let key = |b: &Buffer| -> Vec<_> {
            b.lines
                .iter()
                .map(|l| (l.byte, l.chars, l.fence, l.open))
                .collect()
        };
        assert_eq!(key(buffer), key(&fresh), "{text:?}");
        assert_eq!(buffer.total_chars, text.chars().count());
    }

    fn typed(buffer: &mut Buffer, text: &mut String, s: &str) {
        let range = buffer.selection.range();
        buffer.replace(text, range, s, EditKind::Typing);
    }

    #[test]
    fn index_survives_random_edits() {
        let pieces = ["a", "é", "漢", "\n", "```", "~~~", " ", "👍🏽", "\n\n"];
        let mut text = String::from("start\n```\ncode\n```\nend");
        let mut buffer = Buffer::new(&text);
        let mut seed = 12345u64;
        let mut random = move |limit: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 33) as usize % limit.max(1)
        };
        let original = text.clone();
        for _ in 0..600 {
            let a = floor_boundary(&text, random(text.len() + 1));
            let b = floor_boundary(&text, (a + random(8)).min(text.len()));
            let insert: String = (0..random(4))
                .map(|_| pieces[random(pieces.len())])
                .collect();
            buffer.replace(&mut text, a..b, &insert, EditKind::Other);
            assert_index(&buffer, &text);
        }
        let edited = text.clone();
        while buffer.undo(&mut text) {
            assert_index(&buffer, &text);
        }
        assert_eq!(text, original);
        while buffer.redo(&mut text) {}
        assert_eq!(text, edited);
        assert_index(&buffer, &text);
    }

    #[test]
    fn typing_and_backspacing_undo_in_groups() {
        let mut text = String::new();
        let mut buffer = Buffer::new(&text);
        for c in ["a", "b", "c", "\n", "d"] {
            typed(&mut buffer, &mut text, c);
        }
        buffer.undo(&mut text);
        assert_eq!(text, "abc\n");
        buffer.undo(&mut text);
        assert_eq!(text, "");
        buffer.redo(&mut text);
        buffer.redo(&mut text);
        assert_eq!(text, "abc\nd");

        for _ in 0..3 {
            let head = buffer.selection.head;
            let from = buffer.previous(&text, head);
            buffer.replace(&mut text, from..head, "", EditKind::Deleting);
        }
        assert_eq!(text, "ab");
        buffer.undo(&mut text);
        assert_eq!(text, "abc\nd");
    }

    #[test]
    fn moves_over_whole_graphemes_and_words() {
        let text = "e\u{301}👍🏽 two,  three\nx";
        let buffer = Buffer::new(text);
        let first = buffer.next(text, 0);
        assert_eq!(&text[..first], "e\u{301}");
        let second = buffer.next(text, first);
        assert_eq!(&text[first..second], "👍🏽");
        assert_eq!(buffer.previous(text, second), first);
        assert_eq!(&text[second..buffer.next_word(text, second)], " two");
        let end = text.find('\n').unwrap();
        assert_eq!(&text[buffer.previous_word(text, end)..end], "three");
        assert_eq!(buffer.next(text, end), end + 1);
        assert_eq!(buffer.previous(text, end + 1), end);
        assert_eq!(&text[buffer.word_at(text, second + 2)], "two");
    }

    #[test]
    fn indents_and_outdents_selected_lines() {
        let mut text = String::from("one\ntwo\nthree");
        let mut buffer = Buffer::new(&text);
        buffer.selection = Selection { anchor: 1, head: 8 };
        buffer.indent(&mut text, false);
        assert_eq!(text, "\tone\n\ttwo\nthree");
        buffer.indent(&mut text, true);
        assert_eq!(text, "one\ntwo\nthree");
        buffer.undo(&mut text);
        assert_eq!(text, "\tone\n\ttwo\nthree");
        assert_index(&buffer, &text);
    }

    #[test]
    fn tracks_code_fences() {
        let mut text = String::from("a\n```\nb\n```\nc");
        let mut buffer = Buffer::new(&text);
        let inside: Vec<bool> = buffer.lines().iter().map(|l| l.open.is_some()).collect();
        assert_eq!(inside, [false, false, true, true, false]);
        // Removing the opening fence turns the closing one into an opening one.
        buffer.replace(&mut text, 2..6, "", EditKind::Other);
        assert!(buffer.lines().last().unwrap().open.is_some());
        assert_index(&buffer, &text);
    }
}
