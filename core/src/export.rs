//! Building a standalone HTML file from the rendered preview.

use std::path::{Component, Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use percent_encoding::percent_decode_str;
use url::Url;

use crate::render;

/// Largest local image embedded into an exported page.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// A self-contained page: styles inline, always light (diagrams are exported
/// light), local images embedded as data: URIs.
pub fn standalone_page(title: &str, body: &str, katex_css: Option<&str>, doc_dir: Option<&Path>) -> String {
    let body = match doc_dir {
        Some(dir) => inline_images(body, dir),
        None => body.to_string(),
    };
    let katex = katex_css.map(|c| format!("<style>{c}</style>")).unwrap_or_default();
    format!(
        "<!DOCTYPE html>\n<html class=\"force-light\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"generator\" content=\"Markdown Reader\">\n<title>{title}</title>\n\
         <style>{css}</style>\n{katex}\n</head>\n<body>\n<article class=\"markdown-body\">\n{body}\n</article>\n</body>\n</html>\n",
        title = render::escape_html(title),
        css = render::stylesheet(),
    )
}

/// Replace `src` of `<img>` tags that point at local files with data: URIs.
pub fn inline_images(html: &str, doc_dir: &Path) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(tag_start) = rest.find("<img") {
        let Some(tag_len) = rest[tag_start..].find('>') else { break };
        let tag = &rest[tag_start..tag_start + tag_len + 1];
        out.push_str(&rest[..tag_start]);
        out.push_str(&rewrite_img_tag(tag, doc_dir));
        rest = &rest[tag_start + tag_len + 1..];
    }
    out.push_str(rest);
    out
}

/// Where the `src="…"` value sits in an `<img …>` tag.
fn src_range(tag: &str) -> Option<std::ops::Range<usize>> {
    let start = tag.find(" src=\"")? + " src=\"".len();
    let len = tag[start..].find('"')?;
    Some(start..start + len)
}

/// The `src` of every `<img>` in `html` (sanitized HTML, so `&amp;` is the
/// only entity a URL carries).
pub fn image_sources(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<img") {
        let Some(len) = rest[start..].find('>') else { break };
        let tag = &rest[start..start + len + 1];
        if let Some(r) = src_range(tag) {
            out.push(tag[r].replace("&amp;", "&"));
        }
        rest = &rest[start + len + 1..];
    }
    out
}

/// The local file an image `src` refers to, for a document in `doc_dir`:
/// relative paths (`..` resolved) and file: URLs; None for http, data: etc.
pub fn local_image_path(src: &str, doc_dir: &Path) -> Option<PathBuf> {
    if src.starts_with("file:") {
        return Url::parse(src).ok()?.to_file_path().ok();
    }
    if src.contains(':') || src.starts_with("//") || src.is_empty() {
        return None;
    }
    let no_query = src.split(['?', '#']).next().unwrap_or(src);
    let rel = percent_decode_str(no_query).decode_utf8().ok()?.into_owned();
    let mut path = PathBuf::new();
    for c in doc_dir.join(rel).components() {
        match c {
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            c => path.push(c),
        }
    }
    Some(path)
}

/// Whether `path` names an image type the preview and export can show.
pub fn is_image(path: &Path) -> bool {
    mime_for(path).is_some()
}

fn rewrite_img_tag(tag: &str, doc_dir: &Path) -> String {
    let Some(r) = src_range(tag) else { return tag.to_string() };
    match data_uri_for(&tag[r.clone()].replace("&amp;", "&"), doc_dir) {
        Some(uri) => format!("{}{}{}", &tag[..r.start], uri, &tag[r.end..]),
        None => tag.to_string(),
    }
}

fn data_uri_for(src: &str, doc_dir: &Path) -> Option<String> {
    let path = local_image_path(src, doc_dir)?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let mime = mime_for(&path)?;
    let bytes = std::fs::read(&path).ok()?;
    Some(format!("data:{mime};base64,{}", BASE64.encode(&bytes)))
}

fn mime_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

/// A title for the exported page: the first heading, else the file stem.
pub fn title(toc: &[render::Heading], path: Option<&Path>) -> String {
    toc.first()
        .map(|h| h.text.clone())
        .or_else(|| path.and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Untitled".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_images_are_embedded_remote_ones_kept() {
        let dir = std::env::temp_dir().join(format!("mdreader-export-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("img dir")).unwrap();
        std::fs::write(dir.join("img dir/a b.png"), b"\x89PNG fake").unwrap();
        let html = r#"<p><img src="img%20dir/a%20b.png" alt="x"> <img alt="r" src="https://example.com/r.png"> <img src="missing.png"></p>"#;
        let out = inline_images(html, &dir);
        let expected = format!("<img src=\"data:image/png;base64,{}\" alt=\"x\">", BASE64.encode(b"\x89PNG fake"));
        assert!(out.contains(&expected), "{out}");
        assert!(out.contains(r#"src="https://example.com/r.png""#), "{out}");
        assert!(out.contains(r#"<img src="missing.png">"#), "{out}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn image_paths_resolve_like_the_page() {
        let dir = Path::new("/home/u/proj/docs");
        let html = r#"<img src="../logo.png" alt="a"><p><img alt="b" src="./img/x%20y.png?v=2"></p><img src="https://e.com/r.png"><img src="data:image/png;base64,AA"><img src="a.png?x=1&amp;y=2">"#;
        let srcs = image_sources(html);
        assert_eq!(srcs, ["../logo.png", "./img/x%20y.png?v=2", "https://e.com/r.png", "data:image/png;base64,AA", "a.png?x=1&y=2"]);
        let paths: Vec<_> = srcs.iter().map(|s| local_image_path(s, dir)).collect();
        assert_eq!(
            paths,
            [
                Some(PathBuf::from("/home/u/proj/logo.png")),
                Some(PathBuf::from("/home/u/proj/docs/img/x y.png")),
                None,
                None,
                Some(PathBuf::from("/home/u/proj/docs/a.png")),
            ]
        );
        assert!(is_image(Path::new("a.PNG")) && !is_image(Path::new("notes.md")));
    }

    #[test]
    fn page_is_standalone_and_light() {
        let p = standalone_page("A <b>", "<p>hi</p>", Some(".katex{}"), None);
        assert!(p.starts_with("<!DOCTYPE html>"));
        assert!(p.contains("<html class=\"force-light\">"));
        assert!(p.contains("<title>A &lt;b&gt;</title>"));
        assert!(p.contains(".katex{}"));
        assert!(p.contains("--link:"), "page CSS included");
        assert!(!p.contains("<script"));
    }
}
