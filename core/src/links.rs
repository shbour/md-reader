//! Deciding what a clicked link in the preview should do.

use std::path::{Path, PathBuf};

use url::Url;

#[derive(Debug, PartialEq)]
pub enum LinkAction {
    /// Let WebKit handle it (a `#fragment` within the current page).
    Allow,
    /// Open this Markdown file in the reader, then jump to the fragment if any.
    OpenMarkdown { path: PathBuf, fragment: Option<String> },
    /// Hand a local non-Markdown file to its default application.
    OpenFile(PathBuf),
    /// Hand to the default handler for the scheme (browser, mail client...).
    External(String),
    /// Unknown scheme: do nothing.
    Ignore,
}

pub fn is_markdown(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdx"))
}

/// `uri` is the link's resolved target; `page_uri` is the base URI the current
/// page was loaded with (the document's directory, or `about:blank`).
pub fn classify(uri: &str, page_uri: &str) -> LinkAction {
    let (without_frag, fragment) = match uri.split_once('#') {
        Some((a, f)) => (a, Some(f)),
        None => (uri, None),
    };
    let page_no_frag = page_uri.split('#').next().unwrap_or(page_uri);
    if fragment.is_some() && without_frag == page_no_frag {
        return LinkAction::Allow;
    }

    let scheme = uri.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("file") => {
            let Some(path) = Url::parse(without_frag).ok().and_then(|u| u.to_file_path().ok()) else {
                return LinkAction::Ignore;
            };
            if is_markdown(&path) {
                let fragment = fragment.filter(|f| !f.is_empty()).map(percent_decode);
                LinkAction::OpenMarkdown { path, fragment }
            } else {
                LinkAction::OpenFile(path)
            }
        }
        Some("http" | "https" | "mailto" | "ftp" | "sftp" | "irc" | "ircs" | "xmpp" | "tel") => {
            LinkAction::External(uri.to_string())
        }
        _ => LinkAction::Ignore,
    }
}

fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// `file://` URL of a directory, with the trailing slash that makes relative
/// links resolve inside it.
pub fn dir_url(dir: &Path) -> Option<Url> {
    Url::from_directory_path(dir).ok()
}

/// Classify a link as written in the document (`href` may be relative),
/// for a document living in `doc_dir` (`None` for an unsaved document).
pub fn classify_href(href: &str, doc_dir: Option<&Path>) -> LinkAction {
    let base = doc_dir.and_then(dir_url);
    if let Some(frag) = href.strip_prefix('#') {
        return match &base {
            Some(_) => LinkAction::Allow,
            None if frag.is_empty() => LinkAction::Ignore,
            None => LinkAction::Allow,
        };
    }
    let resolved = match &base {
        Some(b) => b.join(href).map(|u| u.to_string()),
        None => Url::parse(href).map(|u| u.to_string()),
    };
    match resolved {
        Ok(uri) => classify(&uri, base.as_ref().map(Url::as_str).unwrap_or("about:blank")),
        Err(_) => LinkAction::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "file:///home/u/docs/";

    #[test]
    fn same_page_fragment_is_allowed() {
        assert_eq!(classify("file:///home/u/docs/#setup", PAGE), LinkAction::Allow);
        assert_eq!(classify("about:blank#x", "about:blank"), LinkAction::Allow);
    }

    #[test]
    fn markdown_links_open_in_reader() {
        assert_eq!(
            classify("file:///home/u/docs/other.md", PAGE),
            LinkAction::OpenMarkdown { path: "/home/u/docs/other.md".into(), fragment: None }
        );
        assert_eq!(
            classify("file:///home/u/docs/My%20Notes.MD#caf%C3%A9", PAGE),
            LinkAction::OpenMarkdown {
                path: "/home/u/docs/My Notes.MD".into(),
                fragment: Some("café".into())
            }
        );
    }

    #[test]
    fn other_local_files_go_to_default_app() {
        assert_eq!(
            classify("file:///home/u/docs/diagram.png", PAGE),
            LinkAction::OpenFile("/home/u/docs/diagram.png".into())
        );
    }

    #[test]
    fn relative_hrefs_resolve_against_the_document() {
        let dir = Path::new("/home/u/docs");
        assert_eq!(
            classify_href("sub/other.md#part-2", Some(dir)),
            LinkAction::OpenMarkdown { path: "/home/u/docs/sub/other.md".into(), fragment: Some("part-2".into()) }
        );
        assert_eq!(classify_href("../pic%20a.png", Some(dir)), LinkAction::OpenFile("/home/u/pic a.png".into()));
        assert_eq!(classify_href("#setup", Some(dir)), LinkAction::Allow);
        assert_eq!(
            classify_href("https://example.com", Some(dir)),
            LinkAction::External("https://example.com/".into())
        );
        assert_eq!(classify_href("other.md", None), LinkAction::Ignore, "unsaved: nowhere to resolve against");
    }

    #[test]
    fn web_links_are_external_and_odd_schemes_ignored() {
        assert_eq!(
            classify("https://example.com/a#b", PAGE),
            LinkAction::External("https://example.com/a#b".into())
        );
        assert_eq!(
            classify("mailto:a@b.c", PAGE),
            LinkAction::External("mailto:a@b.c".into())
        );
        assert_eq!(classify("javascript:alert(1)", PAGE), LinkAction::Ignore);
        assert_eq!(classify("data:text/html,hi", PAGE), LinkAction::Ignore);
    }
}
