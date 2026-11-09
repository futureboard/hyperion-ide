//! Commands the shell can already run, and the palette state that filters them.
//!
//! Ids are the strings dispatched today: `new-file`, `open-file`, `open-folder`,
//! `save`, `close`, `quit`, `undo`, `redo`, `cut`, `copy`, `paste`, `select-all`, `find`.
#![allow(dead_code)]

use std::rc::Rc;

use gpui::{AnyElement, App, IntoElement, SharedString, Window, div, prelude::*, px};

use crate::theme;
use crate::ui;

/// How many command ids [`CommandRegistry::record`] keeps.
pub const RECENT_LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub category: String,
    pub shortcut: Option<String>,
}

impl Command {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        category: impl Into<String>,
        shortcut: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            category: category.into(),
            shortcut,
        }
    }
}

#[derive(Debug)]
pub struct CommandRegistry {
    commands: Vec<Command>,
    recent: Vec<String>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            recent: Vec::new(),
        }
    }

    /// File and Edit actions that already have a menu row and a key binding.
    pub fn builtin() -> Self {
        let mut registry = Self::new();
        let file = [
            ("new-file", "New File", "N"),
            ("open-file", "Open File", "O"),
            ("open-folder", "Open Folder", "Shift+O"),
            ("save", "Save", "S"),
            ("close", "Close Tab", "W"),
            ("quit", "Quit", "Q"),
        ];
        for (id, title, chord) in file {
            registry.register(Command::new(
                id,
                title,
                "File",
                Some(ui::primary_shortcut(chord)),
            ));
        }
        let edit = [
            ("undo", "Undo", "Z"),
            ("redo", "Redo", "Shift+Z"),
            ("cut", "Cut", "X"),
            ("copy", "Copy", "C"),
            ("paste", "Paste", "V"),
            ("select-all", "Select All", "A"),
            ("find", "Find", "F"),
        ];
        for (id, title, chord) in edit {
            registry.register(Command::new(
                id,
                title,
                "Edit",
                Some(ui::primary_shortcut(chord)),
            ));
        }
        registry
    }

    /// Append `command`, or replace the fields when the id is already registered.
    pub fn register(&mut self, command: Command) {
        if let Some(existing) = self.commands.iter_mut().find(|item| item.id == command.id) {
            *existing = command;
            return;
        }
        self.commands.push(command);
    }

    pub fn list(&self) -> &[Command] {
        &self.commands
    }

    /// Subsequence match on the title and the id. Comparison is case-insensitive.
    /// An empty or blank query returns every command in registration order.
    pub fn filter(&self, query: &str) -> Vec<Command> {
        let query = query.trim();
        if query.is_empty() {
            return self.commands.clone();
        }
        let mut ranked = Vec::new();
        for (index, command) in self.commands.iter().enumerate() {
            if let Some(score) = match_score(query, command) {
                ranked.push((score, index));
            }
        }
        ranked.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        ranked
            .into_iter()
            .map(|(_, index)| self.commands[index].clone())
            .collect()
    }

    /// Remember `id` as the most recent command. Unknown ids are ignored.
    /// A repeated id moves to the front. The list stays within [`RECENT_LIMIT`].
    pub fn record(&mut self, id: &str) -> bool {
        if self.commands.iter().all(|command| command.id != id) {
            return false;
        }
        self.recent.retain(|existing| existing != id);
        self.recent.insert(0, id.to_string());
        self.recent.truncate(RECENT_LIMIT);
        true
    }

    pub fn recent(&self) -> &[String] {
        &self.recent
    }
}

/// Query and selection for the palette. The shell applies [`CommandRegistry::filter`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    query: String,
    selected: usize,
}

impl Palette {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Replace the query. A real change selects the first visible row again.
    pub fn set_query(&mut self, query: impl Into<String>) {
        let query = query.into();
        if query == self.query {
            return;
        }
        self.query = query;
        self.selected = 0;
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn move_up(&mut self, visible: usize) {
        self.clamp(visible);
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self, visible: usize) {
        self.clamp(visible);
        if visible > 0 && self.selected + 1 < visible {
            self.selected += 1;
        }
    }

    /// Pull the selection back into `0..visible`, or to `0` when nothing is visible.
    pub fn clamp(&mut self, visible: usize) {
        if visible == 0 {
            self.selected = 0;
            return;
        }
        if self.selected >= visible {
            self.selected = visible - 1;
        }
    }

    pub fn selected_id<'a>(&self, visible: &'a [Command]) -> Option<&'a str> {
        visible
            .get(self.selected)
            .map(|command| command.id.as_str())
    }
}

/// Paints the current query and rows. Clicks report the command id.
/// The shell owns [`Palette`].
///
/// The selected title is the only accent, so the row that would run is the one mark.
pub fn command_palette(
    query: SharedString,
    rows: Vec<Command>,
    selected: usize,
    on_pick: impl Fn(&str, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    let body: Vec<AnyElement> = if rows.is_empty() {
        vec![
            div()
                .h(px(32.))
                .px_3()
                .flex()
                .items_center()
                .text_color(theme::muted())
                .child("No matching commands")
                .into_any_element(),
        ]
    } else {
        rows.into_iter()
            .enumerate()
            .map(|(index, command)| palette_row(command, index == selected, on_pick.clone()))
            .collect()
    };
    div()
        .w(px(420.))
        .bg(theme::chrome_bg())
        .border_1()
        .border_color(theme::border())
        .flex()
        .flex_col()
        .overflow_hidden()
        .text_size(px(13.))
        .child(palette_query(&query))
        .children(body)
}

fn palette_query(query: &str) -> impl IntoElement {
    let (label, color) = if query.is_empty() {
        ("Commands", theme::muted())
    } else {
        (query, theme::text())
    };
    div()
        .h(px(32.))
        .px_3()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(theme::border())
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(color)
        .child(label.to_string())
}

fn palette_row(
    command: Command,
    selected: bool,
    on_pick: Rc<impl Fn(&str, &mut Window, &mut App) + 'static>,
) -> AnyElement {
    let id = command.id.clone();
    let pick = on_pick.clone();
    let title_color = if selected {
        theme::accent()
    } else {
        theme::text()
    };
    let mut tail = vec![
        div()
            .text_color(theme::muted())
            .child(command.category.clone())
            .into_any_element(),
    ];
    if let Some(shortcut) = command.shortcut.clone() {
        tail.push(
            div()
                .text_color(theme::muted())
                .whitespace_nowrap()
                .child(shortcut)
                .into_any_element(),
        );
    }
    let mut row = div()
        .id(format!("palette-{}", command.id))
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .hover(|style| style.bg(theme::hover()).cursor_pointer())
        .on_click(move |_, window, cx| pick(&id, window, cx));
    if selected {
        row = row.bg(theme::hover());
    }
    row.child(
        div()
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_color(title_color)
            .child(command.title.clone()),
    )
    .child(div().flex().items_center().gap_3().children(tail))
    .into_any_element()
}

fn match_score(query: &str, command: &Command) -> Option<i32> {
    match (
        fuzzy_score(query, &command.title),
        fuzzy_score(query, &command.id),
    ) {
        (Some(title), Some(id)) => Some(title.max(id)),
        (Some(title), None) => Some(title),
        (None, Some(id)) => Some(id),
        (None, None) => None,
    }
}

/// Filename-style subsequence score. Tighter, earlier, word-start runs rank higher.
fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let query: Vec<char> = query.chars().collect();
    if query.is_empty() {
        return Some(0);
    }
    let candidate: Vec<char> = candidate.chars().collect();
    let mut query_index = 0;
    let mut score = 0;
    let mut previous: Option<usize> = None;
    let mut streak = 0;
    let mut matched = false;

    for (index, ch) in candidate.iter().copied().enumerate() {
        if query_index >= query.len() {
            break;
        }
        if !same_char(query[query_index], ch) {
            continue;
        }
        let on_boundary = index == 0 || is_boundary(candidate[index - 1]);
        let mut gain = 1;
        if on_boundary {
            gain += 8;
        }
        if index == 0 {
            gain += 4;
        }
        if let Some(previous_index) = previous {
            if index == previous_index + 1 {
                streak += 1;
                gain += 6 + streak * 2;
            } else {
                streak = 0;
                let gap = (index - previous_index - 1) as i32;
                gain -= gap.min(6);
            }
        }
        if !matched {
            gain -= index as i32;
            matched = true;
        }
        score += gain;
        previous = Some(index);
        query_index += 1;
    }

    if query_index == query.len() {
        Some(score)
    } else {
        None
    }
}

fn same_char(query: char, candidate: char) -> bool {
    query.to_lowercase().eq(candidate.to_lowercase())
}

fn is_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '-' | '_' | '/' | '.' | '\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(commands: &[Command]) -> Vec<&str> {
        commands.iter().map(|command| command.id.as_str()).collect()
    }

    #[test]
    fn fuzzy_ranking_prefers_tighter_earlier_matches() {
        let registry = CommandRegistry::builtin();
        assert_eq!(
            ids(&registry.filter("sa")),
            vec!["save", "select-all", "close"]
        );
        assert_eq!(ids(&registry.filter("FILE")), vec!["new-file", "open-file"]);
        assert_eq!(ids(&registry.filter("opfi")), vec!["open-file"]);
        assert!(registry.filter("git").is_empty());
        assert!(registry.filter("format").is_empty());
        assert!(registry.filter("terminal").is_empty());
    }

    #[test]
    fn empty_query_returns_the_seeded_commands_in_registration_order() {
        let mut registry = CommandRegistry::builtin();
        let expected = [
            ("new-file", "New File", "File", "N"),
            ("open-file", "Open File", "File", "O"),
            ("open-folder", "Open Folder", "File", "Shift+O"),
            ("save", "Save", "File", "S"),
            ("close", "Close Tab", "File", "W"),
            ("quit", "Quit", "File", "Q"),
            ("undo", "Undo", "Edit", "Z"),
            ("redo", "Redo", "Edit", "Shift+Z"),
            ("cut", "Cut", "Edit", "X"),
            ("copy", "Copy", "Edit", "C"),
            ("paste", "Paste", "Edit", "V"),
            ("select-all", "Select All", "Edit", "A"),
            ("find", "Find", "Edit", "F"),
        ];
        assert_eq!(registry.list().len(), expected.len());
        for (command, (id, title, category, chord)) in registry.list().iter().zip(expected) {
            assert_eq!(command.id, id);
            assert_eq!(command.title, title);
            assert_eq!(command.category, category);
            assert_eq!(command.shortcut, Some(ui::primary_shortcut(chord)));
        }
        assert_eq!(ids(&registry.filter("   ")), ids(registry.list()));
        assert!(registry.record("quit"));
        assert_eq!(
            registry
                .filter("")
                .first()
                .map(|command| command.id.as_str()),
            Some("new-file")
        );
        assert_eq!(registry.recent(), &["quit".to_string()][..]);
    }

    #[test]
    fn selection_clamps_when_the_visible_list_shrinks() {
        let registry = CommandRegistry::builtin();
        let mut palette = Palette::new();
        let all = registry.filter(palette.query());
        assert_eq!(palette.selected_index(), 0);
        assert_eq!(palette.selected_id(&all), Some("new-file"));

        for _ in 0..30 {
            palette.move_down(all.len());
        }
        assert_eq!(palette.selected_index(), all.len() - 1);
        assert_eq!(palette.selected_id(&all), Some("find"));

        for _ in 0..30 {
            palette.move_up(all.len());
        }
        assert_eq!(palette.selected_index(), 0);

        palette.move_down(3);
        palette.move_down(3);
        palette.move_down(3);
        assert_eq!(palette.selected_index(), 2);
        palette.clamp(1);
        assert_eq!(palette.selected_index(), 0);
        assert_eq!(palette.selected_id(&all[..1]), Some("new-file"));

        palette.clamp(0);
        assert_eq!(palette.selected_index(), 0);
        assert_eq!(palette.selected_id(&[]), None);
        palette.move_down(0);
        palette.move_up(0);
        assert_eq!(palette.selected_index(), 0);

        palette.set_query("sa");
        let hits = registry.filter(palette.query());
        assert_eq!(palette.query(), "sa");
        assert_eq!(palette.selected_index(), 0);
        assert_eq!(palette.selected_id(&hits), Some("save"));
        palette.move_down(hits.len());
        palette.set_query("sa");
        assert_eq!(palette.selected_index(), 1);
        palette.move_down(hits.len());
        assert_eq!(palette.selected_index(), hits.len() - 1);
        palette.set_query("file");
        assert_eq!(palette.selected_index(), 0);
        assert_eq!(
            palette.selected_id(&registry.filter(palette.query())),
            Some("new-file")
        );
    }

    #[test]
    fn recent_order_is_most_recent_first_and_capped() {
        let mut registry = CommandRegistry::builtin();
        assert!(registry.record("copy"));
        assert!(registry.record("paste"));
        assert!(registry.record("copy"));
        assert!(!registry.record("format"));
        assert!(!registry.record("go-to-definition"));
        assert_eq!(
            registry.recent(),
            &["copy".to_string(), "paste".to_string()][..]
        );

        let mut registry = CommandRegistry::new();
        let count = RECENT_LIMIT + 3;
        for n in 0..count {
            registry.register(Command::new(
                format!("cmd-{n}"),
                format!("Command {n}"),
                "Test",
                None,
            ));
        }
        for n in 0..count {
            assert!(registry.record(&format!("cmd-{n}")));
        }
        assert_eq!(registry.recent().len(), RECENT_LIMIT);
        assert_eq!(registry.recent()[0], format!("cmd-{}", count - 1));
        assert_eq!(
            registry.recent()[RECENT_LIMIT - 1],
            format!("cmd-{}", count - RECENT_LIMIT)
        );
    }
}
