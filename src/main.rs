mod app;
mod buffer;
mod command;
mod editor;
mod highlight;
mod theme;
mod ui;
mod workspace;

use app::{CloseMenu, CloseTab, NewFile, OpenFile, OpenFolder, Quit, Save};
use gpui::{
    App, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};
use gpui_platform::application;

fn main() {
    application().run(|cx: &mut App| {
        editor::bind_editor_keys(cx);
        bind_app_keys(cx);
        app::install_menus(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("HyperionIDE".into()),
                        ..TitlebarOptions::default()
                    }),
                    app_id: Some("hyperion-ide".into()),
                    window_min_size: Some(size(px(760.), px(480.))),
                    ..WindowOptions::default()
                },
                |_window, cx| cx.new(app::Ide::new),
            )
            .expect("failed to open the HyperionIDE window");
        app::focus_window(window, cx);
    });
}

fn bind_app_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-o", OpenFile, None),
        KeyBinding::new("secondary-shift-o", OpenFolder, None),
        KeyBinding::new("secondary-n", NewFile, None),
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("escape", CloseMenu, None),
    ]);
}
