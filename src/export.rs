//! Building a standalone HTML file from the rendered preview.

use std::path::Path;

use gtk::glib;

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

fn rewrite_img_tag(tag: &str, doc_dir: &Path) -> String {
    let Some(i) = tag.find(" src=\"") else { return tag.to_string() };
    let v_start = i + " src=\"".len();
    let Some(v_len) = tag[v_start..].find('"') else { return tag.to_string() };
    let raw = &tag[v_start..v_start + v_len];
    match data_uri_for(&raw.replace("&amp;", "&"), doc_dir) {
        Some(uri) => format!("{}{}{}", &tag[..v_start], uri, &tag[v_start + v_len..]),
        None => tag.to_string(),
    }
}

fn data_uri_for(src: &str, doc_dir: &Path) -> Option<String> {
    let path = if let Some(rest) = src.strip_prefix("file://") {
        glib::Uri::unescape_string(rest, None::<&str>)?.to_string().into()
    } else if src.contains(':') || src.starts_with("//") || src.is_empty() {
        return None; // http(s), data:, etc. stay as they are
    } else {
        let no_query = src.split(['?', '#']).next().unwrap_or(src);
        let rel = glib::Uri::unescape_string(no_query, None::<&str>)?.to_string();
        doc_dir.join(rel)
    };
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let mime = mime_for(&path)?;
    let bytes = std::fs::read(&path).ok()?;
    Some(format!("data:{mime};base64,{}", glib::base64_encode(&bytes)))
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
        let expected = format!("<img src=\"data:image/png;base64,{}\" alt=\"x\">", glib::base64_encode(b"\x89PNG fake"));
        assert!(out.contains(&expected), "{out}");
        assert!(out.contains(r#"src="https://example.com/r.png""#), "{out}");
        assert!(out.contains(r#"<img src="missing.png">"#), "{out}");
        let _ = std::fs::remove_dir_all(dir);
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
