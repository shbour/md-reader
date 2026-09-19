//! Markdown -> HTML rendering, table-of-contents extraction and the page shell
//! the WebView displays.

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use std::borrow::Cow;

use comrak::adapters::{CodefenceRendererAdapter, SyntaxHighlighterAdapter};
use comrak::nodes::{NodeValue, Sourcepos};
use comrak::options::Plugins;
use comrak::plugins::syntect::{SyntectAdapter, SyntectAdapterBuilder};
use comrak::{Anchorizer, Arena, Options, format_html_with_plugins, parse_document};
use syntect::highlighting::ThemeSet;
use syntect::html::{ClassStyle, css_for_theme_with_class_style};

#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    /// The `id` attribute comrak gives the rendered heading.
    pub anchor: String,
    /// 1-based line in the Markdown source.
    pub line: usize,
}

pub struct Rendered {
    pub html: String,
    pub toc: Vec<Heading>,
    /// The page needs KaTeX / Mermaid loaded to finish rendering.
    pub has_math: bool,
    pub has_mermaid: bool,
}

fn options() -> Options<'static> {
    let mut o = Options::default();
    let e = &mut o.extension;
    e.strikethrough = true;
    e.table = true;
    e.autolink = true;
    e.tasklist = true;
    e.footnotes = true;
    e.description_lists = true;
    e.alerts = true;
    e.shortcodes = true;
    e.superscript = true;
    e.math_dollars = true;
    e.math_code = true;
    e.header_id_prefix = Some(String::new());
    // Front matter is metadata, not content; hide it like GitHub does.
    e.front_matter_delimiter = Some("---".into());
    o.render.github_pre_lang = true;
    o.render.tasklist_classes = true;
    // Line numbers on every block drive editor <-> preview scroll sync.
    o.render.sourcepos = true;
    // Raw HTML is let through here and then cleaned by `sanitize`, the same
    // approach GitHub takes: <details>, <img width>, <kbd> etc. survive,
    // scripts, event handlers and styles don't.
    o.render.r#unsafe = true;
    o
}

/// syntect with CSS classes, but keeping comrak's `<pre>` attributes
/// (syntect's class mode drops them, and scroll sync needs data-sourcepos).
struct Highlighter(SyntectAdapter);

impl SyntaxHighlighterAdapter for Highlighter {
    fn write_highlighted(&self, out: &mut dyn fmt::Write, lang: Option<&str>, code: &str) -> fmt::Result {
        self.0.write_highlighted(out, lang, code)
    }

    fn write_pre_tag(&self, out: &mut dyn fmt::Write, attributes: HashMap<&'static str, Cow<'_, str>>) -> fmt::Result {
        out.write_str("<pre class=\"syntax-highlighting\"")?;
        for name in ["lang", "data-sourcepos"] {
            if let Some(v) = attributes.get(name) {
                write!(out, " {name}=\"{}\"", escape_html(v))?;
            }
        }
        out.write_str(">")
    }

    fn write_code_tag(&self, out: &mut dyn fmt::Write, attributes: HashMap<&'static str, Cow<'_, str>>) -> fmt::Result {
        self.0.write_code_tag(out, attributes)
    }
}

fn highlighter() -> &'static Highlighter {
    static H: OnceLock<Highlighter> = OnceLock::new();
    H.get_or_init(|| Highlighter(SyntectAdapterBuilder::new().css().build()))
}

/// ```mermaid blocks become `<pre class="mermaid">` for mermaid.js to draw.
struct MermaidRenderer;

impl CodefenceRendererAdapter for MermaidRenderer {
    fn write(
        &self,
        out: &mut dyn fmt::Write,
        _lang: &str,
        _meta: &str,
        code: &str,
        sourcepos: Option<Sourcepos>,
    ) -> fmt::Result {
        out.write_str("<pre class=\"mermaid\"")?;
        if let Some(sp) = sourcepos {
            write!(out, " data-sourcepos=\"{sp}\"")?;
        }
        out.write_str(">")?;
        out.write_str(&escape_html(code))?;
        out.write_str("</pre>\n")
    }
}

static MERMAID: MermaidRenderer = MermaidRenderer;

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

thread_local! {
    static SANITIZER: ammonia::Builder<'static> = {
        let mut b = ammonia::Builder::default();
        b.add_tags([
            "details", "summary", "input", "picture", "source", "section", "mark", "ins",
            "kbd", "samp", "var", "figure", "figcaption",
        ]);
        b.add_generic_attributes(["class", "id", "align", "dir", "aria-label", "aria-hidden", "role"]);
        b.add_generic_attribute_prefixes(["data-"]);
        b.add_tag_attributes("input", ["type", "checked", "disabled"]);
        b.add_tag_attributes("details", ["open"]);
        b.add_tag_attributes("source", ["srcset", "media", "type"]);
        b.add_tag_attributes("img", ["srcset", "loading"]);
        b.add_tag_attributes("pre", ["lang"]);
        b.add_tag_attributes("ol", ["start", "type"]);
        b
    };
}

/// Strip anything that could run code or restyle the viewer from `html`.
pub fn sanitize(html: &str) -> String {
    SANITIZER.with(|b| b.clean(html).to_string())
}

pub fn render(markdown: &str) -> Rendered {
    let opts = options();
    let arena = Arena::new();
    let root = parse_document(&arena, markdown, &opts);

    // Same text + anchorizer comrak's HTML formatter uses, so ids match.
    let mut anchorizer = Anchorizer::new();
    let mut toc = Vec::new();
    for node in root.descendants() {
        let (level, line) = match node.data().value {
            NodeValue::Heading(ref h) => (h.level, node.data().sourcepos.start.line),
            _ => continue,
        };
        let text = node.collect_text();
        let anchor = anchorizer.anchorize(&text);
        toc.push(Heading { level, text, anchor, line });
    }

    let mut plugins = Plugins::default();
    plugins.render.codefence_syntax_highlighter = Some(highlighter());
    let mut renderers: HashMap<String, &dyn CodefenceRendererAdapter> = HashMap::new();
    renderers.insert("mermaid".into(), &MERMAID);
    plugins.render.codefence_renderers = renderers;
    let mut raw = String::new();
    format_html_with_plugins(root, &opts, &mut raw, &plugins).expect("write to String");
    let html = sanitize(&raw);
    let has_math = html.contains("data-math-style");
    let has_mermaid = html.contains("class=\"mermaid\"");
    Rendered { html, toc, has_math, has_mermaid }
}

/// CSS for syntect's class-based highlighting, light and dark variants.
fn highlight_css() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        let light = css_for_theme_with_class_style(&ts.themes["InspiredGitHub"], ClassStyle::Spaced)
            .unwrap_or_default();
        let dark = css_for_theme_with_class_style(&ts.themes["base16-ocean.dark"], ClassStyle::Spaced)
            .unwrap_or_default();
        // Nested under :root:not(.force-light) so printing/export stay light.
        format!("{light}\n@media (prefers-color-scheme: dark) {{\n:root:not(.force-light) {{\n{dark}\n}}\n}}")
    })
}

const PAGE_CSS: &str = include_str!("page.css");

/// All of the page's own CSS (layout, colors, code highlighting).
pub fn stylesheet() -> String {
    format!("{PAGE_CSS}\n{}", highlight_css())
}

/// A full HTML document with `body_html` inside `<article id="content">`.
/// Later edits replace only that element's contents (see `update_script`),
/// so the scroll position survives live preview.
pub fn page(body_html: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
         <meta name=\"color-scheme\" content=\"light dark\">\
         <style>{PAGE_CSS}</style><style>{hl}</style></head>\
         <body><article id=\"content\" class=\"markdown-body\">{body_html}</article></body></html>",
        hl = highlight_css()
    )
}

/// JavaScript that swaps the article's contents for `body_html`.
pub fn update_script(body_html: &str) -> String {
    let lit = serde_json::to_string(body_html).expect("string serializes");
    format!("document.getElementById('content').innerHTML = {lit};")
}

/// JavaScript that scrolls to the heading with `anchor`.
pub fn scroll_script(anchor: &str) -> String {
    let lit = serde_json::to_string(anchor).expect("string serializes");
    format!(
        "(() => {{ const e = document.getElementById({lit}); \
         if (e) e.scrollIntoView({{behavior: 'smooth', block: 'start'}}); }})();"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_anchors_match_rendered_ids() {
        let md = "# Intro\n\n## Set `up` & run\n\n# Intro\n\ntext\n\n### Ünïcode heading\n";
        let r = render(md);
        let texts: Vec<_> = r.toc.iter().map(|h| (h.level, h.text.as_str())).collect();
        assert_eq!(
            texts,
            [(1, "Intro"), (2, "Set up & run"), (1, "Intro"), (3, "Ünïcode heading")]
        );
        for h in &r.toc {
            assert!(
                r.html.contains(&format!("id=\"{}\"", h.anchor)),
                "anchor {:?} missing from html:\n{}",
                h.anchor,
                r.html
            );
        }
        let lines: Vec<_> = r.toc.iter().map(|h| h.line).collect();
        assert_eq!(lines, [1, 3, 5, 9]);
        // Duplicate headings get distinct anchors.
        assert_ne!(r.toc[0].anchor, r.toc[2].anchor);
    }

    #[test]
    fn gfm_features_render() {
        let md = "| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n\n~~gone~~\n\n\
                  see https://example.com\n\nnote[^1]\n\n[^1]: footnote text\n\n\
                  > [!WARNING]\n> careful\n";
        let h = render(md).html;
        for needle in [
            "<table ",
            "type=\"checkbox\"",
            "gone</del>",
            "href=\"https://example.com\"",
            "footnote text",
            "markdown-alert-warning",
        ] {
            assert!(h.contains(needle), "{needle:?} missing from:\n{h}");
        }
    }

    #[test]
    fn code_blocks_are_highlighted_with_classes() {
        let h = render("```rust\nfn main() { let x = 1; }\n```\n").html;
        assert!(h.contains("class=\"source rust\"") || h.contains("<span class=\""), "{h}");
        assert!(!h.contains("style=\""), "expected classes, not inline styles: {h}");
        assert!(highlight_css().contains("prefers-color-scheme: dark"));
    }

    #[test]
    fn dangerous_html_is_removed() {
        let md = "hi <script>alert(1)</script> <img src=x onerror=alert(1)>\n\n\
                  <a href=\"javascript:alert(1)\">x</a> <p style=\"position:fixed\">s</p>\n\n\
                  <iframe src=\"https://example.com\"></iframe><style>body{display:none}</style>\n";
        let h = render(md).html;
        for bad in ["<script", "alert(1)</", "onerror", "javascript:", "style=", "<iframe", "<style"] {
            assert!(!h.contains(bad), "{bad:?} survived:\n{h}");
        }
    }

    #[test]
    fn common_readme_html_is_kept() {
        let md = "<p align=\"center\"><img src=\"logo.png\" width=\"120\" alt=\"Logo\"></p>\n\n\
                  <details><summary>More</summary>\n\nHidden **text**\n\n</details>\n\n\
                  Press <kbd>Ctrl</kbd>+<kbd>C</kbd><br>H<sub>2</sub>O and x<sup>2</sup>\n";
        let h = render(md).html;
        for good in [
            "align=\"center\"",
            "width=\"120\"",
            "src=\"logo.png\"",
            "<details>",
            "<summary>More</summary>",
            "text</strong>",
            "<kbd>Ctrl</kbd>",
            "<br>",
            "<sub>2</sub>",
        ] {
            assert!(h.contains(good), "{good:?} missing:\n{h}");
        }
    }

    #[test]
    fn math_and_mermaid_are_marked_for_the_page() {
        let r = render("Inline $x^2$ and\n\n$$\\int_0^1 f$$\n\n```math\na+b\n```\n");
        assert!(r.has_math && !r.has_mermaid);
        assert!(r.html.contains("data-math-style=\"inline\""), "{}", r.html);
        assert!(r.html.contains("data-math-style=\"display\""), "{}", r.html);

        let r = render("```mermaid\ngraph TD\n  A-->B & \"C\"\n```\n");
        assert!(r.has_mermaid && !r.has_math);
        assert!(r.html.contains("<pre class=\"mermaid\" data-sourcepos=\"1:1-4:3\">"), "{}", r.html);
        assert!(r.html.contains("A--&gt;B &amp; \"C\""), "{}", r.html);
    }

    #[test]
    fn blocks_carry_source_lines() {
        let h = render("```python\nx = 1\n```\n\n```\nplain\n```\n").html;
        assert!(h.contains("<pre class=\"syntax-highlighting\" lang=\"python\" data-sourcepos=\"1:1-3:3\">"), "{h}");
        assert!(h.contains("<pre class=\"syntax-highlighting\" data-sourcepos=\"5:1-7:3\">"), "{h}");
        let h = render("# T\n\npara\n\n- a\n- b\n").html;
        assert!(h.contains("<h1 id=\"t\" data-sourcepos=\"1:1-1:3\""), "{h}");
        assert!(h.contains("<p data-sourcepos=\"3:1-3:4\""), "{h}");
    }

    #[test]
    fn front_matter_is_hidden() {
        let r = render("---\ntitle: x\n---\n# Body\n");
        assert!(!r.html.contains("title: x"), "{}", r.html);
        assert_eq!(r.toc.len(), 1);
        assert_eq!(r.toc[0].line, 4, "line numbers count the front matter");
    }

    #[test]
    fn update_script_escapes_safely() {
        let s = update_script("</script>\"'\\\n\u{2028}");
        assert!(s.starts_with("document.getElementById('content').innerHTML = \""));
        assert!(!s.contains('\n'));
    }
}

