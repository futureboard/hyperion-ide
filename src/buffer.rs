use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

const UNDO_LIMIT: usize = 80;
const TAB_SPACES: &str = "    ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coalesce {
    None,
    Insert,
    DeleteBackward,
    DeleteForward,
}

#[derive(Clone, Debug)]
struct Snapshot {
    text: String,
    selection: Range<usize>,
    reversed: bool,
}

/// Plain-text buffer with a caret, a selection, and a snapshot undo stack.
///
/// gpui-se exposes text shaping and an input-handler trait, not a document model,
/// so history lives here.
#[derive(Clone, Debug)]
pub struct Buffer {
    text: String,
    saved: String,
    selection: Range<usize>,
    reversed: bool,
    preferred_col: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    coalesce: Coalesce,
    line_starts: Vec<usize>,
    max_line_chars: usize,
}

impl Buffer {
    pub fn new(text: impl Into<String>) -> Self {
        let text = normalize_newlines(&text.into());
        let mut buffer = Self {
            saved: text.clone(),
            text,
            selection: 0..0,
            reversed: false,
            preferred_col: None,
            undo: Vec::new(),
            redo: Vec::new(),
            coalesce: Coalesce::None,
            line_starts: Vec::new(),
            max_line_chars: 0,
        };
        buffer.reindex();
        buffer
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.text.clone();
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn selection_reversed(&self) -> bool {
        self.reversed
    }

    pub fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    /// 1-based line and column at the caret.
    pub fn cursor_position(&self) -> (usize, usize) {
        let (line, column) = self.line_col(self.cursor());
        (line + 1, column + 1)
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn max_line_chars(&self) -> usize {
        self.max_line_chars
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.line_starts[line]
    }

    pub fn line_content_end(&self, line: usize) -> usize {
        if line + 1 < self.line_starts.len() {
            self.line_starts[line + 1] - 1
        } else {
            self.text.len()
        }
    }

    pub fn line_content(&self, line: usize) -> &str {
        let start = self.line_start(line);
        let end = self.line_content_end(line);
        &self.text[start..end]
    }

    pub fn line_index(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        }
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    pub fn clamp_offset(&self, offset: usize) -> usize {
        floor_char(&self.text, offset.min(self.text.len()))
    }

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        let offset = self.clamp_offset(offset);
        self.text[..offset].encode_utf16().count()
    }

    pub fn utf16_to_offset(&self, utf16_offset: usize) -> usize {
        let mut seen = 0;
        for (index, ch) in self.text.char_indices() {
            if seen >= utf16_offset {
                return index;
            }
            seen += ch.len_utf16();
        }
        self.text.len()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.utf16_to_offset(range.start)..self.utf16_to_offset(range.end)
    }

    pub fn insert(&mut self, text: &str) {
        let at_caret = self.selection.is_empty() && self.selection.start == self.cursor();
        let single = at_caret && text.chars().count() == 1 && !text.contains('\n');
        self.splice(
            self.selection.clone(),
            text,
            single.then_some(Coalesce::Insert),
        );
    }

    pub fn newline(&mut self) {
        let line = self.line_index(self.cursor());
        let indent = leading_indent(self.line_content(line));
        let mut inserted = String::from("\n");
        inserted.push_str(indent);
        self.splice(self.selection.clone(), &inserted, None);
    }

    pub fn insert_tab(&mut self) {
        self.splice(self.selection.clone(), TAB_SPACES, None);
    }

    pub fn backspace(&mut self) {
        if !self.selection.is_empty() {
            self.splice(self.selection.clone(), "", None);
            return;
        }
        let cursor = self.cursor();
        let prev = prev_grapheme(&self.text, cursor);
        if prev == cursor {
            return;
        }
        self.splice(prev..cursor, "", Some(Coalesce::DeleteBackward));
    }

    pub fn delete(&mut self) {
        if !self.selection.is_empty() {
            self.splice(self.selection.clone(), "", None);
            return;
        }
        let cursor = self.cursor();
        let next = next_grapheme(&self.text, cursor);
        if next == cursor {
            return;
        }
        self.splice(cursor..next, "", Some(Coalesce::DeleteForward));
    }

    /// Replace bytes without touching the undo stack. Used while an IME composition
    /// is already covered by [`Self::checkpoint`].
    pub fn replace_unrecorded(&mut self, range: Range<usize>, new_text: &str) {
        let start = self.clamp_offset(range.start);
        let end = self.clamp_offset(range.end.max(start));
        self.text.replace_range(start..end, new_text);
        let caret = start + new_text.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.preferred_col = None;
        self.reindex();
    }

    pub fn set_selection(&mut self, range: Range<usize>, reversed: bool) {
        let start = self.clamp_offset(range.start);
        let end = self.clamp_offset(range.end.max(start));
        self.selection = start..end;
        self.reversed = reversed;
        self.coalesce = Coalesce::None;
        self.preferred_col = None;
    }

    pub fn checkpoint(&mut self) {
        self.push_undo();
        self.coalesce = Coalesce::None;
    }

    pub fn move_left(&mut self, select: bool) {
        let next = prev_grapheme(&self.text, self.cursor());
        self.move_to(next, select);
    }

    pub fn move_right(&mut self, select: bool) {
        let next = next_grapheme(&self.text, self.cursor());
        self.move_to(next, select);
    }

    pub fn move_up(&mut self, select: bool) {
        self.move_vertical(-1, select);
    }

    pub fn move_down(&mut self, select: bool) {
        self.move_vertical(1, select);
    }

    pub fn move_vertical_by(&mut self, lines: isize, select: bool) {
        if lines == 0 {
            return;
        }
        self.move_vertical(lines, select);
    }

    pub fn move_line_start(&mut self, select: bool) {
        let line = self.line_index(self.cursor());
        let start = self.line_start(line);
        let content = self.line_content(line);
        let indent_len = leading_indent(content).len();
        let indent = start + indent_len;
        let cursor = self.cursor();
        let target = if cursor == indent { start } else { indent };
        self.move_to(target, select);
    }

    pub fn move_line_end(&mut self, select: bool) {
        let line = self.line_index(self.cursor());
        let end = self.line_content_end(line);
        self.move_to(end, select);
    }

    pub fn move_doc_start(&mut self, select: bool) {
        self.move_to(0, select);
    }

    pub fn move_doc_end(&mut self, select: bool) {
        self.move_to(self.text.len(), select);
    }

    pub fn move_word_left(&mut self, select: bool) {
        let next = word_left(&self.text, self.cursor());
        self.move_to(next, select);
    }

    pub fn move_word_right(&mut self, select: bool) {
        let next = word_right(&self.text, self.cursor());
        self.move_to(next, select);
    }

    pub fn select_all(&mut self) {
        self.coalesce = Coalesce::None;
        self.preferred_col = None;
        self.selection = 0..self.text.len();
        self.reversed = false;
    }

    pub fn select_word_at(&mut self, offset: usize) {
        let offset = self.clamp_offset(offset);
        let range = word_extent(&self.text, offset);
        self.coalesce = Coalesce::None;
        self.preferred_col = None;
        self.selection = range;
        self.reversed = false;
    }

    pub fn select_line_at(&mut self, offset: usize) {
        let line = self.line_index(self.clamp_offset(offset));
        let start = self.line_start(line);
        let end = if line + 1 < self.line_count() {
            self.line_start(line + 1)
        } else {
            self.text.len()
        };
        self.coalesce = Coalesce::None;
        self.preferred_col = None;
        self.selection = start..end;
        self.reversed = false;
    }

    pub fn set_cursor(&mut self, offset: usize, select: bool) {
        self.move_to(self.clamp_offset(offset), select);
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    fn move_vertical(&mut self, delta: isize, select: bool) {
        let (line, column) = self.line_col(self.cursor());
        let preferred = self.preferred_col.unwrap_or(column);
        let last = self.line_count() as isize - 1;
        let new_line = (line as isize + delta).clamp(0, last.max(0)) as usize;
        let offset = self.offset_at_column(new_line, preferred);
        self.move_to(offset, select);
        self.preferred_col = Some(preferred);
    }

    fn move_to(&mut self, offset: usize, select: bool) {
        let offset = self.clamp_offset(offset);
        self.coalesce = Coalesce::None;
        self.preferred_col = None;
        if select {
            let anchor = if self.reversed {
                self.selection.end
            } else {
                self.selection.start
            };
            if offset < anchor {
                self.selection = offset..anchor;
                self.reversed = true;
            } else {
                self.selection = anchor..offset;
                self.reversed = false;
            }
        } else {
            self.selection = offset..offset;
            self.reversed = false;
        }
    }

    fn splice(&mut self, range: Range<usize>, new_text: &str, coalesce: Option<Coalesce>) {
        let start = self.clamp_offset(range.start);
        let end = self.clamp_offset(range.end.max(start));
        let group = coalesce.unwrap_or(Coalesce::None);
        let continued = group != Coalesce::None && group == self.coalesce;
        if !continued {
            self.push_undo();
        }
        self.coalesce = group;
        self.text.replace_range(start..end, new_text);
        let caret = start + new_text.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.preferred_col = None;
        self.reindex();
    }

    fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            selection: self.selection.clone(),
            reversed: self.reversed,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.selection = snapshot.selection;
        self.reversed = snapshot.reversed;
        self.coalesce = Coalesce::None;
        self.preferred_col = None;
        self.reindex();
    }

    fn reindex(&mut self) {
        self.line_starts.clear();
        self.line_starts.push(0);
        self.max_line_chars = 0;
        let mut chars = 0usize;
        for (index, ch) in self.text.char_indices() {
            if ch == '\n' {
                self.max_line_chars = self.max_line_chars.max(chars);
                chars = 0;
                self.line_starts.push(index + ch.len_utf8());
            } else {
                chars += 1;
            }
        }
        self.max_line_chars = self.max_line_chars.max(chars);
    }

    fn line_col(&self, offset: usize) -> (usize, usize) {
        let line = self.line_index(offset);
        let start = self.line_start(line);
        let column = self.text[start..offset.min(self.text.len())]
            .chars()
            .count();
        (line, column)
    }

    fn offset_at_column(&self, line: usize, column: usize) -> usize {
        let start = self.line_start(line);
        let end = self.line_content_end(line);
        let mut offset = start;
        for (index, ch) in self.text[start..end].chars().enumerate() {
            if index == column {
                return offset;
            }
            offset += ch.len_utf8();
        }
        end
    }
}

pub fn normalize_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}

fn floor_char(text: &str, mut offset: usize) -> usize {
    if offset > text.len() {
        return text.len();
    }
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn leading_indent(line: &str) -> &str {
    let bytes = line
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| ch.len_utf8())
        .sum();
    &line[..bytes]
}

fn char_at(text: &str, offset: usize) -> Option<char> {
    text[offset..].chars().next()
}

fn next_grapheme(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    let mut indices = text[offset..].grapheme_indices(true);
    let _ = indices.next();
    match indices.next() {
        Some((index, _)) => offset + index,
        None => text.len(),
    }
}

fn prev_grapheme(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    text[..offset]
        .grapheme_indices(true)
        .last()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Punct,
    Space,
}

fn class_of(ch: char) -> Class {
    if ch.is_alphanumeric() || ch == '_' {
        Class::Word
    } else if ch.is_whitespace() {
        Class::Space
    } else {
        Class::Punct
    }
}

fn word_left(text: &str, mut offset: usize) -> usize {
    while offset > 0 {
        let prev = prev_grapheme(text, offset);
        match char_at(text, prev) {
            Some(ch) if ch != '\n' && ch.is_whitespace() => offset = prev,
            _ => break,
        }
    }
    if offset == 0 {
        return 0;
    }
    let Some(class) = char_at(text, prev_grapheme(text, offset)).map(class_of) else {
        return 0;
    };
    if class == Class::Space {
        return offset;
    }
    while offset > 0 {
        let prev = prev_grapheme(text, offset);
        match char_at(text, prev) {
            Some('\n') => break,
            Some(ch) if class_of(ch) == class => offset = prev,
            _ => break,
        }
    }
    offset
}

fn word_right(text: &str, mut offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    if char_at(text, offset) == Some('\n') {
        return next_grapheme(text, offset);
    }
    if let Some(ch) = char_at(text, offset)
        && !ch.is_whitespace()
    {
        let class = class_of(ch);
        while offset < text.len() {
            match char_at(text, offset) {
                Some('\n') => return offset,
                Some(ch) if class_of(ch) == class => offset = next_grapheme(text, offset),
                _ => break,
            }
        }
    }
    while offset < text.len() {
        match char_at(text, offset) {
            Some('\n') => break,
            Some(ch) if ch.is_whitespace() => offset = next_grapheme(text, offset),
            _ => break,
        }
    }
    offset
}

fn word_extent(text: &str, offset: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }
    let offset = offset.min(text.len());
    let probe = if offset == text.len() {
        prev_grapheme(text, offset)
    } else {
        offset
    };
    let Some(ch) = char_at(text, probe) else {
        return offset..offset;
    };
    if ch == '\n' || ch.is_whitespace() {
        return offset..offset;
    }
    let class = class_of(ch);
    let mut start = probe;
    while start > 0 {
        let prev = prev_grapheme(text, start);
        match char_at(text, prev) {
            Some(ch) if ch != '\n' && class_of(ch) == class => start = prev,
            _ => break,
        }
    }
    let mut end = probe;
    while end < text.len() {
        match char_at(text, end) {
            Some(ch) if ch != '\n' && class_of(ch) == class => end = next_grapheme(text, end),
            _ => break,
        }
    }
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_groups_into_one_undo() {
        let mut buffer = Buffer::new("");
        buffer.insert("a");
        buffer.insert("b");
        assert_eq!(buffer.text(), "ab");
        assert_eq!(buffer.cursor(), 2);
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "");
        assert!(buffer.redo());
        assert_eq!(buffer.text(), "ab");
    }

    #[test]
    fn newline_tracks_line_column_and_indent() {
        let mut buffer = Buffer::new("    fn");
        buffer.move_line_end(false);
        buffer.newline();
        assert_eq!(buffer.text(), "    fn\n    ");
        assert_eq!(buffer.cursor_position(), (2, 5));
        buffer.insert("x");
        assert_eq!(buffer.line_count(), 2);
        assert_eq!(buffer.line_content(1), "    x");
    }

    #[test]
    fn selection_delete_and_vertical_motion() {
        let mut buffer = Buffer::new("abcd\nxyz");
        buffer.set_cursor(1, false);
        buffer.set_cursor(3, true);
        assert_eq!(buffer.selected_text(), "bc");
        buffer.insert("Q");
        assert_eq!(buffer.text(), "aQd\nxyz");
        buffer.move_down(false);
        assert_eq!(buffer.cursor_position(), (2, 3));
        buffer.move_up(false);
        assert_eq!(buffer.cursor_position(), (1, 3));
    }

    #[test]
    fn word_motion_and_line_selection() {
        let mut buffer = Buffer::new("foo bar");
        buffer.move_doc_end(false);
        buffer.move_word_left(false);
        assert_eq!(buffer.cursor(), 4);
        buffer.move_word_left(false);
        assert_eq!(buffer.cursor(), 0);
        buffer.move_word_right(false);
        assert_eq!(buffer.cursor(), 4);
        buffer.select_line_at(0);
        assert_eq!(buffer.selected_text(), "foo bar");
        buffer.select_word_at(5);
        assert_eq!(buffer.selected_text(), "bar");
    }

    #[test]
    fn newlines_are_normalized_and_dirty_tracks_saves() {
        let mut buffer = Buffer::new("a\r\nb\rc");
        assert_eq!(buffer.text(), "a\nb\nc");
        assert!(!buffer.is_dirty());
        buffer.insert("!");
        assert!(buffer.is_dirty());
        buffer.mark_saved();
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn backspace_removes_a_grapheme() {
        let mut buffer = Buffer::new("é");
        buffer.move_doc_end(false);
        buffer.backspace();
        assert!(buffer.is_empty());
    }
}
