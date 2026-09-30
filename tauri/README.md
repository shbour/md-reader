# Markdown Reader for Windows (Tauri)

The same app as the GNOME version in `../gtk`, built with
[Tauri](https://tauri.app) so it runs on Windows (and macOS/Linux). Rendering,
sanitising, links, HTML export and the preview page's script come from the
shared `../core` crate; this folder is the window around them.

- `src/main.rs`: native side: file reading/writing, the file watcher,
  opening links in the browser or default app, HTML export.
- `ui/`: the window (HTML/CSS/JS). The editor is CodeMirror 6, bundled
  offline in `../vendor/codemirror`.
- The preview runs in a sandboxed `<iframe>` with no same-origin access, so a
  document can never reach the app's file access. It talks to the window
  with `postMessage` (see `core/src/runtime.js`).

## Build on Windows

1. Install [Rust](https://rustup.rs) (MSVC toolchain) and the
   [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
   ("Desktop development with C++"). WebView2 is part of Windows 10/11.
2. `cargo install tauri-cli --locked`
3. In this folder: `cargo tauri build`

The installer is written to
`target/release/bundle/nsis/Markdown Reader_<version>_x64-setup.exe`. It installs
for the current user (no admin needed), adds a Start menu entry, and registers
the app for `.md` files.

## Cross-build the Windows installer on Linux

Fedora:

```sh
sudo dnf install mingw64-nsis lld llvm clang
rustup target add x86_64-pc-windows-msvc
cargo install --locked tauri-cli cargo-xwin
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc
```

cargo-xwin downloads the Windows SDK and C runtime libraries (accepting
Microsoft's licence for them) on first use. The installer lands in
`../target/x86_64-pc-windows-msvc/release/bundle/nsis/`.

## Run on Linux (development)

Needs `webkit2gtk4.1-devel` (Fedora) or `libwebkit2gtk-4.1-dev` (Debian/Ubuntu).

```sh
cargo run -p mdreader-tauri -- path/to/file.md
```

On NVIDIA with Wayland, WebKitGTK may exit with "Error 71 (Protocol error)";
run with `WEBKIT_DISABLE_DMABUF_RENDERER=1`.

Debug builds on Linux accept a test driver: `MDREADER_DRIVE=/path/to/fifo`, then
write `js <code>` or `snap <file.png>` lines to the FIFO.

## Differences from the GTK version

- One window. Opening another document, following a link to one, or
  double-clicking a `.md` file replaces the current one (asking about unsaved
  changes first) and Back returns to it.
- PDF export goes through **Print** (Ctrl+P): choose "Save as PDF" (Microsoft Print
  to PDF) in the print dialog.
- Window size and position are restored by Tauri; zoom, sidebar and split are
  kept in the app's WebView2 profile.
