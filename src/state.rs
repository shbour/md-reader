//! Window layout remembered between sessions, in ~/.config/mdreader/state.ini.

use std::path::PathBuf;

use gtk::glib;

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    pub zoom: f64,
    /// `None` until the user first shows or hides the contents sidebar.
    pub sidebar: Option<bool>,
    /// Editor share of the edit/preview split, 0..1.
    pub split: f64,
}

impl Default for State {
    fn default() -> Self {
        State { width: 1100, height: 780, maximized: false, zoom: 1.0, sidebar: None, split: 0.5 }
    }
}

const GROUP: &str = "window";

fn path() -> PathBuf {
    glib::user_config_dir().join("mdreader").join("state.ini")
}

pub fn load() -> State {
    load_from(&path())
}

pub fn save(s: &State) {
    if let Err(e) = save_to(&path(), s) {
        eprintln!("mdreader: could not save window state: {e}");
    }
}

fn load_from(path: &std::path::Path) -> State {
    let d = State::default();
    let kf = glib::KeyFile::new();
    if kf.load_from_file(path, glib::KeyFileFlags::NONE).is_err() {
        return d;
    }
    State {
        width: kf.integer(GROUP, "width").ok().filter(|w| (360..=20000).contains(w)).unwrap_or(d.width),
        height: kf.integer(GROUP, "height").ok().filter(|h| (300..=20000).contains(h)).unwrap_or(d.height),
        maximized: kf.boolean(GROUP, "maximized").unwrap_or(d.maximized),
        zoom: kf.double(GROUP, "zoom").ok().filter(|z| (0.3..=4.0).contains(z)).unwrap_or(d.zoom),
        sidebar: kf.boolean(GROUP, "sidebar").ok(),
        split: kf.double(GROUP, "split").ok().filter(|s| (0.1..=0.9).contains(s)).unwrap_or(d.split),
    }
}

fn save_to(path: &std::path::Path, s: &State) -> Result<(), glib::Error> {
    let kf = glib::KeyFile::new();
    kf.set_integer(GROUP, "width", s.width);
    kf.set_integer(GROUP, "height", s.height);
    kf.set_boolean(GROUP, "maximized", s.maximized);
    kf.set_double(GROUP, "zoom", s.zoom);
    if let Some(b) = s.sidebar {
        kf.set_boolean(GROUP, "sidebar", b);
    }
    kf.set_double(GROUP, "split", s.split);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    kf.save_to_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_nonsense() {
        let dir = std::env::temp_dir().join(format!("mdreader-state-test-{}", std::process::id()));
        let p = dir.join("state.ini");
        assert_eq!(load_from(&p), State::default(), "missing file gives defaults");

        let s = State { width: 1400, height: 900, maximized: true, zoom: 1.25, sidebar: Some(false), split: 0.4 };
        save_to(&p, &s).unwrap();
        assert_eq!(load_from(&p), s);

        std::fs::write(&p, "[window]\nwidth=5\nzoom=99\nsplit=abc\nsidebar=true\n").unwrap();
        let l = load_from(&p);
        assert_eq!((l.width, l.zoom, l.split, l.sidebar), (1100, 1.0, 0.5, Some(true)));
        let _ = std::fs::remove_dir_all(dir);
    }
}
