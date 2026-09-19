//! Platform-independent parts of Markdown Reader: Markdown -> sanitised HTML,
//! the preview page and its runtime script, link handling and HTML export.
//! The GTK and Tauri front ends both build on this crate.

pub mod assets;
pub mod export;
pub mod links;
pub mod render;
