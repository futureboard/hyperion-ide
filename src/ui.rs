use std::rc::Rc;

use gpui::{
    App, ClickEvent, CursorStyle, IntoElement, MouseDownEvent, SharedString, Window, div,
    prelude::*, px,
};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuId {
    File,
    Edit,
}

pub struct TabItem {
    pub id: u64,
    pub title: String,
    pub dirty: bool,
    pub active: bool,
}

pub struct StatusInfo {
    pub message: String,
    pub path: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub language: String,
    pub encoding: String,
    pub dirty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindAction {
    Needle,
    Replacement,
    ReplaceOne,
    ReplaceAll,
}

pub enum MenuEntry {
    Item {
        id: &'static str,
        label: &'static str,
        shortcut: String,
    },
    Separator,
}

pub fn primary_shortcut(chord: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{chord}")
    } else {
        format!("Ctrl+{chord}")
    }
}

pub fn menu_bar(
    on_file: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_edit: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .h(px(34.))
        .w_full()
        .flex()
        .items_center()
        .bg(theme::chrome_bg())
        .border_b_1()
        .border_color(theme::border())
        .text_size(px(13.))
        .child(
            div()
                .w(px(92.))
                .pl_3()
                .text_color(theme::text())
                .child("Hyperion"),
        )
        .child(menu_button("menu-file", "File", on_file))
        .child(menu_button("menu-edit", "Edit", on_edit))
}

pub fn menu_layer(
    left: f32,
    entries: Vec<MenuEntry>,
    on_pick: impl Fn(&str, &mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    div()
        .absolute()
        .size_full()
        .top_0()
        .left_0()
        .child(
            div()
                .size_full()
                .absolute()
                .top_0()
                .left_0()
                .on_mouse_down(gpui::MouseButton::Left, on_dismiss),
        )
        .child(
            div()
                .absolute()
                .top(px(34.))
                .left(px(left))
                .w(px(248.))
                .py_1()
                .bg(theme::menu_bg())
                .border_1()
                .border_color(theme::border())
                .shadow_lg()
                .flex()
                .flex_col()
                .children(
                    entries
                        .into_iter()
                        .map(|entry| menu_row(entry, on_pick.clone())),
                ),
        )
}

pub fn tab_strip(
    tabs: Vec<TabItem>,
    on_select: impl Fn(u64, &mut Window, &mut App) + 'static,
    on_close: impl Fn(u64, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_select = Rc::new(on_select);
    let on_close = Rc::new(on_close);
    div()
        .id("tab-strip")
        .h(px(36.))
        .w_full()
        .flex()
        .flex_row()
        .overflow_x_scroll()
        .bg(theme::chrome_bg())
        .border_b_1()
        .border_color(theme::border())
        .children(tabs.into_iter().map(|tab| {
            let select = on_select.clone();
            let close = on_close.clone();
            let id = tab.id;
            let active = tab.active;
            let title: SharedString = tab.title.into();
            div()
                .id(("tab", id as usize))
                .h_full()
                .min_w(px(120.))
                .max_w(px(220.))
                .flex()
                .flex_col()
                .flex_shrink_0()
                .bg(if active {
                    theme::editor_bg()
                } else {
                    theme::chrome_bg()
                })
                .border_r_1()
                .border_color(theme::border())
                .hover(move |style| {
                    style
                        .bg(if active {
                            theme::editor_bg()
                        } else {
                            theme::hover()
                        })
                        .cursor_pointer()
                })
                .on_click(move |_, window, cx| select(id, window, cx))
                .child(div().h(px(2.)).w_full().flex_shrink_0().bg(if active {
                    theme::accent()
                } else {
                    gpui::rgba(0x00000000)
                }))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().size(px(6.)).rounded_full().bg(if tab.dirty {
                            theme::accent()
                        } else {
                            gpui::rgba(0x00000000)
                        }))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(13.))
                                .child(title),
                        )
                        .child(
                            div()
                                .id(("tab-close", id as usize))
                                .px_1()
                                .text_color(theme::faint())
                                .hover(|style| style.text_color(theme::text()).cursor_pointer())
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    close(id, window, cx);
                                })
                                .child("×"),
                        ),
                )
        }))
}

pub fn empty_state(folder_name: impl Into<SharedString>) -> impl IntoElement {
    let folder_name = folder_name.into();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .bg(theme::editor_bg())
        .child(
            div()
                .max_w_full()
                .px_6()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(20.))
                .text_color(theme::text())
                .child(folder_name),
        )
        .child(
            div()
                .text_color(theme::muted())
                .child("Pick a file from the left."),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_4()
                .text_size(px(12.))
                .text_color(theme::faint())
                .child(format!("{}  Open File", primary_shortcut("O")))
                .child(format!("{}  Open Folder", primary_shortcut("Shift+O")))
                .child(format!("{}  New File", primary_shortcut("N")))
                .child(format!("{}  Save", primary_shortcut("S"))),
        )
}

pub fn find_bar(
    needle: SharedString,
    replacement: SharedString,
    needle_active: bool,
    replacement_active: bool,
    on_action: impl Fn(FindAction, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_action = Rc::new(on_action);
    let needle_action = on_action.clone();
    let replacement_action = on_action.clone();
    let replace_one = on_action.clone();
    let replace_all = on_action;
    div()
        .id("find-bar")
        .h(px(36.))
        .w_full()
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .bg(theme::chrome_bg())
        .border_b_1()
        .border_color(theme::border())
        .text_size(px(13.))
        .child(find_slot(
            "find-needle",
            "Find",
            needle,
            needle_active,
            move |_, window, cx| needle_action(FindAction::Needle, window, cx),
        ))
        .child(find_slot(
            "find-replacement",
            "Replace",
            replacement,
            replacement_active,
            move |_, window, cx| replacement_action(FindAction::Replacement, window, cx),
        ))
        .child(find_button(
            "find-replace",
            "Replace",
            move |_, window, cx| replace_one(FindAction::ReplaceOne, window, cx),
        ))
        .child(find_button(
            "find-replace-all",
            "Replace All",
            move |_, window, cx| replace_all(FindAction::ReplaceAll, window, cx),
        ))
}

fn find_slot(
    id: &'static str,
    label: &'static str,
    value: SharedString,
    active: bool,
    on_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_w(px(0.))
        .h(px(24.))
        .flex()
        .items_center()
        .gap_2()
        .on_mouse_down(gpui::MouseButton::Left, on_down)
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme::muted())
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(24.))
                .px_2()
                .flex()
                .items_center()
                .overflow_hidden()
                .bg(theme::editor_bg())
                .border_1()
                .border_color(if active {
                    theme::accent()
                } else {
                    theme::border()
                })
                .cursor(CursorStyle::IBeam)
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(theme::text())
                        .child(value),
                )
                .when(active, |field| {
                    field.child(
                        div()
                            .w(px(1.))
                            .h(px(14.))
                            .flex_shrink_0()
                            .bg(theme::accent()),
                    )
                }),
        )
}

fn find_button(
    id: &'static str,
    label: &'static str,
    on_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .h(px(24.))
        .px_2()
        .flex_shrink_0()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .text_color(theme::text())
        .hover(|style| style.bg(theme::hover()).cursor_pointer())
        .on_mouse_down(gpui::MouseButton::Left, on_down)
        .child(label)
}

pub fn status_bar(info: StatusInfo) -> impl IntoElement {
    let position = match (info.line, info.column) {
        (Some(line), Some(column)) => format!("Ln {line}, Col {column}"),
        _ => "Ln —, Col —".to_string(),
    };
    let path = if info.path.is_empty() {
        "No file".to_string()
    } else {
        info.path
    };
    let encoding = info.encoding;
    let show_encoding = encoding != "UTF-8";
    div()
        .h(px(26.))
        .w_full()
        .px_3()
        .flex()
        .items_center()
        .bg(theme::status_bg())
        .border_t_1()
        .border_color(theme::border())
        .text_size(px(12.))
        .text_color(theme::muted())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(theme::text())
                .child(path),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .justify_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(theme::faint())
                .child(info.message),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .justify_end()
                .items_center()
                .gap_3()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(position)
                .child(info.language)
                .when(show_encoding, move |row| row.child(encoding))
                .when(info.dirty, |row| row.child("Modified")),
        )
}

pub fn activity_rail() -> impl IntoElement {
    div()
        .w(px(48.))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .pt_2()
        .bg(theme::rail_bg())
        .border_r_1()
        .border_color(theme::border())
        .child(
            div()
                .id("activity-files")
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::line_hl())
                .child(file_mark()),
        )
}

fn file_mark() -> impl IntoElement {
    div()
        .relative()
        .w(px(14.))
        .h(px(16.))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .border_1()
                .border_color(theme::text()),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .size(px(5.))
                .bg(theme::line_hl())
                .border_l_1()
                .border_b_1()
                .border_color(theme::text()),
        )
}

fn menu_button(
    id: &'static str,
    label: &'static str,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(64.))
        .h(px(34.))
        .flex()
        .items_center()
        .justify_center()
        .hover(|style| style.bg(theme::hover()).cursor_pointer())
        .on_click(click)
        .child(label)
}

fn menu_row(
    entry: MenuEntry,
    on_pick: Rc<impl Fn(&str, &mut Window, &mut App) + 'static>,
) -> gpui::AnyElement {
    match entry {
        MenuEntry::Separator => div()
            .h(px(1.))
            .my_1()
            .mx_2()
            .bg(theme::border())
            .into_any_element(),
        MenuEntry::Item {
            id,
            label,
            shortcut,
        } => div()
            .id(id)
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .hover(|style| style.bg(theme::hover()).cursor_pointer())
            .on_click(move |_, window, cx| on_pick(id, window, cx))
            .child(label)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::faint())
                    .child(shortcut),
            )
            .into_any_element(),
    }
}
