//! Desktop entry point; the app itself is in `lib.rs`, which Android loads as
//! a library.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mdreader_app::run();
}
