use std::ops::Range;
use std::path::Path;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, Font, FontFallbacks, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ScrollHandle, ShapedLine, SharedString, Style,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point,
    prelude::*, px, relative, size,
};

use crate::buffer::{self, Buffer};
use crate::highlight;
use crate::theme;

const FONT_PX: f32 = 14.0;
const LINE_PX: f32 = 22.0;
const PAD_PX: f32 = 8.0;

actions!(
    editor,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        LineStart,
        LineEnd,
        SelectLineStart,
        SelectLineEnd,
        DocStart,
        DocEnd,
        SelectDocStart,
        SelectDocEnd,
        PageUp,
        PageDown,
        SelectPageUp,
        SelectPageDown,
        Backspace,
        Delete,
        Newline,
        InsertTab,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
    ]
);

pub fn bind_editor_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, Some("Editor")),
        KeyBinding::new("right", MoveRight, Some("Editor")),
        KeyBinding::new("up", MoveUp, Some("Editor")),
        KeyBinding::new("down", MoveDown, Some("Editor")),
        KeyBinding::new("shift-left", SelectLeft, Some("Editor")),
        KeyBinding::new("shift-right", SelectRight, Some("Editor")),
        KeyBinding::new("shift-up", SelectUp, Some("Editor")),
        KeyBinding::new("shift-down", SelectDown, Some("Editor")),
        KeyBinding::new("home", LineStart, Some("Editor")),
        KeyBinding::new("end", LineEnd, Some("Editor")),
        KeyBinding::new("shift-home", SelectLineStart, Some("Editor")),
        KeyBinding::new("shift-end", SelectLineEnd, Some("Editor")),
        KeyBinding::new("secondary-up", DocStart, Some("Editor")),
        KeyBinding::new("secondary-down", DocEnd, Some("Editor")),
        KeyBinding::new("secondary-shift-up", SelectDocStart, Some("Editor")),
        KeyBinding::new("secondary-shift-down", SelectDocEnd, Some("Editor")),
        KeyBinding::new("pageup", PageUp, Some("Editor")),
        KeyBinding::new("pagedown", PageDown, Some("Editor")),
        KeyBinding::new("shift-pageup", SelectPageUp, Some("Editor")),
        KeyBinding::new("shift-pagedown", SelectPageDown, Some("Editor")),
        KeyBinding::new("backspace", Backspace, Some("Editor")),
        KeyBinding::new("delete", Delete, Some("Editor")),
        KeyBinding::new("enter", Newline, Some("Editor")),
        KeyBinding::new("tab", InsertTab, Some("Editor")),
        KeyBinding::new("secondary-a", SelectAll, Some("Editor")),
        KeyBinding::new("secondary-z", Undo, Some("Editor")),
        KeyBinding::new("secondary-shift-z", Redo, Some("Editor")),
        KeyBinding::new("secondary-y", Redo, Some("Editor")),
        KeyBinding::new("secondary-c", Copy, Some("Editor")),
        KeyBinding::new("secondary-x", Cut, Some("Editor")),
        KeyBinding::new("secondary-v", Paste, Some("Editor")),
    ]);

    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("alt-left", WordLeft, Some("Editor")),
        KeyBinding::new("alt-right", WordRight, Some("Editor")),
        KeyBinding::new("alt-shift-left", SelectWordLeft, Some("Editor")),
        KeyBinding::new("alt-shift-right", SelectWordRight, Some("Editor")),
        KeyBinding::new("secondary-left", LineStart, Some("Editor")),
        KeyBinding::new("secondary-right", LineEnd, Some("Editor")),
        KeyBinding::new("secondary-shift-left", SelectLineStart, Some("Editor")),
        KeyBinding::new("secondary-shift-right", SelectLineEnd, Some("Editor")),
    ]);

    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-left", WordLeft, Some("Editor")),
        KeyBinding::new("ctrl-right", WordRight, Some("Editor")),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some("Editor")),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, Some("Editor")),
        KeyBinding::new("alt-left", WordLeft, Some("Editor")),
        KeyBinding::new("alt-right", WordRight, Some("Editor")),
        KeyBinding::new("alt-shift-left", SelectWordLeft, Some("Editor")),
        KeyBinding::new("alt-shift-right", SelectWordRight, Some("Editor")),
    ]);
}

enum KeyboardNav {
    Left(bool),
    Right(bool),
    Up(bool),
    Down(bool),
    LineStart(bool),
    LineEnd(bool),
    DocStart(bool),
    DocEnd(bool),
    Vertical(isize, bool),
    WordLeft(bool),
    WordRight(bool),
    SelectAll,
}

pub struct CodeEditor {
    focus: FocusHandle,
    scroll: ScrollHandle,
    buffer: Buffer,
    language: highlight::Language,
    marked: Option<Range<usize>>,
    selecting: bool,
    last_bounds: Option<Bounds<Pixels>>,
}

impl CodeEditor {
    pub fn new(text: String, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            buffer: Buffer::new(text),
            language: highlight::Language::Plain,
            marked: None,
            selecting: false,
            last_bounds: None,
        }
    }

    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    pub fn mark_saved(&mut self) {
        self.buffer.mark_saved();
    }

    pub fn set_language_from_path(&mut self, path: &Path) {
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        self.language = highlight::language_from_extension(extension);
    }

    pub fn cursor_position(&self) -> (usize, usize) {
        self.buffer.cursor_position()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.undo() {
            self.marked = None;
            self.changed(window, cx);
        }
    }

    pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.redo() {
            self.marked = None;
            self.changed(window, cx);
        }
    }

    pub fn copy(&mut self, cx: &mut Context<Self>) {
        let selected = self.buffer.selected_text();
        if !selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(selected.to_string()));
        }
    }

    pub fn cut(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.selection().is_empty() {
            return;
        }
        self.copy(cx);
        self.buffer.insert("");
        self.marked = None;
        self.changed(window, cx);
    }

    pub fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.buffer.insert(&buffer::normalize_newlines(&text));
        self.marked = None;
        self.changed(window, cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.nav(KeyboardNav::SelectAll);
        cx.notify();
    }

    fn nav(&mut self, nav: KeyboardNav) {
        apply_keyboard_nav(&mut self.buffer, &mut self.marked, nav);
    }

    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_cursor(window);
        cx.notify();
    }

    fn page_lines(&self) -> isize {
        let height = self.scroll.bounds().size.height;
        let lines = (height / px(LINE_PX)) as isize;
        if lines <= 1 { 24 } else { lines - 1 }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        let Some(offset) = self.offset_for_position(event.position, window) else {
            return;
        };
        let gutter = px(gutter_px(self.buffer.line_count()));
        let in_gutter = self
            .last_bounds
            .is_some_and(|bounds| event.position.x < bounds.left() + gutter);
        if in_gutter || event.click_count >= 3 {
            self.selecting = false;
            self.buffer.select_line_at(offset);
        } else if event.click_count == 2 {
            self.selecting = false;
            self.buffer.select_word_at(offset);
        } else {
            self.selecting = true;
            self.buffer.set_cursor(offset, event.modifiers.shift);
        }
        self.marked = None;
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selecting {
            return;
        }
        if let Some(offset) = self.offset_for_position(event.position, window) {
            self.buffer.set_cursor(offset, true);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            cx.notify();
        }
    }

    fn offset_for_position(&self, position: Point<Pixels>, window: &mut Window) -> Option<usize> {
        let bounds = self.last_bounds?;
        if self.buffer.line_count() == 0 {
            return Some(0);
        }
        let gutter = px(gutter_px(self.buffer.line_count()));
        let line_h = px(LINE_PX);
        if position.y >= bounds.bottom() {
            return Some(self.buffer.len());
        }
        let y = position.y - bounds.top() - px(PAD_PX);
        let line = if y <= px(0.) {
            0
        } else {
            ((y / line_h) as usize).min(self.buffer.line_count() - 1)
        };
        let x = position.x - bounds.left() - gutter;
        if x <= px(0.) {
            return Some(self.buffer.line_start(line));
        }
        let content = self.buffer.line_content(line);
        let byte = match shape_code(content, self.language, None, window) {
            Some(shaped) => shaped.closest_index_for_x(x).min(content.len()),
            None => 0,
        };
        Some(
            self.buffer
                .clamp_offset(self.buffer.line_start(line) + byte),
        )
    }

    fn reveal_cursor(&self, window: &mut Window) {
        let Some(bounds) = self.last_bounds else {
            return;
        };
        let viewport = self.scroll.bounds();
        if viewport.size.height <= px(0.) && viewport.size.width <= px(0.) {
            return;
        }
        let line = self.buffer.line_index(self.buffer.cursor());
        let line_top = bounds.top() + px(PAD_PX + LINE_PX * line as f32);
        let line_bottom = line_top + px(LINE_PX);
        let (caret_left, caret_right) = self.caret_span_x(bounds.left(), window);
        let offset = reveal_offset(
            self.scroll.offset(),
            self.scroll.max_offset(),
            caret_left,
            caret_right,
            line_top,
            line_bottom,
            viewport,
        );
        if offset != self.scroll.offset() {
            self.scroll.set_offset(offset);
        }
    }

    fn caret_span_x(&self, bounds_left: Pixels, window: &mut Window) -> (Pixels, Pixels) {
        let line = self.buffer.line_index(self.buffer.cursor());
        let line_start = self.buffer.line_start(line);
        let content = self.buffer.line_content(line);
        let column = self
            .buffer
            .cursor()
            .saturating_sub(line_start)
            .min(content.len());
        let local = match shape_code(content, self.language, None, window) {
            Some(shaped) => shaped.x_for_index(column),
            None => px(column as f32 * FONT_PX * 0.62),
        };
        let left = bounds_left + px(gutter_px(self.buffer.line_count())) + local;
        (left, left + px(2.))
    }

    fn edit_range(&mut self, range: Range<usize>, new_text: &str) {
        if self.marked.is_some() {
            self.buffer.replace_unrecorded(range, new_text);
            self.marked = None;
            return;
        }
        if range == self.buffer.selection() {
            self.buffer.insert(new_text);
            return;
        }
        self.buffer.checkpoint();
        self.buffer.replace_unrecorded(range, new_text);
    }
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = ordered_range(self.buffer.range_from_utf16(&range_utf16));
        actual_range.replace(self.buffer.range_to_utf16(&range));
        Some(self.buffer.text().get(range).unwrap_or("").to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.buffer.range_to_utf16(&self.buffer.selection()),
            reversed: self.buffer.selection_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|range| self.buffer.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.buffer.range_from_utf16(range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.buffer.selection());
        self.edit_range(range, new_text);
        self.changed(window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.buffer.range_from_utf16(range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.buffer.selection());
        let start = self.buffer.clamp_offset(range.start);
        let end = self.buffer.clamp_offset(range.end.max(start));
        if self.marked.is_none() {
            self.buffer.checkpoint();
        }
        self.buffer.replace_unrecorded(start..end, new_text);
        if new_text.is_empty() {
            self.marked = None;
        } else {
            self.marked = Some(start..start + new_text.len());
        }
        if let Some(selected) = new_selected_range {
            let sel_start = utf16_prefix_bytes(new_text, selected.start);
            let sel_end = utf16_prefix_bytes(new_text, selected.end.max(selected.start));
            self.buffer
                .set_selection(start + sel_start..start + sel_end, false);
        }
        self.changed(window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let bounds = self.last_bounds?;
        let offset = self.buffer.utf16_to_offset(range_utf16.start);
        let line = self.buffer.line_index(offset);
        let line_start = self.buffer.line_start(line);
        let content = self.buffer.line_content(line);
        let column = offset.saturating_sub(line_start).min(content.len());
        let x = match shape_code(content, self.language, None, window) {
            Some(shaped) => shaped.x_for_index(column),
            None => px(0.),
        };
        let top = bounds.top() + px(PAD_PX + LINE_PX * line as f32);
        let left = bounds.left() + px(gutter_px(self.buffer.line_count())) + x;
        Some(Bounds::new(point(left, top), size(px(2.), px(LINE_PX))))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_for_position(point, window)?;
        Some(self.buffer.offset_to_utf16(offset))
    }
}

impl Render for CodeEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context("Editor")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .bg(theme::editor_bg())
            .on_action(cx.listener(|this, _: &MoveLeft, window, cx| {
                this.nav(KeyboardNav::Left(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveRight, window, cx| {
                this.nav(KeyboardNav::Right(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveUp, window, cx| {
                this.nav(KeyboardNav::Up(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveDown, window, cx| {
                this.nav(KeyboardNav::Down(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLeft, window, cx| {
                this.nav(KeyboardNav::Left(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectRight, window, cx| {
                this.nav(KeyboardNav::Right(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectUp, window, cx| {
                this.nav(KeyboardNav::Up(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectDown, window, cx| {
                this.nav(KeyboardNav::Down(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &LineStart, window, cx| {
                this.nav(KeyboardNav::LineStart(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &LineEnd, window, cx| {
                this.nav(KeyboardNav::LineEnd(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLineStart, window, cx| {
                this.nav(KeyboardNav::LineStart(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLineEnd, window, cx| {
                this.nav(KeyboardNav::LineEnd(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DocStart, window, cx| {
                this.nav(KeyboardNav::DocStart(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DocEnd, window, cx| {
                this.nav(KeyboardNav::DocEnd(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectDocStart, window, cx| {
                this.nav(KeyboardNav::DocStart(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectDocEnd, window, cx| {
                this.nav(KeyboardNav::DocEnd(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PageUp, window, cx| {
                let lines = this.page_lines();
                this.nav(KeyboardNav::Vertical(-lines, false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PageDown, window, cx| {
                let lines = this.page_lines();
                this.nav(KeyboardNav::Vertical(lines, false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPageUp, window, cx| {
                let lines = this.page_lines();
                this.nav(KeyboardNav::Vertical(-lines, true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPageDown, window, cx| {
                let lines = this.page_lines();
                this.nav(KeyboardNav::Vertical(lines, true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Backspace, window, cx| {
                this.buffer.backspace();
                this.marked = None;
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Delete, window, cx| {
                this.buffer.delete();
                this.marked = None;
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Newline, window, cx| {
                this.buffer.newline();
                this.marked = None;
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &InsertTab, window, cx| {
                this.buffer.insert_tab();
                this.marked = None;
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &WordLeft, window, cx| {
                this.nav(KeyboardNav::WordLeft(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &WordRight, window, cx| {
                this.nav(KeyboardNav::WordRight(false));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectWordLeft, window, cx| {
                this.nav(KeyboardNav::WordLeft(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectWordRight, window, cx| {
                this.nav(KeyboardNav::WordRight(true));
                this.changed(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo(window, cx)))
            .on_action(cx.listener(|this, _: &Redo, window, cx| this.redo(window, cx)))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &Cut, window, cx| this.cut(window, cx)))
            .on_action(cx.listener(|this, _: &Paste, window, cx| this.paste(window, cx)))
            .child(
                div()
                    .id("editor-scroll")
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_up_out(gpui::MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .child(EditorElement {
                        editor: cx.entity(),
                    }),
            )
    }
}

struct EditorElement {
    editor: Entity<CodeEditor>,
}

struct DrawnLine {
    top: Pixels,
    shaped: Option<ShapedLine>,
}

struct EditorPaint {
    lines: Vec<DrawnLine>,
    gutter_quad: PaintQuad,
    highlight: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    numbers: Vec<(ShapedLine, Point<Pixels>)>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let editor = self.editor.read(cx);
        let lines = editor.buffer.line_count().max(1) as f32;
        let gutter = gutter_px(editor.buffer.line_count());
        let text_width = editor.buffer.max_line_chars() as f32 * FONT_PX * 0.62;
        let content_height = px(PAD_PX * 2.0 + lines * LINE_PX);
        let viewport_height = editor.scroll.bounds().size.height;
        let height = if viewport_height > content_height {
            viewport_height
        } else {
            content_height
        };
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.min_size.width = px(gutter + text_width + 48.0).into();
        style.size.height = height.into();
        style.min_size.height = content_height.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let editor = self.editor.read(cx);
        let line_count = editor.buffer.line_count().max(1);
        let gutter = px(gutter_px(line_count));
        let viewport = editor.scroll.bounds();
        let cursor = editor.buffer.cursor();
        let cursor_line = editor.buffer.line_index(cursor);
        let (first, last) = visible_lines(bounds, viewport, line_count, cursor_line);
        let selection = editor.buffer.selection();
        let marked = editor.marked.clone();
        let focused = editor.focus.is_focused(window);
        let language = editor.language;
        let mut lines = Vec::new();
        let mut selections = Vec::new();
        let mut numbers = Vec::new();
        let mut highlight = None;
        let mut cursor_quad = None;

        for index in first..last {
            let top = bounds.top() + px(PAD_PX + LINE_PX * index as f32);
            let content = editor.buffer.line_content(index);
            let line_start = editor.buffer.line_start(index);
            let mark = line_mark(marked.as_ref(), line_start, content.len());
            let shaped = shape_code(content, language, mark, window);
            if index == cursor_line {
                highlight = Some(fill(
                    Bounds::new(
                        point(bounds.left(), top),
                        size(bounds.size.width, px(LINE_PX)),
                    ),
                    theme::line_hl(),
                ));
            }
            if selection.end > selection.start {
                let content_end = editor.buffer.line_content_end(index);
                let next_line = if index + 1 < editor.buffer.line_count() {
                    editor.buffer.line_start(index + 1)
                } else {
                    editor.buffer.len()
                };
                if selection.end > line_start && selection.start < next_line {
                    let from = selection.start.max(line_start).min(content_end);
                    let to = selection.end.min(content_end).max(from);
                    let x0 = byte_x(&shaped, from - line_start);
                    let mut x1 = byte_x(&shaped, to - line_start);
                    if selection.end > content_end {
                        x1 += px(8.);
                    }
                    if x1 <= x0 {
                        x1 = x0 + px(6.);
                    }
                    selections.push(fill(
                        Bounds::new(
                            point(bounds.left() + gutter + x0, top),
                            size(x1 - x0, px(LINE_PX)),
                        ),
                        theme::selection(),
                    ));
                }
            } else if index == cursor_line && focused {
                let column = cursor.saturating_sub(line_start).min(content.len());
                let x = bounds.left() + gutter + byte_x(&shaped, column);
                cursor_quad = Some(fill(
                    Bounds::new(point(x, top), size(px(2.), px(LINE_PX))),
                    theme::accent(),
                ));
            }
            let number = format!("{}", index + 1);
            let number_color: Hsla = if index == cursor_line {
                theme::text().into()
            } else {
                theme::faint().into()
            };
            if let Some(number_line) = shape_text(&number, number_color, window) {
                let x = bounds.left() + gutter - px(12.) - number_line.width();
                numbers.push((number_line, point(x, top)));
            }
            lines.push(DrawnLine { top, shaped });
        }

        EditorPaint {
            lines,
            gutter_quad: fill(
                Bounds::new(bounds.origin, size(gutter, bounds.size.height)),
                theme::gutter_bg(),
            ),
            highlight,
            selections,
            numbers,
            cursor: cursor_quad,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        window.paint_quad(prepaint.gutter_quad.clone());
        if let Some(highlight) = prepaint.highlight.take() {
            window.paint_quad(highlight);
        }
        for selection in prepaint.selections.drain(..) {
            window.paint_quad(selection);
        }
        for line in &prepaint.lines {
            if let Some(shaped) = &line.shaped {
                let gutter = px(gutter_px(self.editor.read(cx).buffer.line_count()));
                shaped
                    .paint(
                        point(bounds.left() + gutter, line.top),
                        px(LINE_PX),
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
            }
        }
        for (shaped, origin) in &prepaint.numbers {
            shaped
                .paint(*origin, px(LINE_PX), TextAlign::Left, None, window, cx)
                .ok();
        }
        if focus.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }
        self.editor.update(cx, |editor, _cx| {
            editor.last_bounds = Some(bounds);
        });
    }
}

fn gutter_px(line_count: usize) -> f32 {
    let digits = line_count.max(1).to_string().len().max(2) as f32;
    18.0 + digits * FONT_PX * 0.62 + 8.0
}

fn visible_lines(
    bounds: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    line_count: usize,
    cursor_line: usize,
) -> (usize, usize) {
    if viewport.size.height <= px(0.) || viewport.size.width <= px(0.) {
        let last = line_count.min(80).max(1);
        return (0, last.max(cursor_line + 1).min(line_count));
    }
    let origin = bounds.top() + px(PAD_PX);
    let first = ((viewport.top() - origin) / px(LINE_PX)).floor() as isize - 2;
    let last = ((viewport.bottom() - origin) / px(LINE_PX)).ceil() as isize + 2;
    let first = (first.max(0) as usize).min(cursor_line);
    let last = (last.max(0) as usize).max(cursor_line + 1).min(line_count);
    if first >= last {
        let index = cursor_line.min(line_count.saturating_sub(1));
        (index, line_count.min(index + 1))
    } else {
        (first, last)
    }
}

fn line_mark(
    marked: Option<&Range<usize>>,
    line_start: usize,
    line_len: usize,
) -> Option<Range<usize>> {
    let marked = marked?;
    let line_end = line_start + line_len;
    if marked.end <= line_start || marked.start >= line_end {
        return None;
    }
    let start = marked.start.max(line_start) - line_start;
    let end = marked.end.min(line_end) - line_start;
    (start < end).then_some(start..end)
}

fn byte_x(shaped: &Option<ShapedLine>, byte: usize) -> Pixels {
    match shaped {
        Some(shaped) => shaped.x_for_index(byte),
        None => px(0.),
    }
}

fn shape_text(text: &str, color: Hsla, window: &mut Window) -> Option<ShapedLine> {
    if text.is_empty() {
        return None;
    }
    let font = editor_font();
    let runs = vec![text_run(text.len(), color, &font, false)];
    Some(window.text_system().shape_line(
        SharedString::from(text.to_string()),
        px(FONT_PX),
        &runs,
        None,
    ))
}

fn shape_code(
    text: &str,
    language: highlight::Language,
    mark: Option<Range<usize>>,
    window: &mut Window,
) -> Option<ShapedLine> {
    if text.is_empty() {
        return None;
    }
    let font = editor_font();
    let runs = text_runs(text, language, mark, &font);
    Some(window.text_system().shape_line(
        SharedString::from(text.to_string()),
        px(FONT_PX),
        &runs,
        None,
    ))
}

fn text_runs(
    text: &str,
    language: highlight::Language,
    mark: Option<Range<usize>>,
    font: &Font,
) -> Vec<TextRun> {
    let plain: Hsla = theme::text().into();
    let tokens = highlight::highlight_line(text, language);
    let mut runs = Vec::new();
    let mut cursor = 0;
    for token in tokens {
        if token.end <= cursor || token.start >= text.len() {
            continue;
        }
        let start = token.start.max(cursor).min(text.len());
        let end = token.end.min(text.len()).max(start);
        if start > cursor {
            push_runs(&mut runs, cursor..start, plain, mark.as_ref(), font);
        }
        if end > start {
            push_runs(
                &mut runs,
                start..end,
                token_color(token.kind),
                mark.as_ref(),
                font,
            );
        }
        cursor = end;
    }
    if cursor < text.len() {
        push_runs(&mut runs, cursor..text.len(), plain, mark.as_ref(), font);
    }
    if runs.is_empty() {
        runs.push(text_run(text.len(), plain, font, false));
    }
    runs
}

fn token_color(kind: highlight::TokenKind) -> Hsla {
    match kind {
        highlight::TokenKind::Comment => theme::comment().into(),
        highlight::TokenKind::String => theme::string().into(),
        highlight::TokenKind::Keyword => theme::keyword().into(),
        highlight::TokenKind::Number => theme::number().into(),
    }
}

fn push_runs(
    runs: &mut Vec<TextRun>,
    range: Range<usize>,
    color: Hsla,
    mark: Option<&Range<usize>>,
    font: &Font,
) {
    let start = range.start;
    let end = range.end;
    if end <= start {
        return;
    }
    let Some(mark) = mark else {
        runs.push(text_run(end - start, color, font, false));
        return;
    };
    let marked_start = mark.start.clamp(start, end);
    let marked_end = mark.end.clamp(marked_start, end);
    if marked_start > start {
        runs.push(text_run(marked_start - start, color, font, false));
    }
    if marked_end > marked_start {
        runs.push(text_run(marked_end - marked_start, color, font, true));
    }
    if end > marked_end {
        runs.push(text_run(end - marked_end, color, font, false));
    }
}

fn text_run(len: usize, color: Hsla, font: &Font, underline: bool) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: underline.then(|| UnderlineStyle {
            color: Some(color),
            thickness: px(1.0),
            wavy: false,
        }),
        strikethrough: None,
    }
}

fn editor_font() -> Font {
    let mut font = gpui::font("JetBrains Mono");
    font.fallbacks = Some(FontFallbacks::from_fonts(vec![
        "Iosevka".into(),
        "DejaVu Sans Mono".into(),
        "Noto Sans Mono".into(),
        "Liberation Mono".into(),
        ".SystemUIFont".into(),
    ]));
    font
}

fn reveal_offset(
    mut offset: Point<Pixels>,
    max: Point<Pixels>,
    item_left: Pixels,
    item_right: Pixels,
    item_top: Pixels,
    item_bottom: Pixels,
    viewport: Bounds<Pixels>,
) -> Point<Pixels> {
    if viewport.size.height > px(0.) {
        offset.y = reveal_span(
            offset.y,
            max.y,
            item_top,
            item_bottom,
            viewport.top(),
            viewport.bottom(),
        );
    }
    if viewport.size.width > px(0.) {
        offset.x = reveal_span(
            offset.x,
            max.x,
            item_left,
            item_right,
            viewport.left(),
            viewport.right(),
        );
    }
    offset
}

fn reveal_span(
    mut offset: Pixels,
    max_offset: Pixels,
    start: Pixels,
    end: Pixels,
    view_start: Pixels,
    view_end: Pixels,
) -> Pixels {
    if start < view_start {
        offset += view_start - start;
    } else if end > view_end {
        offset += view_end - end;
    }
    offset.clamp(-max_offset, px(0.))
}

fn apply_keyboard_nav(buffer: &mut Buffer, marked: &mut Option<Range<usize>>, nav: KeyboardNav) {
    // Clearing the mark ends the IME underline and keeps the next edit on the undo stack.
    *marked = None;
    match nav {
        KeyboardNav::Left(select) => buffer.move_left(select),
        KeyboardNav::Right(select) => buffer.move_right(select),
        KeyboardNav::Up(select) => buffer.move_up(select),
        KeyboardNav::Down(select) => buffer.move_down(select),
        KeyboardNav::LineStart(select) => buffer.move_line_start(select),
        KeyboardNav::LineEnd(select) => buffer.move_line_end(select),
        KeyboardNav::DocStart(select) => buffer.move_doc_start(select),
        KeyboardNav::DocEnd(select) => buffer.move_doc_end(select),
        KeyboardNav::Vertical(lines, select) => buffer.move_vertical_by(lines, select),
        KeyboardNav::WordLeft(select) => buffer.move_word_left(select),
        KeyboardNav::WordRight(select) => buffer.move_word_right(select),
        KeyboardNav::SelectAll => buffer.select_all(),
    }
}

fn ordered_range(range: Range<usize>) -> Range<usize> {
    if range.start <= range.end {
        range
    } else {
        range.end..range.start
    }
}

fn utf16_prefix_bytes(text: &str, utf16_offset: usize) -> usize {
    let mut seen = 0;
    for (index, ch) in text.char_indices() {
        if seen >= utf16_offset {
            return index;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::{KeyboardNav, apply_keyboard_nav, ordered_range, reveal_offset};
    use crate::buffer::Buffer;
    use gpui::{Bounds, point, px, size};

    #[test]
    fn keyboard_motion_and_select_all_clear_the_ime_mark() {
        let mut buffer = Buffer::new("alpha beta\nsecond line");
        buffer.move_right(false);
        let motions = [
            KeyboardNav::Left(false),
            KeyboardNav::Right(false),
            KeyboardNav::Up(false),
            KeyboardNav::Down(false),
            KeyboardNav::Left(true),
            KeyboardNav::Right(true),
            KeyboardNav::Up(true),
            KeyboardNav::Down(true),
            KeyboardNav::LineStart(false),
            KeyboardNav::LineEnd(false),
            KeyboardNav::LineStart(true),
            KeyboardNav::LineEnd(true),
            KeyboardNav::DocStart(false),
            KeyboardNav::DocEnd(false),
            KeyboardNav::DocStart(true),
            KeyboardNav::DocEnd(true),
            KeyboardNav::Vertical(-4, false),
            KeyboardNav::Vertical(4, true),
            KeyboardNav::WordLeft(false),
            KeyboardNav::WordRight(false),
            KeyboardNav::WordLeft(true),
            KeyboardNav::WordRight(true),
            KeyboardNav::SelectAll,
        ];
        for motion in motions {
            let mut marked = Some(0..1);
            apply_keyboard_nav(&mut buffer, &mut marked, motion);
            assert!(marked.is_none());
        }
        assert_eq!(buffer.selection(), 0..buffer.len());
    }

    #[test]
    fn inverted_utf16_text_range_does_not_panic() {
        let buffer = Buffer::new("a😀b");
        let forwards = buffer.range_from_utf16(&(1..3));
        let backwards = buffer.range_from_utf16(&(3..1));
        assert_eq!(forwards, backwards);
        let text = buffer
            .text()
            .get(ordered_range(backwards))
            .unwrap_or("")
            .to_string();
        assert_eq!(text, "😀");
        assert_eq!(buffer.text().get(ordered_range(6..1)).unwrap_or(""), "😀b");
    }

    #[test]
    fn reveal_offset_scrolls_horizontally_when_the_caret_is_outside() {
        let viewport = Bounds::new(point(px(0.), px(0.)), size(px(200.), px(100.)));
        let max = point(px(800.), px(400.));
        let next = reveal_offset(
            point(px(0.), px(0.)),
            max,
            px(400.),
            px(402.),
            px(8.),
            px(30.),
            viewport,
        );
        assert_eq!(next.x, px(-202.));
        assert_eq!(next.y, px(0.));

        let back = reveal_offset(
            point(px(-300.), px(0.)),
            max,
            px(-290.),
            px(-288.),
            px(8.),
            px(30.),
            viewport,
        );
        assert_eq!(back.x, px(-10.));

        let parked = reveal_offset(
            point(px(-40.), px(-12.)),
            max,
            px(80.),
            px(82.),
            px(20.),
            px(42.),
            viewport,
        );
        assert_eq!(parked.x, px(-40.));
        assert_eq!(parked.y, px(-12.));

        let clamped = reveal_offset(
            point(px(0.), px(0.)),
            point(px(10.), px(10.)),
            px(400.),
            px(402.),
            px(8.),
            px(30.),
            viewport,
        );
        assert_eq!(clamped.x, px(-10.));
    }
}
