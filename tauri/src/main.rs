//! Markdown Reader, Tauri front end (Windows first; also runs on macOS/Linux).
//!
//! Rendering, sanitising, link classification and HTML export come from
//! `mdreader-core`, shared with the GTK app. This file is the thin native
//! layer the web UI (`ui/`) calls: file I/O, watching the open file, and
//! opening links outside the app.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mdreader_core::links::{self, LinkAction};
use mdreader_core::{assets, export, render};
use notify::{EventKind, RecursiveMode, Watcher};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[derive(Serialize)]
struct Document {
    path: String,
    name: String,
    dir: String,
    text: String,
    /// The file wasn't valid UTF-8 and some characters were replaced.
    lossy: bool,
}

#[derive(Serialize)]
struct TocEntry {
    level: u8,
    text: String,
    anchor: String,
    line: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Rendered {
    html: String,
    toc: Vec<TocEntry>,
    has_math: bool,
    has_mermaid: bool,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum LinkReply {
    /// Dealt with here (opened in the browser / default app) or ignored.
    Handled,
    /// Open this Markdown file in the app.
    Markdown { path: String, fragment: Option<String> },
    /// The link points at a local file that doesn't exist.
    Missing { path: String },
}

#[derive(Default)]
struct WatchState(Mutex<Option<notify::RecommendedWatcher>>);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[tauri::command]
fn read_document(app: AppHandle, path: String) -> Result<Document, String> {
    let p = PathBuf::from(&path);
    let bytes = std::fs::read(&p).map_err(err)?;
    let (text, lossy) = match String::from_utf8(bytes) {
        Ok(t) => (t, false),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
    };
    let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
    // Let the preview load images that sit next to the document.
    app.asset_protocol_scope().allow_directory(&dir, true).map_err(err)?;
    Ok(Document {
        name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        dir: path_string(&dir),
        path,
        text,
        lossy,
    })
}

/// Write via a temporary file and rename, so a crash never leaves half a file.
#[tauri::command]
fn write_document(app: AppHandle, path: String, text: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    let name = p.file_name().ok_or("not a file path")?.to_string_lossy().into_owned();
    let tmp = p.with_file_name(format!(".{name}.mdreader-tmp"));
    std::fs::write(&tmp, text).map_err(err)?;
    if let Ok(meta) = std::fs::metadata(&p) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, &p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(e)
    })?;
    if let Some(dir) = p.parent() {
        app.asset_protocol_scope().allow_directory(dir, true).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
fn render_markdown(text: String) -> Rendered {
    let r = render::render(&text);
    Rendered {
        html: r.html,
        toc: r
            .toc
            .into_iter()
            .map(|h| TocEntry { level: h.level, text: h.text, anchor: h.anchor, line: h.line })
            .collect(),
        has_math: r.has_math,
        has_mermaid: r.has_mermaid,
    }
}

/// Characters to escape in an asset URL path; '/' and ':' stay so relative
/// references (`img/a.png`, `../b.png`) resolve like on disk.
const PATH_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'[')
    .add(b']')
    .add(b'^')
    .add(b'|');

/// The asset-protocol URL of a folder. Tauri's handler percent-decodes the
/// URL path minus its first '/', which gives `C:/Users/…` on Windows and
/// `/home/…` on Unix (hence the `//` there).
fn asset_dir_url(dir: &Path) -> String {
    let prefix = if cfg!(any(windows, target_os = "android")) { "http://asset.localhost/" } else { "asset://localhost/" };
    let mut s = dir.to_string_lossy().replace('\\', "/");
    if !s.ends_with('/') {
        s.push('/');
    }
    format!("{prefix}{}", utf8_percent_encode(&s, PATH_SET))
}

#[tauri::command]
fn preview_page(dir: Option<String>) -> String {
    let base = dir.filter(|d| !d.is_empty()).map(|d| asset_dir_url(Path::new(&d)));
    render::embedded_page(base.as_deref())
}

#[tauri::command]
fn open_link(app: AppHandle, href: String, doc_path: Option<String>) -> Result<LinkReply, String> {
    let dir = doc_path.as_deref().and_then(|p| Path::new(p).parent());
    Ok(match links::classify_href(&href, dir) {
        LinkAction::OpenMarkdown { path, fragment } if path.exists() => {
            LinkReply::Markdown { path: path_string(&path), fragment }
        }
        LinkAction::OpenMarkdown { path, .. } | LinkAction::OpenFile(path) if !path.exists() => {
            LinkReply::Missing { path: path_string(&path) }
        }
        LinkAction::OpenFile(path) => {
            app.opener().open_path(path_string(&path), None::<&str>).map_err(err)?;
            LinkReply::Handled
        }
        LinkAction::External(uri) => {
            app.opener().open_url(uri, None::<&str>).map_err(err)?;
            LinkReply::Handled
        }
        LinkAction::OpenMarkdown { .. } | LinkAction::Allow | LinkAction::Ignore => LinkReply::Handled,
    })
}

#[tauri::command]
fn export_html(dest: String, title: String, body: String, has_math: bool, doc_dir: Option<String>) -> Result<(), String> {
    let page = export::standalone_page(
        &title,
        &body,
        has_math.then_some(assets::KATEX_CSS),
        doc_dir.as_deref().filter(|d| !d.is_empty()).map(Path::new),
    );
    std::fs::write(dest, page).map_err(err)
}

/// Watch `path` (or stop watching, for `None`). Emits "file-changed" with the
/// path. The folder is watched, not the file, so editors that save by
/// writing a new file and renaming it over the old one are noticed too.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico", "avif"];

/// An image for the preview, by its asset URL. WebKitGTK only shows custom
/// scheme URLs to pages that may fetch them, and the sandboxed preview (origin
/// null) may fetch nothing, so on Linux it asks for the bytes instead. Serves
/// what the asset protocol would: images inside its scope.
#[tauri::command]
fn read_image(app: AppHandle, url: String) -> Result<tauri::ipc::Response, String> {
    let rest = url.strip_prefix("asset://localhost/").ok_or("not an asset URL")?;
    let path = PathBuf::from(percent_encoding::percent_decode_str(rest).decode_utf8_lossy().into_owned());
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("not allowed".into());
    }
    let path = path.canonicalize().map_err(err)?;
    let is_image = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
    if !is_image || !app.asset_protocol_scope().is_allowed(&path) {
        return Err("not allowed".into());
    }
    std::fs::read(&path).map(tauri::ipc::Response::new).map_err(err)
}

#[tauri::command]
fn watch_files(app: AppHandle, state: State<WatchState>, paths: Vec<String>) -> Result<(), String> {
    let mut slot = state.0.lock().map_err(err)?;
    *slot = None;
    // Watch each folder (not the file): saving via rename replaces the file.
    let targets: Vec<(PathBuf, String)> = paths.into_iter().map(|p| (PathBuf::from(&p), p)).collect();
    let mut dirs: Vec<PathBuf> = targets.iter().filter_map(|(t, _)| t.parent().map(Path::to_path_buf)).collect();
    dirs.sort();
    dirs.dedup();
    if dirs.is_empty() {
        return Ok(());
    }
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        let relevant = matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_))
            && !matches!(ev.kind, EventKind::Modify(notify::event::ModifyKind::Metadata(_)));
        if !relevant {
            return;
        }
        for (target, path) in &targets {
            if ev.paths.iter().any(|p| p.file_name() == target.file_name() && p.parent() == target.parent()) {
                let _ = app.emit("file-changed", path);
            }
        }
    })
    .map_err(err)?;
    for dir in &dirs {
        watcher.watch(dir, RecursiveMode::NonRecursive).map_err(err)?;
    }
    *slot = Some(watcher);
    Ok(())
}

/// The first file named on the command line (e.g. from a file association).
fn file_arg(args: &[String], cwd: &Path) -> Option<String> {
    args.iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(|a| cwd.join(a))
        .find(|p| p.is_file())
        .map(|p| path_string(&p.canonicalize().unwrap_or(p)))
        .map(strip_verbatim)
}

/// `canonicalize` on Windows returns `\\?\C:\…`; show and use `C:\…`.
fn strip_verbatim(p: String) -> String {
    p.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(p)
}

#[tauri::command]
fn startup_file() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    file_arg(&args, &std::env::current_dir().unwrap_or_default())
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Lets tests driving a debug build read values back out of the page.
#[tauri::command]
fn debug_log(msg: String) {
    if cfg!(debug_assertions) {
        eprintln!("mdreader: {msg}");
    }
}

/// Test driver for Linux debug builds: with MDREADER_DRIVE=/path/to/fifo,
/// each line written to the FIFO is `js <code>` (run in the window) or
/// `snap <file.png>` (save a picture of the webview). Not in release builds.
#[cfg(all(debug_assertions, target_os = "linux"))]
mod devdrive {
    use std::io::BufRead;

    use tauri::{AppHandle, Manager};
    use webkit2gtk::WebViewExt;

    pub fn start(app: &AppHandle) {
        let Ok(fifo) = std::env::var("MDREADER_DRIVE") else { return };
        // Console messages, including the preview frame's, go to stdout.
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.with_webview(|pv| {
                use webkit2gtk::SettingsExt;
                if let Some(settings) = pv.inner().settings() {
                    settings.set_enable_write_console_messages_to_stdout(true);
                }
            });
        }
        let app = app.clone();
        std::thread::spawn(move || {
            loop {
                let Ok(f) = std::fs::File::open(&fifo) else { return };
                for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
                    let Some(w) = app.get_webview_window("main") else { continue };
                    if let Some(js) = line.strip_prefix("js ") {
                        let _ = w.eval(js);
                    } else if let Some(path) = line.strip_prefix("snap ") {
                        let path = path.to_string();
                        let _ = w.with_webview(move |pv| {
                            pv.inner().snapshot(
                                webkit2gtk::SnapshotRegion::Visible,
                                webkit2gtk::SnapshotOptions::NONE,
                                None::<&webkit2gtk::gio::Cancellable>,
                                move |r| {
                                    let saved = r.map_err(|e| e.to_string()).and_then(|s| {
                                        let img = cairo::ImageSurface::try_from(s).map_err(|_| "not an image".to_string())?;
                                        let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                                        img.write_to_png(&mut f).map_err(|e| e.to_string())
                                    });
                                    match saved {
                                        Ok(()) => eprintln!("mdreader: snapshot saved to {path}"),
                                        Err(e) => eprintln!("mdreader: snapshot failed: {e}"),
                                    }
                                },
                            );
                        });
                    }
                }
            }
        });
    }
}

/// A window rectangle as (x, y, width, height) in physical pixels.
type Rect = (i32, i32, u32, u32);

/// Where to put a window so all of it is inside `area` (the monitor minus the
/// taskbar), or None when it already is. The default size is taller than a
/// 1366x768 laptop screen, and a saved size may come from a bigger monitor.
fn fit_rect(win: Rect, area: Rect) -> Option<Rect> {
    let (ax, ay, aw, ah) = area;
    let (w, h) = (win.2.min(aw), win.3.min(ah));
    let clamp = |p: i32, len: u32, start: i32, avail: u32| p.clamp(start, start + (avail - len) as i32);
    let fitted = (clamp(win.0, w, ax, aw), clamp(win.1, h, ay, ah), w, h);
    (fitted != win).then_some(fitted)
}

fn fit_to_screen(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else { return Ok(()) };
    let area = monitor.work_area();
    let (pos, outer, inner) = (window.outer_position()?, window.outer_size()?, window.inner_size()?);
    let win = (pos.x, pos.y, outer.width, outer.height);
    let area = (area.position.x, area.position.y, area.size.width, area.size.height);
    if let Some((x, y, w, h)) = fit_rect(win, area) {
        // set_size takes the inner size: take off the title bar and borders.
        let frame = (outer.width - inner.width, outer.height - inner.height);
        window.set_size(tauri::PhysicalSize::new(w - frame.0, h - frame.1))?;
        window.set_position(tauri::PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        // Must be first: a second launch (double-clicking another .md file)
        // hands its file to the running window instead of starting again.
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            if let Some(path) = file_arg(&args, Path::new(&cwd)) {
                let _ = app.emit("open-file", path);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(WatchState::default())
        .setup(|_app| {
            // After the window-state plugin has restored the saved geometry.
            if let Some(window) = _app.get_webview_window("main") {
                let _ = fit_to_screen(&window);
            }
            #[cfg(all(debug_assertions, target_os = "linux"))]
            devdrive::start(_app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            read_document,
            write_document,
            render_markdown,
            preview_page,
            open_link,
            export_html,
            watch_files,
            read_image,
            startup_file,
            app_version,
            debug_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Markdown Reader");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_are_kept_on_screen() {
        let laptop = (0, 0, 1366, 720); // 768 minus a 48 px taskbar
        assert_eq!(fit_rect((100, 50, 1000, 600), laptop), None);
        // Too tall: shrunk to the work area and moved up.
        assert_eq!(fit_rect((133, 0, 1100, 811), laptop), Some((133, 0, 1100, 720)));
        // Fits, but hangs off the bottom right: moved in.
        assert_eq!(fit_rect((800, 400, 1000, 600), laptop), Some((366, 120, 1000, 600)));
        // Bigger than the screen both ways, on a second monitor to the left.
        assert_eq!(fit_rect((-3000, 10, 2500, 1400), (-1920, 0, 1920, 1040)), Some((-1920, 0, 1920, 1040)));
    }

    #[test]
    fn asset_urls_keep_path_structure() {
        let u = asset_dir_url(Path::new("/home/u/My Docs"));
        if cfg!(windows) {
            assert!(u.starts_with("http://asset.localhost/"));
        } else {
            assert_eq!(u, "asset://localhost//home/u/My%20Docs/");
        }
    }

    #[test]
    fn file_args_skip_flags_and_missing_files() {
        let dir = std::env::temp_dir().join(format!("mdreader-tauri-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "x").unwrap();
        let args = vec!["app".to_string(), "--flag".into(), "missing.md".into(), "a.md".into()];
        let found = file_arg(&args, &dir).expect("a.md is found");
        assert!(found.ends_with("a.md"));
        assert_eq!(file_arg(&args[..3], &dir), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
