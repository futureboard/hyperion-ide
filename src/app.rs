use std::fs;
use std::path::{Path, PathBuf};

use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Menu, MenuItem,
    MouseDownEvent, PathPromptOptions, PromptLevel, Render, Window, WindowHandle, actions, div,
    prelude::*, px,
};

use crate::editor::CodeEditor;
use crate::theme;
use crate::ui::{self, MenuEntry, MenuId, StatusInfo, TabItem};
use crate::workspace::{FileTree, display_path, file_title, language_name, same_file};

const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

actions!(
    hyperion,
    [
        OpenFile, OpenFolder, NewFile, Save, CloseTab, Quit, CloseMenu
    ]
);

pub fn install_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu::new("HyperionIDE").items([
            MenuItem::separator(),
            MenuItem::action("Quit HyperionIDE", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New File", NewFile),
            MenuItem::action("Open File…", OpenFile),
            MenuItem::action("Open Folder…", OpenFolder),
            MenuItem::separator(),
            MenuItem::action("Save", Save),
            MenuItem::action("Close Tab", CloseTab),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", crate::editor::Undo),
            MenuItem::action("Redo", crate::editor::Redo),
            MenuItem::separator(),
            MenuItem::action("Cut", crate::editor::Cut),
            MenuItem::action("Copy", crate::editor::Copy),
            MenuItem::action("Paste", crate::editor::Paste),
            MenuItem::action("Select All", crate::editor::SelectAll),
        ]),
    ]);
}

struct Tab {
    id: u64,
    path: Option<PathBuf>,
    title: String,
    lossy: bool,
    editor: Entity<CodeEditor>,
}

pub struct Ide {
    focus: FocusHandle,
    root: Option<PathBuf>,
    tree: FileTree,
    tabs: Vec<Tab>,
    active: Option<usize>,
    next_id: u64,
    untitled: u64,
    status: String,
    menu: Option<MenuId>,
    dialog: bool,
}

impl Ide {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let root = std::env::current_dir().ok();
        let tree = match &root {
            Some(path) => FileTree::open(path.clone()),
            None => FileTree::empty(),
        };
        let status = tree.error.clone().unwrap_or_else(|| "Ready".to_string());
        Self {
            focus: cx.focus_handle(),
            root,
            tree,
            tabs: Vec::new(),
            active: None,
            next_id: 1,
            untitled: 1,
            status,
            menu: None,
            dialog: false,
        }
    }

    pub fn focus_root(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    fn on_open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.open_file_dialog(window, cx);
    }

    fn on_open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.open_folder_dialog(window, cx);
    }

    fn on_new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.new_file(window, cx);
    }

    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.save_active(window, cx);
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(ix) = self.active {
            let id = self.tabs[ix].id;
            self.close_tab(id, window, cx);
        }
    }

    fn on_quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.quit(window, cx);
    }

    fn on_close_menu(&mut self, _: &CloseMenu, _: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    fn toggle_menu(&mut self, menu: MenuId, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }

    fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        match id {
            "new-file" => self.new_file(window, cx),
            "open-file" => self.open_file_dialog(window, cx),
            "open-folder" => self.open_folder_dialog(window, cx),
            "save" => self.save_active(window, cx),
            "close" => self.on_close_tab(&CloseTab, window, cx),
            "quit" => self.quit(window, cx),
            "undo" => self.with_editor(window, cx, |editor, window, cx| editor.undo(window, cx)),
            "redo" => self.with_editor(window, cx, |editor, window, cx| editor.redo(window, cx)),
            "cut" => self.with_editor(window, cx, |editor, window, cx| editor.cut(window, cx)),
            "copy" => self.with_editor(window, cx, |editor, _, cx| editor.copy(cx)),
            "paste" => self.with_editor(window, cx, |editor, window, cx| editor.paste(window, cx)),
            "select-all" => self.with_editor(window, cx, |editor, _, cx| editor.select_all(cx)),
            _ => cx.notify(),
        }
    }

    fn with_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut CodeEditor, &mut Window, &mut Context<CodeEditor>),
    ) {
        let Some(ix) = self.active else {
            self.status = "No file open".into();
            cx.notify();
            return;
        };
        self.tabs[ix]
            .editor
            .update(cx, |editor, cx| f(editor, window, cx));
    }

    fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = if self.untitled == 1 {
            self.untitled += 1;
            "untitled".to_string()
        } else {
            let title = format!("untitled {}", self.untitled);
            self.untitled += 1;
            title
        };
        let editor = cx.new(|cx| CodeEditor::new(String::new(), cx));
        let id = self.alloc_id();
        self.tabs.push(Tab {
            id,
            path: None,
            title,
            lossy: false,
            editor,
        });
        self.active = Some(self.tabs.len() - 1);
        self.status = "New file".into();
        self.focus_active(window, cx);
        cx.notify();
    }

    fn open_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.claim_dialog(cx) {
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, async_cx| {
            let picked = rx.await;
            this.update_in(async_cx, |this, window, cx| {
                this.dialog = false;
                match picked {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.open_path(path, window, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.status = format!("Open file failed: {error}"),
                    Err(_) => this.status = "Open file failed".into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_folder_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.claim_dialog(cx) {
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, async_cx| {
            let picked = rx.await;
            this.update_in(async_cx, |this, _, cx| {
                this.dialog = false;
                match picked {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.tree.set_root(path.clone());
                            this.root = Some(path);
                            this.status = this
                                .tree
                                .error
                                .clone()
                                .unwrap_or_else(|| "Opened folder".into());
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.status = format!("Open folder failed: {error}"),
                    Err(_) => this.status = "Open folder failed".into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|tab| {
            tab.path
                .as_deref()
                .is_some_and(|open| same_file(open, &path))
        }) {
            self.active = Some(ix);
            self.tree.expand_to(&path);
            self.focus_active(window, cx);
            cx.notify();
            return;
        }
        match read_file(&path) {
            Ok((text, lossy)) => {
                let title = file_title(&path);
                let editor = cx.new(|cx| {
                    let mut editor = CodeEditor::new(text, cx);
                    editor.set_language_from_path(&path);
                    editor
                });
                let id = self.alloc_id();
                self.tabs.push(Tab {
                    id,
                    path: Some(path.clone()),
                    title: title.clone(),
                    lossy,
                    editor,
                });
                self.active = Some(self.tabs.len() - 1);
                self.status = if lossy {
                    format!("Opened {title} with lossy UTF-8")
                } else {
                    format!("Opened {title}")
                };
                self.tree.expand_to(&path);
                self.focus_active(window, cx);
            }
            Err(error) => self.status = error,
        }
        cx.notify();
    }

    fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else {
            self.status = "Nothing to save".into();
            cx.notify();
            return;
        };
        if self.tabs[ix].path.is_none() {
            self.save_as(ix, false, window, cx);
        } else if self.write_tab(ix, cx) {
            cx.notify();
        } else {
            cx.notify();
        }
    }

    fn save_as(
        &mut self,
        ix: usize,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dialog && !close_after {
            self.status = "Finish the open dialog first".into();
            cx.notify();
            return;
        }
        self.dialog = true;
        let directory = self
            .root
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let suggested = self.tabs[ix].title.clone();
        let tab_id = self.tabs[ix].id;
        let rx = cx.prompt_for_new_path(&directory, Some(&suggested));
        cx.spawn_in(window, async move |this, async_cx| {
            let picked = rx.await;
            this.update_in(async_cx, |this, window, cx| {
                this.dialog = false;
                let Some(ix) = this.index_of(tab_id) else {
                    cx.notify();
                    return;
                };
                match picked {
                    Ok(Ok(Some(path))) => {
                        this.tabs[ix].path = Some(path.clone());
                        this.tabs[ix].title = file_title(&path);
                        if this.write_tab(ix, cx) && close_after {
                            this.remove_tab(ix, window, cx);
                        }
                        this.tree.refresh();
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.status = format!("Save failed: {error}"),
                    Err(_) => this.status = "Save failed".into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn write_tab(&mut self, ix: usize, cx: &mut Context<Self>) -> bool {
        let Some(path) = self.tabs[ix].path.clone() else {
            return false;
        };
        let text = self.tabs[ix].editor.read(cx).text().to_string();
        match fs::write(&path, text) {
            Ok(()) => {
                self.tabs[ix].editor.update(cx, |editor, cx| {
                    editor.mark_saved();
                    cx.notify();
                });
                self.tabs[ix].lossy = false;
                self.status = format!("Saved {}", display_path(self.root.as_deref(), &path));
                true
            }
            Err(error) => {
                self.status = format!("Save failed: {error}");
                false
            }
        }
    }

    fn close_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.index_of(id) else {
            return;
        };
        if !self.tabs[ix].editor.read(cx).is_dirty() {
            self.remove_tab(ix, window, cx);
            cx.notify();
            return;
        }
        if self.dialog {
            self.status = "Finish the open dialog first".into();
            cx.notify();
            return;
        }
        self.dialog = true;
        let label = self.tabs[ix].title.clone();
        let rx = window.prompt(
            PromptLevel::Warning,
            &format!("Save changes to {label}?"),
            None,
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, async_cx| {
            let answer = rx.await.unwrap_or(2);
            this.update_in(async_cx, |this, window, cx| {
                this.dialog = false;
                let Some(ix) = this.index_of(id) else {
                    return;
                };
                match answer {
                    0 => {
                        if this.tabs[ix].path.is_some() {
                            if this.write_tab(ix, cx) {
                                this.remove_tab(ix, window, cx);
                            }
                        } else {
                            this.save_as(ix, true, window, cx);
                        }
                    }
                    1 => this.remove_tab(ix, window, cx),
                    _ => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self.tabs.iter().any(|tab| tab.editor.read(cx).is_dirty());
        if !dirty {
            cx.quit();
            return;
        }
        if self.dialog {
            cx.quit();
            return;
        }
        self.dialog = true;
        let rx = window.prompt(
            PromptLevel::Warning,
            "Quit without saving?",
            Some("Unsaved changes will be lost."),
            &["Quit", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, async_cx| {
            let answer = rx.await.unwrap_or(1);
            this.update_in(async_cx, |this, _, cx| {
                this.dialog = false;
                if answer == 0 {
                    cx.quit();
                } else {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn activate_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.index_of(id) {
            self.active = Some(ix);
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn activate_entry(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.tree.entries.get(index).cloned() else {
            return;
        };
        if entry.is_dir {
            self.tree.toggle_dir(&entry.path);
        } else {
            self.open_path(entry.path, window, cx);
        }
        cx.notify();
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let closing_active = self.active == Some(ix);
        self.tabs.remove(ix);
        if self.tabs.is_empty() {
            self.active = None;
        } else if closing_active {
            self.active = Some(ix.min(self.tabs.len() - 1));
        } else if let Some(active) = self.active
            && active > ix
        {
            self.active = Some(active - 1);
        }
        self.status = "Closed tab".into();
        self.focus_active(window, cx);
    }

    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            let handle = self.tabs[ix].editor.read(cx).focus_handle();
            handle.focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
    }

    fn claim_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if self.dialog {
            self.status = "Finish the open dialog first".into();
            cx.notify();
            false
        } else {
            self.dialog = true;
            true
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn window_title(&self, cx: &App) -> String {
        match self.active.and_then(|ix| self.tabs.get(ix)) {
            Some(tab) => {
                let dirty = if tab.editor.read(cx).is_dirty() {
                    "*"
                } else {
                    ""
                };
                format!("{dirty}{} — HyperionIDE", tab.title)
            }
            None => "HyperionIDE".into(),
        }
    }

    fn status_info(&self, cx: &App) -> StatusInfo {
        if let Some(ix) = self.active {
            let tab = &self.tabs[ix];
            let editor = tab.editor.read(cx);
            let (line, column) = editor.cursor_position();
            StatusInfo {
                message: self.status.clone(),
                path: tab
                    .path
                    .as_ref()
                    .map(|path| display_path(self.root.as_deref(), path))
                    .unwrap_or_else(|| tab.title.clone()),
                line: Some(line),
                column: Some(column),
                language: language_name(tab.path.as_deref()).to_string(),
                encoding: if tab.lossy {
                    "UTF-8 (lossy)".into()
                } else {
                    "UTF-8".into()
                },
                dirty: editor.is_dirty(),
            }
        } else {
            StatusInfo {
                message: self.status.clone(),
                path: String::new(),
                line: None,
                column: None,
                language: "Plain Text".into(),
                encoding: "UTF-8".into(),
                dirty: false,
            }
        }
    }
}

impl Render for Ide {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&self.window_title(cx));
        let entity = cx.entity();
        let tabs: Vec<TabItem> = self
            .tabs
            .iter()
            .map(|tab| TabItem {
                id: tab.id,
                title: tab.title.clone(),
                dirty: tab.editor.read(cx).is_dirty(),
                active: self.active.is_some_and(|ix| self.tabs[ix].id == tab.id),
            })
            .collect();
        let status = self.status_info(cx);
        let folder_name = self
            .root
            .as_ref()
            .map(|path| file_title(path))
            .unwrap_or_else(|| "No folder".into());
        let menu = self.menu;
        let active_editor = self
            .active
            .and_then(|ix| self.tabs.get(ix))
            .map(|tab| tab.editor.clone());

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::window_bg())
            .text_color(theme::text())
            .font_family(".SystemUIFont")
            .text_size(px(13.))
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_open_file))
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_new_file))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_close_menu))
            .child(ui::menu_bar(
                cx.listener(|ide, _: &ClickEvent, _, cx| ide.toggle_menu(MenuId::File, cx)),
                cx.listener(|ide, _: &ClickEvent, _, cx| ide.toggle_menu(MenuId::Edit, cx)),
            ))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(ui::activity_rail())
                    .child(self.sidebar(cx))
                    .child(editor_column(
                        tabs,
                        active_editor,
                        folder_name,
                        entity.clone(),
                    )),
            )
            .child(ui::status_bar(status))
            .when(menu.is_some(), |this| {
                let left = if menu == Some(MenuId::Edit) {
                    156.0
                } else {
                    92.0
                };
                let entries = menu_entries(menu.unwrap_or(MenuId::File));
                let command_entity = entity.clone();
                let dismiss_entity = entity.clone();
                this.child(ui::menu_layer(
                    left,
                    entries,
                    move |id, window, cx| {
                        command_entity.update(cx, |ide, cx| ide.run_command(id, window, cx));
                    },
                    move |_: &MouseDownEvent, _, cx| {
                        dismiss_entity.update(cx, |ide, cx| {
                            ide.menu = None;
                            cx.notify();
                        });
                    },
                ))
            })
    }
}

impl Focusable for Ide {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn editor_column(
    tabs: Vec<TabItem>,
    editor: Option<Entity<CodeEditor>>,
    folder_name: String,
    entity: Entity<Ide>,
) -> impl IntoElement {
    let select_entity = entity.clone();
    let close_entity = entity;
    div()
        .flex_1()
        .h_full()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .when(!tabs.is_empty(), |column| {
            column.child(ui::tab_strip(
                tabs,
                move |id, window, cx| {
                    select_entity.update(cx, |ide, cx| ide.activate_tab(id, window, cx));
                },
                move |id, window, cx| {
                    close_entity.update(cx, |ide, cx| ide.close_tab(id, window, cx));
                },
            ))
        })
        .child(match editor {
            Some(editor) => div()
                .flex_1()
                .min_h(px(0.))
                .child(editor)
                .into_any_element(),
            None => ui::empty_state(folder_name).into_any_element(),
        })
}

impl Ide {
    fn sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let root_label = self
            .root
            .as_ref()
            .map(|path| file_title(path))
            .unwrap_or_else(|| "No folder".into());
        let count = self.tree.entries.len();
        let error = self.tree.error.clone();
        div()
            .w(px(260.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::sidebar_bg())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .h(px(36.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(13.))
                            .text_color(theme::text())
                            .child(root_label),
                    )
                    .child(
                        div()
                            .id("refresh-tree")
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(theme::muted())
                            .hover(|style| style.text_color(theme::text()).cursor_pointer())
                            .on_click(cx.listener(|ide, _: &ClickEvent, _, cx| {
                                ide.tree.refresh();
                                ide.status = "Refreshed folder".into();
                                cx.notify();
                            }))
                            .child("Refresh"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .when(error.is_some(), |pane| {
                        pane.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(12.))
                                .text_color(theme::text())
                                .child(error.clone().unwrap_or_default()),
                        )
                    })
                    .when(count == 0 && error.is_none(), |pane| {
                        pane.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(12.))
                                .text_color(theme::faint())
                                .child(if self.root.is_some() {
                                    "This folder is empty."
                                } else {
                                    "Open a folder to browse files."
                                }),
                        )
                    })
                    .when(count > 0, |pane| {
                        pane.child(
                            gpui::uniform_list(
                                "file-tree",
                                count,
                                cx.processor(|ide, range: std::ops::Range<usize>, _, cx| {
                                    let active = ide.active.and_then(|ix| {
                                        ide.tabs.get(ix).and_then(|tab| tab.path.clone())
                                    });
                                    let mut rows = Vec::new();
                                    for index in range {
                                        let Some(entry) = ide.tree.entries.get(index).cloned()
                                        else {
                                            continue;
                                        };
                                        let selected = active
                                            .as_ref()
                                            .is_some_and(|path| same_file(path, &entry.path));
                                        let mark = if entry.is_dir {
                                            if entry.expanded { "▾" } else { "▸" }
                                        } else {
                                            "·"
                                        };
                                        let name_color = if entry.name.starts_with('.') {
                                            theme::faint()
                                        } else if entry.is_dir {
                                            theme::text()
                                        } else {
                                            theme::muted()
                                        };
                                        let depth = entry.depth;
                                        rows.push(
                                            div()
                                                .id(("file", index))
                                                .h(px(26.))
                                                .pl(px(6.))
                                                .pr_2()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .text_size(px(13.))
                                                .text_color(name_color)
                                                .bg(if selected {
                                                    theme::chrome_bg()
                                                } else {
                                                    theme::sidebar_bg()
                                                })
                                                .border_l_2()
                                                .border_color(if selected {
                                                    theme::accent()
                                                } else {
                                                    gpui::rgba(0x00000000)
                                                })
                                                .hover(|style| {
                                                    style.bg(theme::hover()).cursor_pointer()
                                                })
                                                .on_click(cx.listener(
                                                    move |ide, _: &ClickEvent, window, cx| {
                                                        ide.activate_entry(index, window, cx);
                                                    },
                                                ))
                                                .when(depth > 0, |row| {
                                                    row.child(indent_guides(depth))
                                                })
                                                .child(
                                                    div()
                                                        .w(px(12.))
                                                        .text_color(theme::faint())
                                                        .child(mark),
                                                )
                                                .child(entry.name),
                                        );
                                    }
                                    rows
                                }),
                            )
                            .size_full(),
                        )
                    }),
            )
    }
}

fn indent_guides(depth: usize) -> impl IntoElement {
    div().h(px(26.)).flex().flex_row().children((0..depth).map(|_| {
        div()
            .w(px(14.))
            .h(px(26.))
            .flex()
            .justify_center()
            .child(div().w(px(1.)).h(px(26.)).bg(theme::border()))
    }))
}

fn menu_entries(menu: MenuId) -> Vec<MenuEntry> {
    match menu {
        MenuId::File => vec![
            MenuEntry::Item {
                id: "new-file",
                label: "New File",
                shortcut: ui::primary_shortcut("N"),
            },
            MenuEntry::Item {
                id: "open-file",
                label: "Open File…",
                shortcut: ui::primary_shortcut("O"),
            },
            MenuEntry::Item {
                id: "open-folder",
                label: "Open Folder…",
                shortcut: ui::primary_shortcut("Shift+O"),
            },
            MenuEntry::Separator,
            MenuEntry::Item {
                id: "save",
                label: "Save",
                shortcut: ui::primary_shortcut("S"),
            },
            MenuEntry::Item {
                id: "close",
                label: "Close Tab",
                shortcut: ui::primary_shortcut("W"),
            },
            MenuEntry::Separator,
            MenuEntry::Item {
                id: "quit",
                label: "Quit",
                shortcut: ui::primary_shortcut("Q"),
            },
        ],
        MenuId::Edit => vec![
            MenuEntry::Item {
                id: "undo",
                label: "Undo",
                shortcut: ui::primary_shortcut("Z"),
            },
            MenuEntry::Item {
                id: "redo",
                label: "Redo",
                shortcut: ui::primary_shortcut("Shift+Z"),
            },
            MenuEntry::Separator,
            MenuEntry::Item {
                id: "cut",
                label: "Cut",
                shortcut: ui::primary_shortcut("X"),
            },
            MenuEntry::Item {
                id: "copy",
                label: "Copy",
                shortcut: ui::primary_shortcut("C"),
            },
            MenuEntry::Item {
                id: "paste",
                label: "Paste",
                shortcut: ui::primary_shortcut("V"),
            },
            MenuEntry::Separator,
            MenuEntry::Item {
                id: "select-all",
                label: "Select All",
                shortcut: ui::primary_shortcut("A"),
            },
        ],
    }
}

fn read_file(path: &Path) -> Result<(String, bool), String> {
    let meta = fs::metadata(path).map_err(|error| error.to_string())?;
    if meta.is_dir() {
        return Err("That path is a folder".into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err("File is larger than 8 MiB".into());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let lossy = std::str::from_utf8(&bytes).is_err();
    Ok((String::from_utf8_lossy(&bytes).into_owned(), lossy))
}

pub fn focus_window(window: WindowHandle<Ide>, cx: &mut App) {
    window
        .update(cx, |ide, window, cx| {
            ide.focus_root(window, cx);
            cx.activate(true);
        })
        .ok();
}
