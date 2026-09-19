mod assets;
mod export;
mod links;
mod render;
mod state;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "io.github.mdreader.MdReader";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.add_main_option(
        "new",
        glib::Char::from(b'n'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Start a new document",
        None,
    );
    // `mdreader --new` (the launcher's "New Document" action): hand off to
    // the running instance, or start one, and open a blank document.
    app.connect_handle_local_options(|app, opts| {
        use std::ops::ControlFlow;
        if !opts.contains("new") {
            return ControlFlow::Continue(());
        }
        if let Err(e) = app.register(None::<&gio::Cancellable>) {
            eprintln!("mdreader: {e}");
            return ControlFlow::Break(glib::ExitCode::FAILURE);
        }
        app.activate_action("new", None);
        if app.is_remote() {
            return ControlFlow::Break(glib::ExitCode::SUCCESS);
        }
        ControlFlow::Continue(())
    });

    app.connect_startup(|app| {
        sourceview5::init();
        setup_app_actions(app);
    });
    app.connect_activate(|app| match app.active_window() {
        // `--new` already opened a window; don't add an empty second one.
        Some(w) => w.present(),
        None => window::Win::new(app).window.present(),
    });
    app.connect_open(|app, files, _hint| {
        for f in files {
            window::open(app, f);
        }
    });
    app.run()
}

fn setup_app_actions(app: &adw::Application) {
    let new = gio::SimpleAction::new("new", None);
    let a = app.clone();
    new.connect_activate(move |_, _| window::new_document(&a));
    app.add_action(&new);

    // Close windows one by one so each can ask about unsaved changes.
    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate(|_, _| {
        for w in window::all() {
            w.window.close();
        }
    });
    app.add_action(&quit);

    let about = gio::SimpleAction::new("about", None);
    let a = app.clone();
    about.connect_activate(move |_, _| {
        let dialog = adw::AboutDialog::builder()
            .application_name("Markdown Reader")
            .application_icon(APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Read, edit and preview Markdown files.")
            .license_type(gtk::License::MitX11)
            .build();
        dialog.add_legal_section(
            "KaTeX (math)",
            Some("© 2013-2020 Khan Academy and other contributors"),
            gtk::License::MitX11,
            None,
        );
        dialog.add_legal_section("Mermaid (diagrams)", Some("© 2014-2022 Knut Sveidqvist"), gtk::License::MitX11, None);
        dialog.present(a.active_window().as_ref());
    });
    app.add_action(&about);

    for (action, accels) in [
        ("app.new", &["<Ctrl>n"][..]),
        ("app.quit", &["<Ctrl>q"]),
        ("win.open", &["<Ctrl>o"]),
        ("win.save", &["<Ctrl>s"]),
        ("win.save-as", &["<Ctrl><Shift>s"]),
        ("win.print", &["<Ctrl>p"]),
        ("win.edit", &["<Ctrl>e"]),
        ("win.toc", &["F9"]),
        ("win.find", &["<Ctrl>f"]),
        ("win.find-next", &["<Ctrl>g"]),
        ("win.find-previous", &["<Ctrl><Shift>g"]),
        ("win.back", &["<Alt>Left"]),
        ("win.close", &["<Ctrl>w"]),
        ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]),
        ("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]),
        ("win.zoom-reset", &["<Ctrl>0", "<Ctrl>KP_0"]),
    ] {
        app.set_accels_for_action(action, accels);
    }
}
