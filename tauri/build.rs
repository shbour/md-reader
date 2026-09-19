//! Assemble the web UI Tauri embeds (`dist/`) from `ui/` plus the vendored
//! libraries, then run Tauri's own build step.

use std::fs;
use std::path::Path;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dist dir");
    for entry in fs::read_dir(from).expect("read ui dir") {
        let entry = entry.expect("dir entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).expect("copy ui file");
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dist = root.join("dist");
    let vendor = root.join("../vendor");

    copy_dir(&root.join("ui"), &dist);
    println!("cargo:rerun-if-changed=ui");

    for (src, dest) in [
        ("katex/katex.min.js", "vendor/katex.min.js"),
        ("katex/katex.inline.css", "vendor/katex.css"),
        ("mermaid/mermaid.min.js", "vendor/mermaid.min.js"),
        ("codemirror/codemirror.min.js", "vendor/codemirror.min.js"),
    ] {
        let from = vendor.join(src);
        let to = dist.join(dest);
        fs::create_dir_all(to.parent().unwrap()).expect("create vendor dir");
        fs::copy(&from, &to).unwrap_or_else(|e| panic!("copy {}: {e}", from.display()));
        println!("cargo:rerun-if-changed={}", from.display());
    }

    tauri_build::build();
}
