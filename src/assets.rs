//! JavaScript and CSS bundled into the binary so the preview works offline.
//! Refresh the vendored libraries with `vendor/update.sh`.

use std::sync::OnceLock;

pub const RUNTIME_JS: &str = include_str!("runtime.js");
pub const KATEX_JS: &str = include_str!("../vendor/katex/katex.min.js");
/// KaTeX's stylesheet with its fonts inlined as data: URIs.
pub const KATEX_CSS: &str = include_str!("../vendor/katex/katex.inline.css");
pub const MERMAID_JS: &str = include_str!("../vendor/mermaid/mermaid.min.js");

/// Script that adds KaTeX's stylesheet to the current page.
pub fn katex_css_script() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| {
        let lit = serde_json::to_string(KATEX_CSS).expect("string serializes");
        format!(
            "(() => {{ const s = document.createElement('style'); s.id = 'katex-css'; \
             s.textContent = {lit}; document.head.appendChild(s); }})();"
        )
    })
}
