//! Deciding what a clicked link in the preview should do.

use std::path::PathBuf;

use gtk::gio::prelude::FileExt;

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
            let Some(path) = gtk::gio::File::for_uri(without_frag).path() else {
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
    gtk::glib::Uri::unescape_string(s, None::<&str>)
        .map(|g| g.to_string())
        .unwrap_or_else(|| s.to_string())
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
