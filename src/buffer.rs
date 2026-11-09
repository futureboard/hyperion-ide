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
        let next = prev_grapheme(&self.text, self.snapped_cursor());
        self.move_to(next, select);
    }

    pub fn move_right(&mut self, select: bool) {
        let next = next_grapheme(&self.text, self.snapped_cursor());
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
        let cursor = self.snapped_cursor();
        let line = self.line_index(cursor);
        let start = self.line_start(line);
        let content = self.line_content(line);
        let indent_len = leading_indent(content).len();
        let indent = start + indent_len;
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
        let next = word_left(&self.text, self.snapped_cursor());
        self.move_to(next, select);
    }

    pub fn move_word_right(&mut self, select: bool) {
        let next = word_right(&self.text, self.snapped_cursor());
        self.move_to(next, select);
    }

    pub fn select_all(&mut self) {
        self.coalesce = Coalesce::None;
        self.preferred_col = None;
        self.selection = 0..self.text.len();
        self.reversed = false;
    }

    pub fn select_word_at(&mut self, offset: usize) {
        let offset = floor_grapheme(&self.text, self.clamp_offset(offset));
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
        self.move_to(offset, select);
    }

    /// Select the next case-sensitive literal match at or after the caret.
    /// Searches from the start once if nothing remains after the caret.
    pub fn find_next(&mut self, needle: &str) -> bool {
        let Some(range) = self.find_wrapping(needle, self.cursor()) else {
            return false;
        };
        self.set_selection(range, false);
        true
    }

    /// Byte range of the next literal match at or after `from`. Does not wrap.
    pub fn find_literal(&self, needle: &str, from: usize) -> Option<Range<usize>> {
        if needle.is_empty() {
            return None;
        }
        let from = self.clamp_offset(from);
        let index = self.text[from..].find(needle)?;
        let start = from + index;
        Some(start..start + needle.len())
    }

    /// Replace the current selection when it is `needle`, otherwise the next match.
    pub fn replace_one(&mut self, needle: &str, replacement: &str) -> bool {
        if needle.is_empty() {
            return false;
        }
        let range = if self.selected_text() == needle {
            self.selection.clone()
        } else {
            let Some(range) = self.find_wrapping(needle, self.cursor()) else {
                return false;
            };
            range
        };
        self.splice(range, replacement, None);
        true
    }

    /// Replace every non-overlapping literal match. Returns how many were replaced.
    pub fn replace_all(&mut self, needle: &str, replacement: &str) -> usize {
        if needle.is_empty() {
            return 0;
        }
        let cursor = self.cursor();
        let mut out = String::with_capacity(self.text.len());
        let mut last = 0usize;
        let mut count = 0usize;
        let mut shift = 0isize;
        let mut caret = cursor;
        let mut caret_fixed = false;
        while let Some(found) = self.text[last..].find(needle) {
            let start = last + found;
            let end = start + needle.len();
            out.push_str(&self.text[last..start]);
            out.push_str(replacement);
            if !caret_fixed {
                if cursor <= start {
                    caret = shift_offset(cursor, shift);
                    caret_fixed = true;
                } else if cursor < end {
                    caret = out.len();
                    caret_fixed = true;
                } else {
                    shift += replacement.len() as isize - needle.len() as isize;
                }
            }
            last = end;
            count += 1;
        }
        if count == 0 {
            return 0;
        }
        out.push_str(&self.text[last..]);
        if !caret_fixed {
            caret = shift_offset(cursor, shift);
        }
        if out != self.text {
            self.splice(0..self.text.len(), &out, None);
            let caret = caret.min(self.len());
            self.selection = caret..caret;
            self.reversed = false;
        }
        count
    }

    pub fn duplicate_line(&mut self) {
        let (first, last) = self.selected_lines();
        let span = self.line_byte_span(first, last);
        let block = self.text[span.clone()].to_string();
        let needs_break = !block.ends_with('\n');
        let inserted = if needs_break {
            let mut owned = String::with_capacity(block.len() + 1);
            owned.push('\n');
            owned.push_str(&block);
            owned
        } else {
            block
        };
        let reversed = self.reversed;
        let rel_start = self.selection.start - span.start;
        let rel_end = self.selection.end - span.start;
        let insert_at = span.end;
        self.splice(insert_at..insert_at, &inserted, None);
        let prefix = usize::from(needs_break);
        let new_start = insert_at + prefix + rel_start;
        let new_end = insert_at + prefix + rel_end;
        self.selection = new_start..new_end;
        self.reversed = reversed && new_start != new_end;
    }

    pub fn move_line_up(&mut self) -> bool {
        let (first, last) = self.selected_lines();
        if first == 0 {
            return false;
        }
        let prev_span = self.line_byte_span(first - 1, first - 1);
        let block_span = self.line_byte_span(first, last);
        self.transpose_spans(prev_span, block_span.clone(), block_span);
        true
    }

    pub fn move_line_down(&mut self) -> bool {
        let (first, last) = self.selected_lines();
        if last + 1 >= self.line_count() {
            return false;
        }
        let block_span = self.line_byte_span(first, last);
        let next_span = self.line_byte_span(last + 1, last + 1);
        self.transpose_spans(block_span.clone(), next_span, block_span);
        true
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
        let (line, column) = self.line_col(self.snapped_cursor());
        let preferred = self.preferred_col.unwrap_or(column);
        let last = self.line_count() as isize - 1;
        let new_line = (line as isize + delta).clamp(0, last.max(0)) as usize;
        let offset = self.offset_at_column(new_line, preferred);
        self.move_to(offset, select);
        self.preferred_col = Some(preferred);
    }

    fn move_to(&mut self, offset: usize, select: bool) {
        let offset = snap_grapheme(&self.text, offset);
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
        let end = offset.min(self.line_content_end(line)).max(start);
        let column = self.text[start..end].graphemes(true).count();
        (line, column)
    }

    fn offset_at_column(&self, line: usize, column: usize) -> usize {
        let start = self.line_start(line);
        let end = self.line_content_end(line);
        let mut seen = 0usize;
        for (index, _) in self.text[start..end].grapheme_indices(true) {
            if seen == column {
                return start + index;
            }
            seen += 1;
        }
        end
    }

    fn snapped_cursor(&self) -> usize {
        floor_grapheme(&self.text, self.cursor())
    }

    fn find_wrapping(&self, needle: &str, from: usize) -> Option<Range<usize>> {
        if let Some(range) = self.find_literal(needle, from) {
            return Some(range);
        }
        if from == 0 {
            return None;
        }
        let range = self.find_literal(needle, 0)?;
        (range.start < from).then_some(range)
    }

    fn selected_lines(&self) -> (usize, usize) {
        let start_line = self.line_index(self.selection.start);
        let mut end_line = self.line_index(self.selection.end);
        if self.selection.end > self.selection.start
            && end_line > start_line
            && self.selection.end == self.line_start(end_line)
        {
            end_line -= 1;
        }
        (start_line, end_line)
    }

    fn line_byte_span(&self, first: usize, last: usize) -> Range<usize> {
        let start = self.line_start(first);
        let end = if last + 1 < self.line_count() {
            self.line_start(last + 1)
        } else {
            self.text.len()
        };
        start..end
    }

    fn transpose_spans(&mut self, left: Range<usize>, right: Range<usize>, tracked: Range<usize>) {
        let left_text = with_trailing_newline(&self.text[left.clone()]);
        let right_text = with_trailing_newline(&self.text[right.clone()]);
        let region_end = right.end;
        // Both chunks are swapped with a trailing break. Drop that break when
        // the region is EOF and the buffer did not already end with one.
        let trim_eof = region_end == self.text.len() && !self.text.ends_with('\n');
        let mut combined = String::with_capacity(left_text.len() + right_text.len());
        combined.push_str(&right_text);
        combined.push_str(&left_text);
        if trim_eof {
            combined.pop();
        }
        let tracks_right = tracked.start == right.start;
        let (tracked_len, origin) = if tracks_right {
            (right_text.len(), 0)
        } else {
            (left_text.len(), right_text.len())
        };
        let strip_end = trim_eof && !tracks_right;
        let reversed = self.reversed;
        let rel_start = self.selection.start - tracked.start;
        let rel_end = self.selection.end - tracked.start;
        let map = |rel: usize| {
            if strip_end && rel >= tracked_len.saturating_sub(1) {
                tracked_len - 1
            } else {
                rel
            }
        };
        self.splice(left.start..region_end, &combined, None);
        let start = left.start + origin + map(rel_start);
        let end = left.start + origin + map(rel_end);
        self.selection = start..end.max(start);
        self.reversed = reversed && start != end;
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

fn floor_grapheme(text: &str, offset: usize) -> usize {
    let offset = floor_char(text, offset.min(text.len()));
    if offset == 0 || offset == text.len() {
        return offset;
    }
    let line_start = text[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut boundary = line_start;
    for (index, _) in text[line_start..].grapheme_indices(true) {
        let abs = line_start + index;
        if abs == offset {
            return offset;
        }
        if abs > offset {
            break;
        }
        boundary = abs;
    }
    boundary
}

fn snap_grapheme(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if offset == text.len() {
        return offset;
    }
    let start = floor_grapheme(text, offset);
    if start == offset {
        return offset;
    }
    let end = next_grapheme(text, start);
    if offset >= end || offset - start > end - offset {
        end
    } else {
        start
    }
}

fn with_trailing_newline(chunk: &str) -> String {
    if chunk.ends_with('\n') {
        chunk.to_string()
    } else {
        let mut owned = String::with_capacity(chunk.len() + 1);
        owned.push_str(chunk);
        owned.push('\n');
        owned
    }
}

fn shift_offset(offset: usize, shift: isize) -> usize {
    if shift >= 0 {
        offset + shift as usize
    } else {
        offset.saturating_sub(shift.unsigned_abs())
    }
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

    #[test]
    fn undo_restores_reversed_selection_and_redo_clears() {
        let mut buffer = Buffer::new("abcdef");
        buffer.set_cursor(4, false);
        buffer.move_left(true);
        buffer.move_left(true);
        assert_eq!(buffer.selection(), 2..4);
        assert!(buffer.selection_reversed());
        assert_eq!(buffer.cursor(), 2);
        buffer.insert("Z");
        assert_eq!(buffer.text(), "abZef");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "abcdef");
        assert_eq!(buffer.selection(), 2..4);
        assert!(buffer.selection_reversed());
        assert!(buffer.redo());
        assert_eq!(buffer.text(), "abZef");
        assert_eq!(buffer.cursor(), 3);
        buffer.insert("!");
        assert!(!buffer.redo());
        assert_eq!(buffer.text(), "abZ!ef");
    }

    #[test]
    fn checkpoint_and_delete_direction_split_undo_groups() {
        let mut buffer = Buffer::new("");
        buffer.insert("a");
        buffer.insert("b");
        buffer.checkpoint();
        buffer.insert("c");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "ab");
        assert!(buffer.redo());
        assert_eq!(buffer.text(), "abc");

        let mut buffer = Buffer::new("abcd");
        buffer.set_cursor(2, false);
        buffer.backspace();
        buffer.backspace();
        assert_eq!(buffer.text(), "cd");
        buffer.move_right(false);
        buffer.delete();
        assert_eq!(buffer.text(), "c");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "cd");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "abcd");
    }

    #[test]
    fn undo_drops_snapshots_past_the_limit() {
        let mut buffer = Buffer::new("");
        let edits = UNDO_LIMIT + 3;
        for _ in 0..edits {
            buffer.insert("ab");
        }
        assert_eq!(buffer.text().len(), edits * 2);
        for _ in 0..UNDO_LIMIT {
            assert!(buffer.undo());
        }
        assert_eq!(buffer.text().len(), 6);
        assert!(!buffer.undo());
    }

    #[test]
    fn selection_anchor_stays_put_and_line_start_toggles_indent() {
        let mut buffer = Buffer::new("abcdef");
        buffer.set_cursor(3, false);
        buffer.move_left(true);
        buffer.move_left(true);
        assert_eq!(buffer.selection(), 1..3);
        assert!(buffer.selection_reversed());
        buffer.move_right(true);
        buffer.move_right(true);
        buffer.move_right(true);
        assert_eq!(buffer.selection(), 3..4);
        assert!(!buffer.selection_reversed());
        buffer.select_all();
        assert_eq!(buffer.selected_text(), "abcdef");
        buffer.insert("Q");
        assert_eq!(buffer.text(), "Q");
        assert!(buffer.undo());
        assert_eq!(buffer.selected_text(), "abcdef");

        let mut buffer = Buffer::new("    abc");
        buffer.move_line_end(false);
        buffer.move_line_start(false);
        assert_eq!(buffer.cursor(), 4);
        buffer.move_line_start(false);
        assert_eq!(buffer.cursor(), 0);
        buffer.move_line_start(false);
        assert_eq!(buffer.cursor(), 4);

        let mut buffer = Buffer::new("abcdef\nz");
        buffer.move_line_end(false);
        buffer.move_down(false);
        assert_eq!(buffer.cursor_position(), (2, 2));
        buffer.move_up(false);
        assert_eq!(buffer.cursor_position(), (1, 7));
    }

    #[test]
    fn grapheme_motion_covers_combining_marks_and_emoji() {
        let mut buffer = Buffer::new("e\u{0301}x");
        buffer.move_right(false);
        assert_eq!(buffer.cursor(), "e\u{0301}".len());
        buffer.move_right(false);
        assert_eq!(buffer.cursor(), buffer.len());
        buffer.backspace();
        buffer.backspace();
        assert!(buffer.is_empty());

        let mut buffer = Buffer::new("e\u{0301}z");
        buffer.set_cursor(1, false);
        assert_eq!(buffer.cursor(), 0);
        buffer.set_cursor(2, false);
        assert_eq!(buffer.cursor(), "e\u{0301}".len());
        buffer.delete();
        assert_eq!(buffer.text(), "e\u{0301}");

        let mut buffer = Buffer::new("e\u{0301}\nxy");
        buffer.move_line_end(false);
        assert_eq!(buffer.cursor_position(), (1, 2));
        buffer.move_down(false);
        assert_eq!(buffer.cursor_position(), (2, 2));
        assert_eq!(&buffer.text()[buffer.cursor()..buffer.cursor() + 1], "y");
        buffer.move_up(false);
        assert_eq!(buffer.cursor_position(), (1, 2));

        let emoji = "👨‍👩‍👧‍👦";
        let toned = "👍🏽";
        let mut buffer = Buffer::new(format!("{emoji}{toned}\nxy"));
        buffer.move_right(false);
        assert_eq!(buffer.cursor(), emoji.len());
        buffer.move_right(false);
        assert_eq!(buffer.cursor(), emoji.len() + toned.len());
        assert_eq!(buffer.cursor_position(), (1, 3));
        buffer.move_down(false);
        assert_eq!(buffer.cursor_position(), (2, 3));
        buffer.move_doc_start(false);
        buffer.delete();
        assert_eq!(buffer.text(), format!("{toned}\nxy"));
        buffer.move_right(false);
        buffer.backspace();
        assert_eq!(buffer.text(), "\nxy");

        let mut buffer = Buffer::new("e\u{0301} z");
        buffer.move_word_right(false);
        assert_eq!(buffer.cursor(), buffer.text().find('z').unwrap());
    }

    #[test]
    fn literal_find_and_replace_are_case_sensitive() {
        let mut buffer = Buffer::new("alpha beta alpha");
        assert!(buffer.find_next("alpha"));
        assert_eq!(buffer.selection(), 0..5);
        assert!(buffer.find_next("alpha"));
        assert_eq!(buffer.selection(), 11..16);
        assert!(buffer.find_next("alpha"));
        assert_eq!(buffer.selection(), 0..5);
        assert!(!buffer.find_next("ALPHA"));
        assert!(!buffer.find_next("a.c"));
        assert!(!buffer.find_next(""));
        assert_eq!(buffer.find_literal("beta", 0), Some(6..10));
        assert_eq!(buffer.find_literal("alpha", 1), Some(11..16));

        buffer.set_cursor(0, false);
        assert!(buffer.replace_one("alpha", "ALPHA"));
        assert_eq!(buffer.text(), "ALPHA beta alpha");
        assert_eq!(buffer.cursor(), 5);
        assert!(buffer.replace_one("alpha", "ALPHA"));
        assert_eq!(buffer.text(), "ALPHA beta ALPHA");
        assert!(!buffer.replace_one("missing", "x"));
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "ALPHA beta alpha");

        let mut buffer = Buffer::new("Foo foo FOO foo");
        assert_eq!(buffer.replace_all("foo", "bar"), 2);
        assert_eq!(buffer.text(), "Foo bar FOO bar");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "Foo foo FOO foo");
        assert_eq!(buffer.replace_all("foo", "foo"), 2);
        assert_eq!(buffer.text(), "Foo foo FOO foo");
        assert!(!buffer.undo());

        let mut buffer = Buffer::new("aaaa");
        buffer.set_cursor(4, false);
        assert_eq!(buffer.replace_all("aa", "b"), 2);
        assert_eq!(buffer.text(), "bb");
        assert_eq!(buffer.cursor(), 2);

        let mut buffer = Buffer::new("abcdef");
        buffer.set_cursor(3, false);
        assert_eq!(buffer.replace_all("cd", "X"), 1);
        assert_eq!(buffer.text(), "abXef");
        assert_eq!(buffer.cursor(), 3);

        let mut buffer = Buffer::new("café café");
        assert!(buffer.find_next("fé"));
        assert_eq!(buffer.selected_text(), "fé");
        assert!(buffer.replace_one("fé", "fe"));
        assert_eq!(buffer.text(), "cafe café");
        assert_eq!(buffer.replace_all("", "nope"), 0);
    }

    #[test]
    fn duplicate_and_move_lines_keep_the_column() {
        let mut buffer = Buffer::new("ab\ncd");
        buffer.set_cursor(1, false);
        buffer.duplicate_line();
        assert_eq!(buffer.text(), "ab\nab\ncd");
        assert_eq!(buffer.cursor(), 4);
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "ab\ncd");
        assert_eq!(buffer.cursor(), 1);

        buffer.set_cursor(4, false);
        buffer.duplicate_line();
        assert_eq!(buffer.text(), "ab\ncd\ncd");
        assert_eq!(buffer.cursor(), 7);

        let mut buffer = Buffer::new("ab\ncd");
        buffer.set_cursor(4, false);
        assert!(buffer.move_line_up());
        assert_eq!(buffer.text(), "cd\nab");
        assert_eq!(buffer.cursor(), 1);
        assert!(buffer.move_line_down());
        assert_eq!(buffer.text(), "ab\ncd");
        assert_eq!(buffer.cursor(), 4);
        assert!(!buffer.move_line_down());
        buffer.set_cursor(0, false);
        assert!(!buffer.move_line_up());
        assert_eq!(buffer.text(), "ab\ncd");

        let mut buffer = Buffer::new("a\nb\nc\nd");
        buffer.set_selection(2..5, false);
        assert!(buffer.move_line_down());
        assert_eq!(buffer.text(), "a\nd\nb\nc");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "a\nb\nc\nd");
        buffer.set_selection(2..5, true);
        assert!(buffer.move_line_up());
        assert_eq!(buffer.text(), "b\nc\na\nd");
        assert_eq!(buffer.selection(), 0..3);
        assert!(buffer.selection_reversed());
        buffer.move_doc_end(false);
        assert!(!buffer.move_line_down());

        let mut buffer = Buffer::new("one\n");
        buffer.move_doc_end(false);
        buffer.duplicate_line();
        assert_eq!(buffer.text(), "one\n\n");
        let mut buffer = Buffer::new("");
        buffer.duplicate_line();
        assert_eq!(buffer.text(), "\n");
    }
}
