//! The Markdown files under a folder, for the Files sidebar.

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::links::is_markdown;

/// One row of the folder tree, in display order (a folder, then what is in it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    /// 0 for what sits directly in the opened folder.
    pub depth: usize,
    pub is_dir: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// The folder held more than the listing shows (see the limits below).
    pub truncated: bool,
}

/// Enough for any documentation tree; stops a scan of `~` or `/` from hanging the app.
const MAX_ENTRIES: usize = 5_000;
/// Folders read, listed or not: bounds the time a scan takes.
const MAX_DIRS_READ: usize = 5_000;
const MAX_DEPTH: usize = 16;
/// Never worth searching for documents to read.
const SKIPPED_DIRS: &[&str] = &["node_modules"];

/// The Markdown files under `root` and the folders that lead to them. Hidden
/// files and folders are left out, and so are folders with no Markdown in
/// them. Symlinked files are listed; symlinked folders are not entered, so a
/// link loop can't recurse forever. Folders come first, then files, each
/// sorted by name.
///
/// Folders are read level by level, so when a big tree hits the limits it is
/// the deepest folders that are missing, not everything after the first big one.
pub fn scan(root: &Path) -> std::io::Result<Listing> {
    std::fs::read_dir(root)?; // report an unreadable root; unreadable subfolders are skipped
    let mut truncated = false;
    let mut read: HashMap<PathBuf, DirContents> = HashMap::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0)]);
    let mut files = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        if read.len() >= MAX_DIRS_READ || files >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let c = read_dir(&dir);
        files += c.files.len();
        if depth + 1 < MAX_DEPTH {
            queue.extend(c.dirs.iter().map(|(_, p)| (p.clone(), depth + 1)));
        } else if !c.dirs.is_empty() {
            truncated = true;
        }
        read.insert(dir, c);
    }
    let mut entries = Vec::new();
    emit(root, 0, &read, &mut entries);
    if entries.len() > MAX_ENTRIES {
        entries.truncate(MAX_ENTRIES);
        // A folder at the very end would show nothing inside it.
        while entries.last().is_some_and(|e| e.is_dir) {
            entries.pop();
        }
        truncated = true;
    }
    Ok(Listing { entries, truncated })
}

/// A folder's subfolders and Markdown files, each sorted by name.
#[derive(Default)]
struct DirContents {
    dirs: Vec<(String, PathBuf)>,
    files: Vec<(String, PathBuf)>,
}

fn read_dir(dir: &Path) -> DirContents {
    let mut c = DirContents::default();
    let Ok(entries) = std::fs::read_dir(dir) else { return c };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                c.dirs.push((name, path));
            }
        } else if (ft.is_file() || (ft.is_symlink() && path.is_file())) && is_markdown(&path) {
            c.files.push((name, path));
        }
    }
    c.dirs.sort_by(|a, b| by_name(&a.0, &b.0));
    c.files.sort_by(|a, b| by_name(&a.0, &b.0));
    c
}

/// Append `dir`'s rows in display order. False if it had nothing to show.
fn emit(dir: &Path, depth: usize, read: &HashMap<PathBuf, DirContents>, out: &mut Vec<Entry>) -> bool {
    let Some(c) = read.get(dir) else { return false }; // not reached before the limits
    let start = out.len();
    for (name, path) in &c.dirs {
        let at = out.len();
        out.push(Entry { path: path.clone(), name: name.clone(), depth, is_dir: true });
        if !emit(path, depth + 1, read, out) {
            out.truncate(at);
        }
    }
    for (name, path) in &c.files {
        out.push(Entry { path: path.clone(), name: name.clone(), depth, is_dir: false });
    }
    out.len() > start
}

/// Case-insensitive, with runs of digits compared as numbers, so
/// "chapter-2" sorts before "chapter-10".
fn by_name(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[&str]) -> PathBuf {
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "mdreader-folder-test-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for f in files {
            let p = root.join(f);
            if f.ends_with('/') {
                std::fs::create_dir_all(&p).unwrap();
            } else {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, "# x").unwrap();
            }
        }
        root
    }

    fn shown(l: &Listing) -> Vec<String> {
        l.entries
            .iter()
            .map(|e| format!("{}{}{}", "  ".repeat(e.depth), e.name, if e.is_dir { "/" } else { "" }))
            .collect()
    }

    #[test]
    fn lists_markdown_folders_first_and_prunes_the_rest() {
        let root = tree(&[
            "README.md",
            "notes.txt",
            "Guide.markdown",
            "docs/b.md",
            "docs/A.MD",
            "docs/img/logo.png",
            "docs/deep/er/x.md",
            "empty/",
            "only-images/a.png",
            ".git/HEAD.md",
            ".hidden.md",
            "node_modules/pkg/readme.md",
        ]);
        let l = scan(&root).unwrap();
        assert_eq!(
            shown(&l),
            ["docs/", "  deep/", "    er/", "      x.md", "  A.MD", "  b.md", "Guide.markdown", "README.md"]
        );
        assert!(!l.truncated);
        assert_eq!(l.entries[3].path, root.join("docs/deep/er/x.md"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn numbers_sort_by_value() {
        let mut v = vec!["ch10.md", "ch2.md", "Ch1.md", "ch02b.md", "appendix.md"];
        v.sort_by(|a, b| by_name(a, b));
        assert_eq!(v, ["appendix.md", "Ch1.md", "ch2.md", "ch02b.md", "ch10.md"]);
    }

    #[test]
    fn missing_root_is_an_error() {
        assert!(scan(Path::new("/nonexistent/mdreader")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_folders_are_not_entered() {
        let root = tree(&["a/x.md"]);
        std::os::unix::fs::symlink(&root, root.join("a/loop")).unwrap();
        std::os::unix::fs::symlink(root.join("a/x.md"), root.join("link.md")).unwrap();
        let l = scan(&root).unwrap();
        assert_eq!(shown(&l), ["a/", "  x.md", "link.md"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn deep_trees_of_empty_folders_stop_early() {
        // Wide, not deep: a chain would hit MAX_DEPTH first.
        let names: Vec<String> = (0..MAX_DIRS_READ + 10).map(|i| format!("d{i}/")).collect();
        let mut refs: Vec<&str> = names.iter().map(String::as_str).collect();
        refs.push("zz.md");
        let root = tree(&refs);
        let l = scan(&root).unwrap();
        assert!(l.truncated);
        assert_eq!(shown(&l), ["zz.md"], "files of folders already read are still listed");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn one_big_folder_does_not_hide_the_others() {
        // "a" holds more folders than a scan reads; "b", after it, still shows.
        let names: Vec<String> = (0..MAX_DIRS_READ + 10).map(|i| format!("a/d{i}/")).collect();
        let mut refs: Vec<&str> = names.iter().map(String::as_str).collect();
        refs.push("b/x.md");
        let root = tree(&refs);
        let l = scan(&root).unwrap();
        assert!(l.truncated);
        assert_eq!(shown(&l), ["b/", "  x.md"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn huge_folders_are_cut_short() {
        let names: Vec<String> = (0..MAX_ENTRIES + 10).map(|i| format!("{i}.md")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let root = tree(&refs);
        let l = scan(&root).unwrap();
        assert_eq!(l.entries.len(), MAX_ENTRIES);
        assert!(l.truncated);
        let _ = std::fs::remove_dir_all(root);
    }
}
