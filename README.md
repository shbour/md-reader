# Markdown Reader

A GNOME desktop app for reading, editing and previewing Markdown files, written
in Rust with GTK 4, libadwaita and WebKitGTK.

## Features

- **Preview** with GitHub-flavoured Markdown: tables, task lists, footnotes,
  alerts (`> [!NOTE]`), emoji shortcodes, syntax-highlighted code, and front
  matter hidden.
- **HTML inside Markdown**, sanitised the way GitHub does it: `<details>`,
  `<kbd>`, `<sub>`, `<img width>`, `<p align="center">` work; scripts, styles
  and event handlers are removed.
- **Math** (`$…$`, `$$…$$`, `` ```math ``) rendered with KaTeX.
- **Diagrams** (`` ```mermaid ``) rendered with Mermaid.
- **Edit mode** (Ctrl+E): side-by-side editor and live preview, scrolled in
  sync line by line in both directions. Long lines wrap.
- **Search** (Ctrl+F): in edit mode it steps through matches in the editor and
  highlights them in the preview; Escape leaves the match selected.
- **Contents sidebar** (F9) built from the headings.
- **Print** (Ctrl+P), **Export as PDF**, **Export as HTML** (a single
  self-contained file with images embedded).
- Follows links between Markdown files (with Back, Alt+Left), reloads when the
  file changes on disk, and remembers the window layout between sessions.
- Works offline: KaTeX and Mermaid are bundled into the binary.

## Building

Fedora:

```sh
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel gtksourceview5-devel
```

Then, with a Rust toolchain (1.85+):

```sh
cargo build --release
```

## Installing

```sh
./install.sh               # build, install to ~/.local, set as the default .md app
./install.sh --no-default  # same, but leave the default app alone
./install.sh --uninstall
```

Run it with `mdreader [FILE…]` or `mdreader --new`.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| Ctrl+O / Ctrl+N | Open / new document |
| Ctrl+S / Ctrl+Shift+S | Save / save as |
| Ctrl+E | Toggle edit mode |
| Ctrl+F, Ctrl+G, Ctrl+Shift+G | Find, next, previous |
| F9 | Contents sidebar |
| Alt+Left | Back |
| Ctrl+P | Print |
| Ctrl+Plus / Ctrl+Minus / Ctrl+0 | Zoom |
| Ctrl+W / Ctrl+Q | Close window / quit |

## Bundled libraries

`vendor/` holds [KaTeX](https://katex.org) and [Mermaid](https://mermaid.js.org),
both MIT-licensed (licences included). `vendor/update.sh` re-downloads them and
rebuilds KaTeX's self-contained stylesheet.

## Licence

MIT, see [LICENSE](LICENSE).
