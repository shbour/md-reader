# Markdown Reader

A desktop app for reading, editing and previewing Markdown files, written in
Rust: a GNOME app for Linux (GTK 4, libadwaita, WebKitGTK) and a Windows app
(Tauri, WebView2) on the same rendering core.

## Download

Packages are on the [Releases page](https://github.com/shbour/md-reader/releases).

| System | File | Install |
|---|---|---|
| Windows 10 / 11 | `markdown-reader_<version>_x64-setup.exe` | Run it; it installs for your user, no admin needed |
| Fedora 40+ | `markdown-reader-<version>-1.x86_64.rpm` | `sudo dnf install ./markdown-reader-*.rpm` |
| RHEL / AlmaLinux / Rocky 10 | the same `.rpm` | `sudo dnf install epel-release` first (EPEL has WebKitGTK 6.0) |
| Ubuntu 24.04+, Debian 13+ (Mint 22, Pop!_OS 24.04…) | `markdown-reader_<version>_amd64.deb` | `sudo apt install ./markdown-reader_*.deb` |

The Windows installer isn't code-signed yet, so SmartScreen may warn the first
time: choose **More info** → **Run anyway**. On Ubuntu the package also installs
a small AppArmor profile that WebKitGTK's sandbox needs there.

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

Requires GTK ≥ 4.14, libadwaita ≥ 1.5, WebKitGTK 6.0 ≥ 2.40 and
GtkSourceView 5, so it builds on Fedora 40+, Ubuntu 24.04+ and Debian 13+
(Ubuntu 22.04 and Debian 12 ship libraries that are too old).

Fedora:

```sh
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel webkitgtk6.0-devel gtksourceview5-devel
```

Debian 13 / Ubuntu 24.04 (and derivatives such as Linux Mint 22, Pop!_OS 24.04):

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libgtksourceview-5-dev
```

Then, with a Rust toolchain (1.85+, e.g. from [rustup](https://rustup.rs)):

```sh
cargo build --release
```

## Installing from source

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

## Project layout

| Path | What |
|---|---|
| `core/` | Markdown rendering, sanitising, link handling, HTML export, and the preview page's CSS and script, shared by both front ends |
| `gtk/` | The Linux app (GTK 4, libadwaita, WebKitGTK) |
| `tauri/` | The Windows app (Tauri, WebView2, CodeMirror), see [tauri/README.md](tauri/README.md) |
| `vendor/` | Bundled KaTeX and Mermaid |
| `data/` | Icon, desktop entry and AppStream metadata |
| `packaging/` | `build-rpm.sh` (Fedora/RHEL), `build-deb.sh` (Debian/Ubuntu, run it on the oldest release to support), the RPM spec and the AppArmor profile |

## Bundled libraries

`vendor/` holds [KaTeX](https://katex.org), [Mermaid](https://mermaid.js.org) and,
for the Windows app, [CodeMirror](https://codemirror.net), all MIT-licensed (licences
included). `vendor/update.sh` re-downloads KaTeX and Mermaid and rebuilds KaTeX's
self-contained stylesheet; `vendor/update-codemirror.sh` rebuilds the CodeMirror bundle.

## Licence

MIT, see [LICENSE](LICENSE).
