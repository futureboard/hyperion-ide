# HyperionIDE

HyperionIDE is a small desktop IDE shell: a folder tree, tabbed text editor, and status bar. It is built on [gpui-se](https://github.com/futureboard/gpui-se), the standalone edition of Zed's GPUI.

gpui-se is a GPU-accelerated UI toolkit (entities, layout, text shaping, input, menus, file dialogs). It does not ship a code editor, syntax highlighter, language server client, or terminal. This app uses the toolkit's text system and `EntityInputHandler` for a real multi-line editor, and does not pretend to provide LSP or a terminal.

## Dependency

- Repository: https://github.com/futureboard/gpui-se
- Package: `gpui` 0.2.2 (as published in that tree)
- Pinned commit: `5779e7325fc499906eb7aa19c9156f9dcb96a9cc`

The crate is not on crates.io under this fork, so `Cargo.toml` depends on that git revision. `gpui_platform` comes from the same commit and selects the Linux Wayland/X11 backend.

## Build and run

```sh
cargo build --release
cargo run --release --bin HyperionIDE
```

The window opens on the current working directory. `cargo test` covers the text buffer and the file-tree walker; those tests do not open a window.

A display is required. The process owns the GPUI event loop and does not have a headless GUI mode.

## What you can do

- Browse the working folder. Directories expand in place. Refresh re-reads the tree.
- Open a file from the tree, or use **File → Open File** / **Open Folder**.
- Create an untitled buffer with **File → New File**.
- Edit text: caret, selection, word movement, clipboard, indent-aware Enter, Tab as four spaces.
- Undo and redo. History is a local snapshot stack (gpui-se has no editor undo).
- Save and close tabs. A dirty tab shows an amber mark and asks before it closes.
- Line endings are stored as LF. CRLF and CR are converted when a file is opened.

Shortcuts use the platform command key (`Ctrl` on Linux, `⌘` on macOS):

| Action | Shortcut |
| --- | --- |
| Open File | Ctrl/⌘ O |
| Open Folder | Ctrl/⌘ Shift O |
| New File | Ctrl/⌘ N |
| Save | Ctrl/⌘ S |
| Close Tab | Ctrl/⌘ W |
| Quit | Ctrl/⌘ Q |
| Undo / Redo | Ctrl/⌘ Z, Ctrl/⌘ Shift Z |

Home and End move within the line. On Linux, Ctrl+Left/Right moves by word; on macOS that is Option+Left/Right, and Command+Left/Right moves to the line edges.

Files larger than 8 MiB are refused. Syntax highlighting, LSP, and a terminal are not included.

## Arch Linux packages

GPUI renders with Vulkan and talks to Wayland or X11. File dialogs go through the XDG desktop portal.

```sh
sudo pacman -S --needed \
  base-devel pkgconf fontconfig \
  wayland libxkbcommon libxkbcommon-x11 \
  libxcb xcb-util-wm \
  vulkan-icd-loader \
  xdg-desktop-portal xdg-desktop-portal-gtk
```

Install a Vulkan driver as well: `vulkan-radeon`, `vulkan-intel`, or `mesa` (including lavapipe) depending on the GPU. A monospace face such as `ttf-jetbrains-mono` or `ttf-dejavu` is used for the editor; the system UI font is the fallback.

This commit of gpui-se does not build on current crates.io without three small local patches, kept under `vendor/`:

- `vendor/xattr` is xattr 0.2.3 with one Linux fix. Current `libc` no longer exports `ENOATTR`, and gpui-se's HTTP client still pulls that crate through `async-tar`.
- `vendor/accesskit_unix` is accesskit_unix 0.22.0 (built against accesskit 0.24) with its package version set to 0.17.2, because gpui-se asks for `accesskit_unix` 0.17 while using accesskit 0.24.
- `vendor/gpui_linux` is the Linux backend from the pinned commit. File dialogs use `url::Url`, which is what ashpd 0.11 returns. The upstream file calls `ashpd::Uri`, a type that crate version does not export.
