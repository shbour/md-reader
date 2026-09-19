//! One reader/editor window per document.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use sourceview5::prelude::*;
use webkit6::prelude::*;

use crate::links::{self, LinkAction};
use crate::render::{self, Heading};
use crate::{assets, export, state};

const APP_NAME: &str = "Markdown Reader";
const RENDER_DELAY: Duration = Duration::from_millis(120);
const DISK_CHECK_DELAY: Duration = Duration::from_millis(250);
const EMPTY_STATE: &str = "<div class=\"empty-state\"><p>Open a Markdown file with <b>Ctrl+O</b>, \
    drop one here, or start a new one with <b>Ctrl+N</b>.</p></div>";

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<Win>>> = const { RefCell::new(Vec::new()) };
}

pub struct Win {
    app: adw::Application,
    pub window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    banner: adw::Banner,
    split: adw::OverlaySplitView,
    toc_list: gtk::ListBox,
    toc: RefCell<Vec<Heading>>,
    /// The user's last explicit choice to show/hide the contents sidebar.
    sidebar_pref: Cell<Option<bool>>,
    paned: gtk::Paned,
    editor_scroll: gtk::ScrolledWindow,
    view: sourceview5::View,
    buffer: sourceview5::Buffer,
    web: webkit6::WebView,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    match_label: gtk::Label,
    back_btn: gtk::Button,
    save_btn: gtk::Button,
    search_settings: sourceview5::SearchSettings,
    search_ctx: sourceview5::SearchContext,
    /// The match the search bar is on. Marked with a tag rather than the
    /// selection: typing in the search entry takes the PRIMARY clipboard,
    /// which makes GTK clear the editor's selection.
    current_match_tag: gtk::TextTag,
    match_tag: gtk::TextTag,

    /// Editor share of the edit/preview split.
    split_ratio: Cell<f64>,
    applying_split: Cell<bool>,
    /// What the current document needs the page to load.
    has_math: Cell<bool>,
    has_mermaid: Cell<bool>,
    katex_loaded: Cell<bool>,
    mermaid_loaded: Cell<bool>,
    /// Editor scrolls we caused ourselves (following the preview) until then.
    ignore_editor_scroll_until: Cell<Option<Instant>>,
    print_op: RefCell<Option<webkit6::PrintOperation>>,

    file: RefCell<Option<gio::File>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    /// The text last read from or written to disk; lets the file monitor
    /// ignore our own saves and no-op touches.
    disk_text: RefCell<String>,
    history: RefCell<Vec<PathBuf>>,
    editing: Cell<bool>,
    /// True while we replace the buffer programmatically.
    loading: Cell<bool>,
    force_close: Cell<bool>,
    page_base: RefCell<String>,
    page_ready: Cell<bool>,
    pending_anchor: RefCell<Option<String>>,
    render_timer: RefCell<Option<glib::SourceId>>,
    disk_timer: RefCell<Option<glib::SourceId>>,
}

/// Open `file` in a new window, or raise the window already showing it.
/// An empty, untouched window is reused.
pub fn open(app: &adw::Application, file: &gio::File) {
    let path = file.path();
    let existing = WINDOWS.with_borrow(|ws| {
        ws.iter()
            .find(|w| path.is_some() && w.path() == path)
            .or_else(|| ws.iter().find(|w| w.is_blank()))
            .cloned()
    });
    let win = existing.unwrap_or_else(|| Win::new(app));
    if win.path() != path {
        win.load_file(file, true);
    }
    win.window.present();
}

pub fn new_document(app: &adw::Application) {
    let win = Win::new(app);
    win.set_editing(true);
    win.window.present();
}

pub fn all() -> Vec<Rc<Win>> {
    WINDOWS.with_borrow(|ws| ws.clone())
}

impl Win {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        // ---- editor ----
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_highlight_matching_brackets(false);
        if let Some(lang) = sourceview5::LanguageManager::default().language("markdown") {
            buffer.set_language(Some(&lang));
        }
        let view = sourceview5::View::with_buffer(&buffer);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_show_line_numbers(true);
        view.set_highlight_current_line(true);
        view.set_auto_indent(true);
        view.set_tab_width(4);
        view.set_insert_spaces_instead_of_tabs(true);
        view.set_left_margin(8);
        view.set_right_margin(8);
        view.set_top_margin(8);
        view.set_bottom_margin(48);
        let search_settings = sourceview5::SearchSettings::builder()
            .case_sensitive(false)
            .wrap_around(true)
            .build();
        let search_ctx = sourceview5::SearchContext::new(&buffer, Some(&search_settings));
        // We highlight matches ourselves: the context's own highlighting keeps
        // raising its tag above ours, hiding which match is current.
        search_ctx.set_highlight(false);
        let match_tag = buffer.create_tag(Some("search-match"), &[]).expect("new tag name");
        match_tag.set_background(Some("#f6d32d"));
        match_tag.set_foreground(Some("#1f2328"));
        // Created later, so it has the higher priority.
        let current_match_tag = buffer.create_tag(Some("current-match"), &[]).expect("new tag name");
        current_match_tag.set_background(Some("#ff7800"));
        current_match_tag.set_foreground(Some("#ffffff"));
        let editor_scroll = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(260)
            .visible(false)
            .build();

        // ---- preview ----
        // The page reports its scroll position through window.webkit.messageHandlers.mdr.
        let ucm = webkit6::UserContentManager::new();
        ucm.register_script_message_handler("mdr", None);
        let web = webkit6::WebView::builder().user_content_manager(&ucm).build();
        if let Some(s) = webkit6::prelude::WebViewExt::settings(&web) {
            // Our own evaluate_javascript() still works; scripts in page markup don't.
            s.set_enable_javascript_markup(false);
            s.set_enable_developer_extras(false);
            s.set_enable_back_forward_navigation_gestures(false);
        }
        web.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
        web.set_hexpand(true);
        web.set_vexpand(true);
        web.set_width_request(260);

        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&editor_scroll)
            .end_child(&web)
            .resize_start_child(true)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .wide_handle(true)
            .build();

        // ---- search ----
        let search_entry = gtk::SearchEntry::builder()
            .placeholder_text("Find in document")
            .hexpand(true)
            .build();
        let match_label = gtk::Label::builder().css_classes(["dim-label", "numeric"]).build();
        let prev_btn = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text("Previous Match (Shift+Enter)")
            .build();
        let next_btn = gtk::Button::builder()
            .icon_name("go-down-symbolic")
            .tooltip_text("Next Match (Enter)")
            .build();
        let search_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        search_box.set_width_request(360);
        search_box.append(&search_entry);
        search_box.append(&match_label);
        search_box.append(&prev_btn);
        search_box.append(&next_btn);
        let search_bar = gtk::SearchBar::builder().child(&search_box).show_close_button(true).build();
        search_bar.connect_entry(&search_entry);

        let banner = adw::Banner::builder()
            .title("The file changed on disk")
            .button_label("Reload")
            .revealed(false)
            .build();

        // ---- sidebar ----
        let toc_list = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .selection_mode(gtk::SelectionMode::None)
            .build();
        let toc_scroll = gtk::ScrolledWindow::builder()
            .child(&toc_list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let toc_page = adw::ToolbarView::new();
        let toc_header = adw::HeaderBar::builder()
            .show_title(true)
            .title_widget(&adw::WindowTitle::new("Contents", ""))
            .build();
        toc_page.add_top_bar(&toc_header);
        toc_page.set_content(Some(&toc_scroll));

        // ---- content column ----
        let back_btn = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text("Back (Alt+Left)")
            .action_name("win.back")
            .visible(false)
            .build();
        let toc_btn = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Contents (F9)")
            .action_name("win.toc")
            .build();
        let open_btn = gtk::Button::builder()
            .label("Open")
            .tooltip_text("Open a File (Ctrl+O)")
            .action_name("win.open")
            .build();
        let edit_btn = gtk::ToggleButton::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Edit (Ctrl+E)")
            .action_name("win.edit")
            .build();
        let save_btn = gtk::Button::builder()
            .label("Save")
            .tooltip_text("Save (Ctrl+S)")
            .action_name("win.save")
            .css_classes(["suggested-action"])
            .visible(false)
            .build();
        let menu = gio::Menu::new();
        let sec1 = gio::Menu::new();
        sec1.append(Some("New Document"), Some("app.new"));
        sec1.append(Some("Open…"), Some("win.open"));
        sec1.append(Some("Save As…"), Some("win.save-as"));
        menu.append_section(None, &sec1);
        let sec_out = gio::Menu::new();
        sec_out.append(Some("Print…"), Some("win.print"));
        sec_out.append(Some("Export as PDF…"), Some("win.export-pdf"));
        sec_out.append(Some("Export as HTML…"), Some("win.export-html"));
        menu.append_section(None, &sec_out);
        let sec2 = gio::Menu::new();
        sec2.append(Some("Find…"), Some("win.find"));
        sec2.append(Some("Zoom In"), Some("win.zoom-in"));
        sec2.append(Some("Zoom Out"), Some("win.zoom-out"));
        sec2.append(Some("Reset Zoom"), Some("win.zoom-reset"));
        menu.append_section(None, &sec2);
        let sec3 = gio::Menu::new();
        sec3.append(Some("About Markdown Reader"), Some("app.about"));
        menu.append_section(None, &sec3);
        let menu_btn = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text("Main Menu")
            .build();

        let title = adw::WindowTitle::new(APP_NAME, "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&toc_btn);
        header.pack_start(&back_btn);
        header.pack_start(&open_btn);
        header.pack_end(&menu_btn);
        header.pack_end(&edit_btn);
        header.pack_end(&save_btn);

        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.add_top_bar(&search_bar);
        content.add_top_bar(&banner);
        content.set_content(Some(&paned));

        let split = adw::OverlaySplitView::builder()
            .sidebar(&toc_page)
            .content(&content)
            .show_sidebar(false)
            .min_sidebar_width(200.0)
            .max_sidebar_width(300.0)
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));

        let saved = state::load();
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(APP_NAME)
            .default_width(saved.width)
            .default_height(saved.height)
            .maximized(saved.maximized)
            .content(&toasts)
            .build();
        web.set_zoom_level(saved.zoom);
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            720.0,
            adw::LengthUnit::Sp,
        ));
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);

        let win = Rc::new(Win {
            app: app.clone(),
            window,
            title,
            toasts,
            banner,
            split,
            toc_list,
            toc: RefCell::new(Vec::new()),
            sidebar_pref: Cell::new(saved.sidebar),
            paned,
            editor_scroll,
            view,
            buffer,
            web,
            search_bar,
            search_entry,
            match_label,
            back_btn,
            save_btn,
            search_settings,
            search_ctx,
            current_match_tag,
            match_tag,
            split_ratio: Cell::new(saved.split),
            applying_split: Cell::new(false),
            has_math: Cell::new(false),
            has_mermaid: Cell::new(false),
            katex_loaded: Cell::new(false),
            mermaid_loaded: Cell::new(false),
            ignore_editor_scroll_until: Cell::new(None),
            print_op: RefCell::new(None),
            file: RefCell::new(None),
            monitor: RefCell::new(None),
            disk_text: RefCell::new(String::new()),
            history: RefCell::new(Vec::new()),
            editing: Cell::new(false),
            loading: Cell::new(false),
            force_close: Cell::new(false),
            page_base: RefCell::new(String::new()),
            page_ready: Cell::new(false),
            pending_anchor: RefCell::new(None),
            render_timer: RefCell::new(None),
            disk_timer: RefCell::new(None),
        });
        win.setup_actions();
        win.setup_signals(&prev_btn, &next_btn);
        win.follow_style_scheme();
        WINDOWS.with_borrow_mut(|ws| ws.push(win.clone()));
        win.update_title();
        win.render_now();
        win
    }

    fn path(&self) -> Option<PathBuf> {
        self.file.borrow().as_ref().and_then(|f| f.path())
    }

    fn is_blank(&self) -> bool {
        self.file.borrow().is_none() && !self.buffer.is_modified() && self.buffer.char_count() == 0
    }

    fn text(&self) -> String {
        let (s, e) = self.buffer.bounds();
        self.buffer.text(&s, &e, true).to_string()
    }

    fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::new(msg));
    }

    // ------------------------------------------------------------------ setup

    fn setup_actions(self: &Rc<Self>) {
        let group = &self.window;
        let add = |name: &str, f: Box<dyn Fn(&Rc<Win>)>| {
            let a = gio::SimpleAction::new(name, None);
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(w) = w.upgrade() {
                    f(&w)
                }
            });
            group.add_action(&a);
        };
        add("open", Box::new(|w| w.choose_file()));
        add("save", Box::new(|w| {
            let w = w.clone();
            glib::spawn_future_local(async move {
                w.save().await;
            });
        }));
        add("save-as", Box::new(|w| {
            let w = w.clone();
            glib::spawn_future_local(async move {
                w.save_as().await;
            });
        }));
        add("find", Box::new(|w| {
            w.search_bar.set_search_mode(true);
            w.search_entry.grab_focus();
        }));
        add("back", Box::new(|w| w.go_back()));
        add("find-next", Box::new(|w| w.search_step(true)));
        add("find-previous", Box::new(|w| w.search_step(false)));
        add("close", Box::new(|w| w.window.close()));
        add("zoom-in", Box::new(|w| w.web.set_zoom_level((w.web.zoom_level() * 1.1).min(4.0))));
        add("zoom-out", Box::new(|w| w.web.set_zoom_level((w.web.zoom_level() / 1.1).max(0.3))));
        add("zoom-reset", Box::new(|w| w.web.set_zoom_level(1.0)));
        add("print", Box::new(|w| w.print()));
        add("export-pdf", Box::new(|w| {
            let w = w.clone();
            glib::spawn_future_local(async move { w.export_pdf().await });
        }));
        add("export-html", Box::new(|w| {
            let w = w.clone();
            glib::spawn_future_local(async move { w.export_html().await });
        }));

        let edit = gio::SimpleAction::new_stateful("edit", None, &false.to_variant());
        let w = Rc::downgrade(self);
        edit.connect_change_state(move |_, v| {
            if let (Some(w), Some(on)) = (w.upgrade(), v.and_then(|v| v.get::<bool>())) {
                w.set_editing(on);
            }
        });
        group.add_action(&edit);

        let toc = gio::SimpleAction::new_stateful("toc", None, &false.to_variant());
        let w = Rc::downgrade(self);
        toc.connect_change_state(move |_, v| {
            if let (Some(w), Some(on)) = (w.upgrade(), v.and_then(|v| v.get::<bool>())) {
                w.sidebar_pref.set(Some(on));
                w.split.set_show_sidebar(on);
            }
        });
        group.add_action(&toc);

        if std::env::var_os("MDREADER_DEBUG").is_some() {
            // Test hooks, driven over D-Bus with org.gtk.Actions.Activate.
            let a = gio::SimpleAction::new("debug-set-text", Some(glib::VariantTy::STRING));
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, v| {
                if let (Some(w), Some(t)) = (w.upgrade(), v.and_then(|v| v.get::<String>())) {
                    w.buffer.set_text(&t);
                }
            });
            group.add_action(&a);
            let a = gio::SimpleAction::new("debug-snapshot", Some(glib::VariantTy::STRING));
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, v| {
                if let (Some(w), Some(p)) = (w.upgrade(), v.and_then(|v| v.get::<String>())) {
                    w.snapshot_to_png(&p);
                }
            });
            group.add_action(&a);
            let a = gio::SimpleAction::new("debug-toc", Some(glib::VariantTy::INT32));
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, v| {
                if let (Some(w), Some(i)) = (w.upgrade(), v.and_then(|v| v.get::<i32>())) {
                    if let Some(row) = w.toc_list.row_at_index(i) {
                        row.activate();
                    }
                }
            });
            group.add_action(&a);
            let a = gio::SimpleAction::new("debug-js", Some(glib::VariantTy::STRING));
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, v| {
                if let (Some(w), Some(js)) = (w.upgrade(), v.and_then(|v| v.get::<String>())) {
                    w.run_js(&js);
                }
            });
            group.add_action(&a);
            for (name, pdf) in [("debug-export-pdf", true), ("debug-export-html", false)] {
                let a = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
                let w = Rc::downgrade(self);
                a.connect_activate(move |_, v| {
                    if let (Some(w), Some(p)) = (w.upgrade(), v.and_then(|v| v.get::<String>())) {
                        glib::spawn_future_local(async move { w.debug_export(&p, pdf).await });
                    }
                });
                group.add_action(&a);
            }
            let a = gio::SimpleAction::new("debug-eval", Some(glib::VariantTy::STRING));
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, v| {
                if let (Some(w), Some(js)) = (w.upgrade(), v.and_then(|v| v.get::<String>())) {
                    // Editor state, and the sync anchor substituted for ANCHOR/F in `js`.
                    let adj = w.editor_scroll.vadjustment();
                    let max = adj.upper() - adj.page_size();
                    let f = if max > 0.0 { (adj.value() / max).clamp(0.0, 1.0) } else { 0.0 };
                    let ay = adj.value() + f * adj.page_size();
                    let (ai, _) = w.view.line_at_y(ay as i32);
                    let (ly, lh) = w.view.line_yrange(&ai);
                    let aline = ai.line() as f64 + 1.0 + ((ay - ly as f64) / lh.max(1) as f64).clamp(0.0, 1.0);
                    let (top, _) = w.view.line_at_y(adj.value() as i32);
                    let sel = w.buffer.selection_bounds();
                    eprintln!(
                        "mdreader: editor value={} upper={} page={} top_line={} f={f:.3} anchor={aline:.2} count={} pos={:?}",
                        adj.value(), adj.upper(), adj.page_size(), top.line() + 1,
                        w.search_ctx.occurrences_count(),
                        sel.map(|(a, b)| w.search_ctx.occurrence_position(&a, &b))
                    );
                    let js = js.replace("ANCHOR", &format!("{aline}")).replace("FRAC", &format!("{f}"));
                    w.web.evaluate_javascript(&js, None, None, None::<&gio::Cancellable>, |r| match r {
                        Ok(v) => eprintln!("mdreader: eval = {}", v.to_str()),
                        Err(e) => eprintln!("mdreader: eval error: {e}"),
                    });
                }
            });
            group.add_action(&a);
            let a = gio::SimpleAction::new("debug-editor-line", Some(glib::VariantTy::INT32));
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, v| {
                if let (Some(w), Some(n)) = (w.upgrade(), v.and_then(|v| v.get::<i32>())) {
                    if let Some(iter) = w.buffer.iter_at_line(n - 1) {
                        let (y, _) = w.view.line_yrange(&iter);
                        w.editor_scroll.vadjustment().set_value(y as f64);
                    }
                }
            });
            group.add_action(&a);
            let a = gio::SimpleAction::new("debug-close-search", None);
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(w) = w.upgrade() {
                    w.search_bar.set_search_mode(false);
                    let sel = w.buffer.selection_bounds().map(|(a, b)| w.buffer.text(&a, &b, true).to_string());
                    eprintln!("mdreader: selection after closing search = {sel:?}, editor focused = {}", w.view.has_focus());
                }
            });
            group.add_action(&a);
            let a = gio::SimpleAction::new("debug-search", Some(glib::VariantTy::STRING));
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, v| {
                if let (Some(w), Some(t)) = (w.upgrade(), v.and_then(|v| v.get::<String>())) {
                    w.search_bar.set_search_mode(true);
                    w.search_entry.set_text(&t);
                }
            });
            group.add_action(&a);
        }

    }

    fn action_state(&self, name: &str, on: bool) {
        if let Some(a) = self.window.lookup_action(name) {
            a.downcast::<gio::SimpleAction>().unwrap().set_state(&on.to_variant());
        }
    }

    fn setup_signals(self: &Rc<Self>, prev_btn: &gtk::Button, next_btn: &gtk::Button) {
        let w = Rc::downgrade(self);
        self.buffer.connect_changed(move |_| {
            if let Some(w) = w.upgrade() {
                if !w.loading.get() {
                    w.schedule_render();
                }
            }
        });
        let w = Rc::downgrade(self);
        self.buffer.connect_modified_changed(move |_| {
            if let Some(w) = w.upgrade() {
                w.update_title();
            }
        });

        // Keep the preview roughly in step with the editor while editing.
        let w = Rc::downgrade(self);
        self.editor_scroll.vadjustment().connect_value_changed(move |_| {
            if let Some(w) = w.upgrade() {
                w.sync_preview_scroll();
            }
        });

        let w = Rc::downgrade(self);
        self.web.connect_load_changed(move |_, ev| {
            let Some(w) = w.upgrade() else { return };
            if ev == webkit6::LoadEvent::Finished {
                w.page_ready.set(true);
                w.katex_loaded.set(false);
                w.mermaid_loaded.set(false);
                w.run_js(assets::RUNTIME_JS);
                w.finish_content();
                if w.search_bar.is_search_mode() {
                    w.search_changed();
                }
                if let Some(anchor) = w.pending_anchor.take() {
                    w.scroll_preview_to(&anchor);
                } else {
                    w.sync_preview_scroll();
                }
            }
        });

        if let Some(ucm) = self.web.user_content_manager() {
            let w = Rc::downgrade(self);
            ucm.connect_script_message_received(Some("mdr"), move |_, value| {
                let Some(w) = w.upgrade() else { return };
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&value.to_str()) else { return };
                if msg["type"] == "scroll" {
                    let line = msg["line"].as_f64().unwrap_or(1.0);
                    w.follow_preview_scroll(line, msg["f"].as_f64().unwrap_or(0.0));
                }
            });
        }

        // The split is kept as a ratio: re-applied when the pane area changes
        // size (window resize, sidebar toggle), and updated only when the
        // divider moves for any other reason, i.e. the user dragging it.
        let w = Rc::downgrade(self);
        self.paned.connect_position_notify(move |p| {
            let Some(w) = w.upgrade() else { return };
            let width = p.width();
            if w.editing.get() && width > 0 && !w.applying_split.get() {
                w.split_ratio.set((p.position() as f64 / width as f64).clamp(0.1, 0.9));
            }
        });
        let w = Rc::downgrade(self);
        self.paned.connect_max_position_notify(move |_| {
            if let Some(w) = w.upgrade() {
                w.apply_split();
            }
        });

        let w = Rc::downgrade(self);
        self.web.connect_decide_policy(move |_, decision, kind| {
            let Some(w) = w.upgrade() else { return false };
            w.decide_policy(decision, kind)
        });

        // The preview is read-only output: no "Reload", "Back", "Inspect".
        self.web.connect_context_menu(|_, menu, _| {
            use webkit6::ContextMenuAction as A;
            for item in menu.items() {
                let keep = matches!(
                    item.stock_action(),
                    A::Copy | A::CopyLinkToClipboard | A::CopyImageToClipboard | A::SelectAll
                );
                if !keep {
                    menu.remove(&item);
                }
            }
            menu.n_items() == 0
        });

        let w = Rc::downgrade(self);
        self.toc_list.connect_row_activated(move |_, row| {
            let Some(w) = w.upgrade() else { return };
            let Some(h) = w.toc.borrow().get(row.index() as usize).cloned() else { return };
            w.scroll_preview_to(&h.anchor);
            if w.editing.get() {
                if let Some(mut it) = w.buffer.iter_at_line(h.line.saturating_sub(1) as i32) {
                    w.buffer.place_cursor(&it);
                    w.view.scroll_to_iter(&mut it, 0.0, true, 0.0, 0.1);
                }
            }
            if w.split.is_collapsed() {
                w.split.set_show_sidebar(false);
            }
        });
        let w = Rc::downgrade(self);
        self.split.connect_show_sidebar_notify(move |s| {
            if let Some(w) = w.upgrade() {
                w.action_state("toc", s.shows_sidebar());
            }
        });

        // ---- search ----
        // Reading: WebKit's find bar behaviour in the preview.
        // Editing: matches are stepped through in the editor (so you land on
        // the text to change) and highlighted in the preview, which follows
        // the editor through scroll sync.
        let w = Rc::downgrade(self);
        self.search_entry.connect_search_changed(move |_| {
            if let Some(w) = w.upgrade() {
                w.search_changed();
            }
        });
        for (signal_forward, widget) in [(true, next_btn), (false, prev_btn)] {
            let w = Rc::downgrade(self);
            widget.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    w.search_step(signal_forward);
                }
            });
        }
        let w = Rc::downgrade(self);
        self.search_entry.connect_activate(move |_| {
            if let Some(w) = w.upgrade() {
                w.search_step(true);
            }
        });
        let w = Rc::downgrade(self);
        self.search_entry.connect_next_match(move |_| {
            if let Some(w) = w.upgrade() {
                w.search_step(true);
            }
        });
        let w = Rc::downgrade(self);
        self.search_entry.connect_previous_match(move |_| {
            if let Some(w) = w.upgrade() {
                w.search_step(false);
            }
        });
        let shift_enter = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        shift_enter.connect_key_pressed(move |_, key, _, mods| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) && mods.contains(gdk::ModifierType::SHIFT_MASK) {
                if let Some(w) = w.upgrade() {
                    w.search_step(false);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.search_entry.add_controller(shift_enter);
        let w = Rc::downgrade(self);
        self.search_bar.connect_search_mode_enabled_notify(move |bar| {
            let Some(w) = w.upgrade() else { return };
            if !bar.is_search_mode() {
                w.clear_search();
                if w.editing.get() {
                    // Escape drops you into the editor at the match.
                    w.view.grab_focus();
                }
            } else if !w.search_entry.text().is_empty() {
                w.search_changed();
            }
        });
        let w = Rc::downgrade(self);
        self.search_ctx.connect_occurrences_count_notify(move |_| {
            if let Some(w) = w.upgrade() {
                if w.editing.get() {
                    w.update_editor_match_label();
                }
            }
        });
        let find = self.web.find_controller().expect("webview has a find controller");
        let w = Rc::downgrade(self);
        find.connect_counted_matches(move |_, n| {
            if let Some(w) = w.upgrade() {
                if !w.editing.get() {
                    w.match_label.set_label(&match n {
                        1 => "1 match".to_string(),
                        n => format!("{n} matches"),
                    });
                }
            }
        });
        let w = Rc::downgrade(self);
        find.connect_failed_to_find_text(move |_| {
            if let Some(w) = w.upgrade() {
                if !w.editing.get() {
                    w.match_label.set_label("No matches");
                    w.search_entry.add_css_class("error");
                }
            }
        });

        // ---- banner: file changed on disk while we have unsaved edits ----
        let w = Rc::downgrade(self);
        self.banner.connect_button_clicked(move |b| {
            b.set_revealed(false);
            if let Some(w) = w.upgrade() {
                if let Some(f) = w.file.borrow().clone() {
                    w.load_file(&f, false);
                }
            }
        });

        // ---- drag and drop ----
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let w = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            let (Some(w), Ok(list)) = (w.upgrade(), value.get::<gdk::FileList>()) else {
                return false;
            };
            for f in list.files() {
                w.open_from_here(&f, None);
            }
            true
        });
        self.window.add_controller(drop);

        // ---- close with unsaved changes ----
        let w = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            let Some(w) = w.upgrade() else { return glib::Propagation::Proceed };
            if w.force_close.get() || !w.buffer.is_modified() {
                w.save_state();
                return glib::Propagation::Proceed;
            }
            w.confirm_close();
            glib::Propagation::Stop
        });
        let w = Rc::downgrade(self);
        self.window.connect_destroy(move |_| {
            if let Some(w) = w.upgrade() {
                if let Some(m) = w.monitor.take() {
                    m.cancel();
                }
                WINDOWS.with_borrow_mut(|ws| ws.retain(|x| !Rc::ptr_eq(x, &w)));
            }
        });
    }

    fn follow_style_scheme(self: &Rc<Self>) {
        let sm = adw::StyleManager::default();
        let apply = |buffer: &sourceview5::Buffer, dark: bool| {
            let id = if dark { "Adwaita-dark" } else { "Adwaita" };
            if let Some(s) = sourceview5::StyleSchemeManager::default().scheme(id) {
                buffer.set_style_scheme(Some(&s));
            }
        };
        apply(&self.buffer, sm.is_dark());
        let w = Rc::downgrade(self);
        sm.connect_dark_notify(move |sm| {
            let Some(w) = w.upgrade() else { return };
            apply(&w.buffer, sm.is_dark());
            // Reload the page so Mermaid redraws diagrams in the new theme.
            w.page_ready.set(false);
            w.render_now();
        });
    }

    fn save_state(&self) {
        let mut s = state::load();
        let (w, h) = self.window.default_size();
        if w > 0 && h > 0 {
            s.width = w;
            s.height = h;
        }
        s.maximized = self.window.is_maximized();
        s.zoom = self.web.zoom_level();
        s.sidebar = self.sidebar_pref.get().or(s.sidebar);
        s.split = self.split_ratio.get();
        state::save(&s);
    }

    // ------------------------------------------------------------ rendering

    fn base_uri(&self) -> String {
        match self.path().and_then(|p| p.parent().map(Path::to_path_buf)) {
            Some(dir) => {
                let mut uri = gio::File::for_path(dir).uri().to_string();
                if !uri.ends_with('/') {
                    uri.push('/');
                }
                uri
            }
            None => "about:blank".to_string(),
        }
    }

    fn schedule_render(self: &Rc<Self>) {
        if let Some(id) = self.render_timer.take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(RENDER_DELAY, move || {
            if let Some(w) = w.upgrade() {
                w.render_timer.take();
                w.render_now();
            }
        });
        self.render_timer.replace(Some(id));
    }

    fn render_now(&self) {
        let text = self.text();
        let r = render::render(&text);
        self.set_toc(r.toc);
        let body = if text.trim().is_empty() && self.file.borrow().is_none() && !self.editing.get() {
            EMPTY_STATE.to_string()
        } else {
            r.html
        };
        if self.editing.get() && self.search_bar.is_search_mode() && !self.search_entry.text().is_empty() {
            self.mark_all_matches();
        }
        self.has_math.set(r.has_math);
        self.has_mermaid.set(r.has_mermaid);
        let base = self.base_uri();
        if self.page_ready.get() && *self.page_base.borrow() == base {
            self.run_js(&render::update_script(&body));
            self.finish_content();
            // New content can change heights; keep the panes lined up.
            self.sync_preview_scroll();
        } else {
            self.page_ready.set(false);
            self.page_base.replace(base.clone());
            self.web.load_html(&render::page(&body), Some(&base));
        }
    }

    /// Load KaTeX / Mermaid if the document needs them, then let the page
    /// typeset math and draw diagrams.
    fn finish_content(&self) {
        if self.has_math.get() && !self.katex_loaded.get() {
            self.run_js(assets::KATEX_JS);
            self.run_js(assets::katex_css_script());
            self.katex_loaded.set(true);
        }
        if self.has_mermaid.get() && !self.mermaid_loaded.get() {
            self.run_js(assets::MERMAID_JS);
            self.mermaid_loaded.set(true);
        }
        self.run_js("mdr.after();");
    }

    fn run_js(&self, script: &str) {
        // `void 0`: we never want the value back, and WebKit can't convert
        // some (Promises, functions) and reports that as an error.
        let script = format!("{script}\n;void 0;");
        self.web
            .evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |r| {
                if let Err(e) = r {
                    eprintln!("mdreader: preview script failed: {e}");
                }
            });
    }

    fn scroll_preview_to(&self, anchor: &str) {
        if self.page_ready.get() {
            self.run_js(&render::scroll_script(anchor));
        } else {
            self.pending_anchor.replace(Some(anchor.to_string()));
        }
    }

    /// Editor -> preview. See the sync model in runtime.js: the line `f` of
    /// the way down the editor's viewport goes to the same height in the
    /// preview, where `f` is the editor's scroll progress.
    fn sync_preview_scroll(&self) {
        if !self.editing.get() || !self.page_ready.get() {
            return;
        }
        if self.ignore_editor_scroll_until.get().is_some_and(|t| Instant::now() < t) {
            return;
        }
        let adj = self.editor_scroll.vadjustment();
        let max = adj.upper() - adj.page_size();
        let f = if max > 0.0 { (adj.value() / max).clamp(0.0, 1.0) } else { 0.0 };
        let y = adj.value() + f * adj.page_size();
        let (iter, _) = self.view.line_at_y(y as i32);
        let (line_y, line_h) = self.view.line_yrange(&iter);
        let frac = if line_h > 0 { ((y - line_y as f64) / line_h as f64).clamp(0.0, 1.0) } else { 0.0 };
        let line = iter.line() as f64 + 1.0 + frac;
        let total = self.buffer.line_count();
        self.run_js(&format!("mdr.scrollToLine({line}, {f}, {total});"));
    }

    /// Preview -> editor, the same model in reverse.
    fn follow_preview_scroll(&self, line: f64, f: f64) {
        if !self.editing.get() {
            return;
        }
        let adj = self.editor_scroll.vadjustment();
        let max = adj.upper() - adj.page_size();
        let line = line.max(1.0);
        let anchor = match self.buffer.iter_at_line(line.floor() as i32 - 1) {
            Some(iter) => {
                let (ly, lh) = self.view.line_yrange(&iter);
                ly as f64 + line.fract() * lh as f64
            }
            None => adj.upper(),
        };
        let target = (anchor - f.clamp(0.0, 1.0) * adj.page_size()).clamp(0.0, max.max(0.0));
        self.ignore_editor_scroll_until.set(Some(Instant::now() + Duration::from_millis(250)));
        adj.set_value(target);
    }

    // --------------------------------------------------------------- search

    fn search_changed(&self) {
        let text = self.search_entry.text().to_string();
        self.search_entry.remove_css_class("error");
        let find = self.web.find_controller().expect("webview has a find controller");
        if text.is_empty() {
            self.clear_search();
            return;
        }
        let opts = (webkit6::FindOptions::CASE_INSENSITIVE | webkit6::FindOptions::WRAP_AROUND).bits();
        if self.editing.get() {
            find.search_finish();
            self.search_settings.set_search_text(Some(&text));
            self.mark_all_matches();
            self.run_js(&format!("mdr.highlight({});", serde_json::to_string(&text).unwrap()));
            // Incremental: stay on the current match while the word grows.
            let (start, _) = self.current_match_bounds();
            self.select_editor_match(self.search_ctx.forward(&start));
        } else {
            self.search_settings.set_search_text(None);
            self.run_js("mdr.highlight('');");
            find.count_matches(&text, opts, 10_000);
            find.search(&text, opts, 10_000);
        }
    }

    fn search_step(&self, forward: bool) {
        if self.search_entry.text().is_empty() {
            return;
        }
        if self.editing.get() {
            let (start, end) = self.current_match_bounds();
            let found = if forward { self.search_ctx.forward(&end) } else { self.search_ctx.backward(&start) };
            self.select_editor_match(found);
        } else {
            let find = self.web.find_controller().expect("webview has a find controller");
            if forward {
                find.search_next();
            } else {
                find.search_previous();
            }
        }
    }

    /// The current match, or an empty range at the cursor when there is none.
    fn current_match_bounds(&self) -> (gtk::TextIter, gtk::TextIter) {
        let marks = (self.buffer.mark("match-start"), self.buffer.mark("match-end"));
        match marks {
            (Some(s), Some(e)) => (self.buffer.iter_at_mark(&s), self.buffer.iter_at_mark(&e)),
            _ => {
                let c = self.buffer.iter_at_mark(&self.buffer.get_insert());
                (c, c)
            }
        }
    }

    /// Tag every occurrence in the editor (bounded, for huge documents).
    fn mark_all_matches(&self) {
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_tag(&self.match_tag, &start, &end);
        if self.search_settings.search_text().is_none_or(|t| t.is_empty()) {
            return;
        }
        let mut from = start;
        for _ in 0..10_000 {
            match self.search_ctx.forward(&from) {
                Some((s, e, false)) if e.offset() > s.offset() => {
                    self.buffer.apply_tag(&self.match_tag, &s, &e);
                    from = e;
                }
                _ => break,
            }
        }
    }

    fn unmark_current_match(&self) {
        let (s, e) = self.buffer.bounds();
        self.buffer.remove_tag(&self.current_match_tag, &s, &e);
        for name in ["match-start", "match-end"] {
            if let Some(m) = self.buffer.mark(name) {
                self.buffer.delete_mark(&m);
            }
        }
    }

    fn select_editor_match(&self, found: Option<(gtk::TextIter, gtk::TextIter, bool)>) {
        self.unmark_current_match();
        match found {
            Some((mut s, e, _wrapped)) => {
                self.buffer.create_mark(Some("match-start"), &s, true);
                self.buffer.create_mark(Some("match-end"), &e, false);
                self.buffer.apply_tag(&self.current_match_tag, &s, &e);
                // Above the search context's own highlight tag.
                self.current_match_tag.set_priority(self.buffer.tag_table().size() - 1);
                self.buffer.place_cursor(&s);
                self.view.scroll_to_iter(&mut s, 0.1, true, 0.0, 0.3);
                self.search_entry.remove_css_class("error");
            }
            None => self.search_entry.add_css_class("error"),
        }
        self.update_editor_match_label();
    }

    fn update_editor_match_label(&self) {
        let total = self.search_ctx.occurrences_count();
        let (s, e) = self.current_match_bounds();
        let pos = if s == e { 0 } else { self.search_ctx.occurrence_position(&s, &e) };
        let label = match total {
            -1 => String::new(), // still counting
            0 => "No matches".to_string(),
            n if pos > 0 => format!("{pos} of {n}"),
            1 => "1 match".to_string(),
            n => format!("{n} matches"),
        };
        if total == 0 {
            self.search_entry.add_css_class("error");
        }
        self.match_label.set_label(&label);
    }

    fn clear_search(&self) {
        let (s, e) = self.current_match_bounds();
        self.unmark_current_match();
        if self.editing.get() && s != e {
            // Leave the match selected, ready to type over.
            self.buffer.select_range(&s, &e);
        }
        if let Some(find) = self.web.find_controller() {
            find.search_finish();
        }
        self.search_settings.set_search_text(None);
        self.mark_all_matches();
        self.run_js("mdr.highlight('');");
        self.match_label.set_label("");
        self.search_entry.remove_css_class("error");
    }

    fn set_toc(&self, toc: Vec<Heading>) {
        if *self.toc.borrow() == toc {
            return;
        }
        while let Some(row) = self.toc_list.first_child() {
            self.toc_list.remove(&row);
        }
        let min = toc.iter().map(|h| h.level).min().unwrap_or(1);
        for h in &toc {
            let label = gtk::Label::builder()
                .label(&h.text)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .tooltip_text(&h.text)
                .margin_start(12 * (h.level - min) as i32)
                .build();
            if h.level == min {
                label.add_css_class("heading");
            }
            self.toc_list.append(&label);
        }
        let first_toc = self.toc.borrow().is_empty();
        self.toc.replace(toc);
        // Follow the user's last choice; without one, offer the outline for
        // documents with several headings.
        if first_toc && !self.split.is_collapsed() {
            let n = self.toc.borrow().len();
            let show = match self.sidebar_pref.get() {
                Some(pref) => pref && n > 0,
                None => n >= 3,
            };
            self.split.set_show_sidebar(show);
        }
    }

    // ---------------------------------------------------------------- files

    fn update_title(&self) {
        let modified = self.buffer.is_modified();
        let name = match self.path() {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            None if modified || self.editing.get() => "Untitled".to_string(),
            None => APP_NAME.to_string(),
        };
        let dir = self
            .path()
            .and_then(|p| p.parent().map(tilde))
            .unwrap_or_default();
        let shown = if modified { format!("• {name}") } else { name.clone() };
        self.title.set_title(&shown);
        self.title.set_subtitle(&dir);
        self.window.set_title(Some(&if name == APP_NAME { name } else { format!("{shown} — {APP_NAME}") }));
        self.save_btn.set_visible(modified || self.editing.get());
        self.save_btn.set_sensitive(modified || self.file.borrow().is_none());
    }

    /// Replace the window's document with `file`. `record` pushes the
    /// current document onto the Back history.
    fn load_file(self: &Rc<Self>, file: &gio::File, record: bool) -> bool {
        let bytes = match file.load_contents(None::<&gio::Cancellable>) {
            Ok((b, _)) => b,
            Err(e) => {
                self.toast(&format!("Could not open {}: {}", display_name(file), e.message()));
                return false;
            }
        };
        let text = match String::from_utf8(bytes.to_vec()) {
            Ok(t) => t,
            Err(_) => {
                self.toast("This file is not valid UTF-8; some characters were replaced");
                String::from_utf8_lossy(&bytes).into_owned()
            }
        };
        if record {
            if let Some(p) = self.path() {
                if Some(&p) != file.path().as_ref() {
                    self.history.borrow_mut().push(p);
                }
            }
        }
        self.back_btn.set_visible(!self.history.borrow().is_empty());
        let same_file = self.path() == file.path();
        self.file.replace(Some(file.clone()));
        self.set_buffer_text(&text, same_file);
        self.disk_text.replace(text);
        self.banner.set_revealed(false);
        self.watch_file();
        self.update_title();
        if !same_file {
            // Force a full page load so relative images resolve against the new directory.
            self.page_ready.set(false);
        }
        self.render_now();
        if let Some(p) = file.path() {
            gtk::RecentManager::default().add_item(&gio::File::for_path(p).uri());
        }
        true
    }

    fn set_buffer_text(&self, text: &str, keep_cursor: bool) {
        let offset = self.buffer.cursor_position();
        self.loading.set(true);
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(text);
        self.buffer.end_irreversible_action();
        self.buffer.set_modified(false);
        self.loading.set(false);
        let it = if keep_cursor {
            self.buffer.iter_at_offset(offset)
        } else {
            self.buffer.start_iter()
        };
        self.buffer.place_cursor(&it);
    }

    fn watch_file(self: &Rc<Self>) {
        if let Some(m) = self.monitor.take() {
            m.cancel();
        }
        let Some(file) = self.file.borrow().clone() else { return };
        let m = match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("mdreader: cannot watch {}: {e}", display_name(&file));
                return;
            }
        };
        let w = Rc::downgrade(self);
        m.connect_changed(move |_, _, _, event| {
            use gio::FileMonitorEvent as E;
            if !matches!(
                event,
                E::Changed | E::ChangesDoneHint | E::Created | E::MovedIn | E::Renamed | E::Moved
            ) {
                return;
            }
            let Some(w) = w.upgrade() else { return };
            if let Some(id) = w.disk_timer.take() {
                id.remove();
            }
            let w2 = Rc::downgrade(&w);
            let id = glib::timeout_add_local_once(DISK_CHECK_DELAY, move || {
                if let Some(w) = w2.upgrade() {
                    w.disk_timer.take();
                    w.check_disk();
                }
            });
            w.disk_timer.replace(Some(id));
        });
        self.monitor.replace(Some(m));
    }

    /// The file changed on disk: follow it, unless the user has unsaved edits.
    fn check_disk(self: &Rc<Self>) {
        let Some(file) = self.file.borrow().clone() else { return };
        let Ok((bytes, _)) = file.load_contents(None::<&gio::Cancellable>) else { return };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if text == *self.disk_text.borrow() {
            return;
        }
        if self.buffer.is_modified() {
            self.banner.set_revealed(true);
            return;
        }
        self.disk_text.replace(text.clone());
        self.set_buffer_text(&text, true);
        self.render_now();
    }

    /// Open `file` in this window if that loses nothing, else in another.
    fn open_from_here(self: &Rc<Self>, file: &gio::File, fragment: Option<String>) {
        let target = if self.buffer.is_modified() && self.path() != file.path() {
            open(&self.app, file);
            all().into_iter().find(|w| w.path() == file.path())
        } else {
            if self.path() != file.path() && !self.load_file(file, true) {
                return;
            }
            Some(self.clone())
        };
        if let (Some(w), Some(f)) = (target, fragment) {
            w.scroll_preview_to(&f);
        }
    }

    fn go_back(self: &Rc<Self>) {
        if self.buffer.is_modified() {
            self.toast("Save or undo your changes before going back");
            return;
        }
        let Some(prev) = self.history.borrow_mut().pop() else { return };
        self.load_file(&gio::File::for_path(prev), false);
    }

    fn choose_file(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Open Markdown File")
            .filters(&file_filters())
            .modal(true)
            .build();
        if let Some(dir) = self.path().and_then(|p| p.parent().map(Path::to_path_buf)) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let w = self.clone();
        dialog.open(Some(&self.window), None::<&gio::Cancellable>, move |res| {
            if let Ok(f) = res {
                if w.is_blank() {
                    w.load_file(&f, true);
                } else {
                    open(&w.app, &f);
                }
            }
        });
    }

    async fn save(self: &Rc<Self>) -> bool {
        let Some(file) = self.file.borrow().clone() else {
            return self.save_as().await;
        };
        self.write_to(&file).await
    }

    async fn save_as(self: &Rc<Self>) -> bool {
        let dialog = gtk::FileDialog::builder()
            .title("Save Markdown File")
            .filters(&file_filters())
            .modal(true)
            .build();
        match self.path() {
            Some(p) => {
                if let Some(dir) = p.parent() {
                    dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
                }
                dialog.set_initial_name(p.file_name().and_then(|n| n.to_str()));
            }
            None => dialog.set_initial_name(Some("Untitled.md")),
        }
        let Ok(file) = dialog.save_future(Some(&self.window)).await else { return false };
        if !self.write_to(&file).await {
            return false;
        }
        let moved = self.path() != file.path();
        self.file.replace(Some(file));
        self.watch_file();
        self.update_title();
        if moved {
            self.page_ready.set(false);
            self.render_now();
        }
        true
    }

    async fn write_to(self: &Rc<Self>, file: &gio::File) -> bool {
        let text = self.text();
        // Set first so the monitor event from our own write is ignored.
        let previous = self.disk_text.replace(text.clone());
        let res = file
            .replace_contents_future(text.into_bytes(), None, false, gio::FileCreateFlags::NONE)
            .await;
        match res {
            Ok(_) => {
                // Only clear "modified" if nothing was typed during the write.
                if self.text() == *self.disk_text.borrow() {
                    self.buffer.set_modified(false);
                }
                self.banner.set_revealed(false);
                self.update_title();
                true
            }
            Err((_, e)) => {
                self.disk_text.replace(previous);
                self.toast(&format!("Could not save {}: {}", display_name(file), e.message()));
                false
            }
        }
    }

    fn confirm_close(self: &Rc<Self>) {
        let name = self
            .path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Untitled".into());
        let dialog = adw::AlertDialog::new(
            Some("Save Changes?"),
            Some(&format!("“{name}” has unsaved changes. Changes you don't save are lost.")),
        );
        dialog.add_responses(&[("cancel", "_Cancel"), ("discard", "_Discard"), ("save", "_Save")]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let w = self.clone();
        glib::spawn_future_local(async move {
            match dialog.choose_future(Some(&w.window)).await.as_str() {
                "discard" => {
                    w.force_close.set(true);
                    w.window.close();
                }
                "save" => {
                    if w.save().await {
                        w.window.close();
                    }
                }
                _ => {}
            }
        });
    }

    // -------------------------------------------------- print and export

    /// Switch the page to its light palette (and redraw diagrams light), for
    /// paper and exported files. Resolves once the page has re-rendered.
    async fn force_light(&self, on: bool) {
        let (tx, rx) = futures_channel::oneshot::channel();
        self.web.call_async_javascript_function(
            &format!("await mdr.setForceLight({on}); return true;"),
            None,
            None,
            None,
            None::<&gio::Cancellable>,
            move |r| {
                if let Err(e) = &r {
                    eprintln!("mdreader: setForceLight failed: {e}");
                }
                let _ = tx.send(());
            },
        );
        let _ = rx.await;
    }

    fn print(self: &Rc<Self>) {
        if !self.page_ready.get() {
            return;
        }
        let w = self.clone();
        glib::spawn_future_local(async move {
            w.force_light(true).await;
            let op = webkit6::PrintOperation::new(&w.web);
            w.watch_print(&op, None);
            if op.run_dialog(Some(&w.window)) == webkit6::PrintOperationResponse::Cancel {
                w.print_op.take();
                w.force_light(false).await;
            }
        });
    }

    async fn export_pdf(self: &Rc<Self>) {
        if !self.page_ready.get() {
            return;
        }
        let Some(file) = self.choose_export_file("Export as PDF", "pdf", "PDF documents", "application/pdf").await
        else {
            return;
        };
        self.export_pdf_to(file).await;
    }

    async fn export_pdf_to(self: &Rc<Self>, file: gio::File) {
        self.force_light(true).await;
        let settings = gtk::PrintSettings::new();
        settings.set_printer("Print to File");
        settings.set(gtk::PRINT_SETTINGS_OUTPUT_FILE_FORMAT, Some("pdf"));
        settings.set(gtk::PRINT_SETTINGS_OUTPUT_URI, Some(&file.uri()));
        let setup = gtk::PageSetup::new();
        setup.set_paper_size(&gtk::PaperSize::new(None)); // the locale's default paper
        setup.set_top_margin(15.0, gtk::Unit::Mm);
        setup.set_bottom_margin(15.0, gtk::Unit::Mm);
        setup.set_left_margin(15.0, gtk::Unit::Mm);
        setup.set_right_margin(15.0, gtk::Unit::Mm);
        let op = webkit6::PrintOperation::new(&self.web);
        op.set_print_settings(&settings);
        op.set_page_setup(&setup);
        self.watch_print(&op, Some(file));
        op.print();
    }

    /// Keep `op` alive until it ends, then restore the theme and report.
    fn watch_print(self: &Rc<Self>, op: &webkit6::PrintOperation, exported: Option<gio::File>) {
        let w = Rc::downgrade(self);
        op.connect_finished(move |_| {
            let Some(w) = w.upgrade() else { return };
            w.print_op.take();
            if let Some(f) = &exported {
                w.toast(&format!("Exported {}", display_name(f)));
            }
            let w2 = w.clone();
            glib::spawn_future_local(async move { w2.force_light(false).await });
        });
        let w = Rc::downgrade(self);
        op.connect_failed(move |_, e| {
            if let Some(w) = w.upgrade() {
                w.toast(&format!("Printing failed: {}", e.message()));
            }
        });
        self.print_op.replace(Some(op.clone()));
    }

    async fn export_html(self: &Rc<Self>) {
        if !self.page_ready.get() {
            return;
        }
        let Some(file) = self.choose_export_file("Export as HTML", "html", "HTML pages", "text/html").await else {
            return;
        };
        self.export_html_to(file).await;
    }

    async fn export_html_to(self: &Rc<Self>, file: gio::File) {
        self.force_light(true).await;
        let (tx, rx) = futures_channel::oneshot::channel();
        self.web.call_async_javascript_function(
            "return await mdr.exportBody();",
            None,
            None,
            None,
            None::<&gio::Cancellable>,
            move |r| {
                let _ = tx.send(r.map(|v| v.to_str().to_string()));
            },
        );
        let body = rx.await;
        self.force_light(false).await;
        let body = match body {
            Ok(Ok(b)) => b,
            Ok(Err(e)) => return self.toast(&format!("Export failed: {}", e.message())),
            Err(_) => return,
        };
        let doc_dir = self.path().and_then(|p| p.parent().map(Path::to_path_buf));
        let title = export::title(&self.toc.borrow(), self.path().as_deref());
        let katex = self.has_math.get().then_some(assets::KATEX_CSS);
        let page = export::standalone_page(&title, &body, katex, doc_dir.as_deref());
        match file
            .replace_contents_future(page.into_bytes(), None, false, gio::FileCreateFlags::NONE)
            .await
        {
            Ok(_) => self.toast(&format!("Exported {}", display_name(&file))),
            Err((_, e)) => self.toast(&format!("Export failed: {}", e.message())),
        }
    }

    async fn choose_export_file(&self, title: &str, ext: &str, filter_name: &str, mime: &str) -> Option<gio::File> {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(filter_name));
        filter.add_mime_type(mime);
        filter.add_suffix(ext);
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title(title).filters(&filters).modal(true).build();
        let stem = self
            .path()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Untitled".into());
        dialog.set_initial_name(Some(&format!("{stem}.{ext}")));
        if let Some(dir) = self.path().and_then(|p| p.parent().map(Path::to_path_buf)) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        dialog.save_future(Some(&self.window)).await.ok()
    }

    // ---------------------------------------------------------------- modes

    fn apply_split(&self) {
        if !self.editing.get() {
            return;
        }
        let width = self.paned.width();
        let width = if width > 0 { width } else { 1000 };
        self.applying_split.set(true);
        self.paned.set_position((width as f64 * self.split_ratio.get()).round() as i32);
        self.applying_split.set(false);
    }

    pub fn set_editing(self: &Rc<Self>, on: bool) {
        if self.editing.get() == on {
            return;
        }
        self.editing.set(on);
        self.action_state("edit", on);
        self.editor_scroll.set_visible(on);
        if on {
            self.apply_split();
            self.view.grab_focus();
        }
        self.update_title();
        self.render_now();
        if self.search_bar.is_search_mode() {
            self.search_changed();
        }
    }

    // -------------------------------------------------------------- links

    fn decide_policy(self: &Rc<Self>, decision: &webkit6::PolicyDecision, kind: webkit6::PolicyDecisionType) -> bool {
        use webkit6::PolicyDecisionType as T;
        if kind == T::Response {
            return false;
        }
        let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else {
            return false;
        };
        let Some(action) = nav.navigation_action() else { return false };
        let uri = action.request().and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
        // Our own load_html() calls are the only non-click navigations we allow.
        if kind == T::NavigationAction && action.navigation_type() != webkit6::NavigationType::LinkClicked {
            let ours = uri == *self.page_base.borrow() || uri == "about:blank";
            if ours {
                return false;
            }
            decision.ignore();
            return true;
        }
        let act = links::classify(&uri, &self.page_base.borrow());
        if act == LinkAction::Allow {
            return false;
        }
        decision.ignore();
        match act {
            LinkAction::OpenMarkdown { path, fragment } => {
                if Some(&path) == self.path().as_ref() {
                    if let Some(f) = fragment {
                        self.scroll_preview_to(&f);
                    }
                } else if !path.exists() {
                    self.toast(&format!("{} does not exist", tilde(&path)));
                } else {
                    self.open_from_here(&gio::File::for_path(&path), fragment);
                }
            }
            LinkAction::OpenFile(path) => {
                if !path.exists() {
                    self.toast(&format!("{} does not exist", tilde(&path)));
                } else {
                    gtk::FileLauncher::new(Some(&gio::File::for_path(path))).launch(
                        Some(&self.window),
                        None::<&gio::Cancellable>,
                        |_| {},
                    );
                }
            }
            LinkAction::External(uri) => {
                gtk::UriLauncher::new(&uri).launch(Some(&self.window), None::<&gio::Cancellable>, |_| {});
            }
            LinkAction::Allow | LinkAction::Ignore => {}
        }
        true
    }

    // -------------------------------------------------------------- testing

    async fn debug_export(self: &Rc<Self>, path: &str, pdf: bool) {
        let file = gio::File::for_path(path);
        if pdf {
            self.export_pdf_to(file).await;
        } else {
            self.export_html_to(file).await;
        }
    }

    /// Render the whole window (including the WebView) to a PNG on the next
    /// frame, so there is always freshly drawn content to capture.
    fn snapshot_to_png(&self, path: &str) {
        let path = path.to_string();
        self.window.queue_draw();
        self.window.add_tick_callback(move |window, _| {
            let paintable = gtk::WidgetPaintable::new(Some(window));
            let snap = gtk::Snapshot::new();
            paintable.snapshot(&snap, window.width() as f64, window.height() as f64);
            let Some(node) = snap.to_node() else {
                return glib::ControlFlow::Continue; // try the next frame
            };
            let Some(renderer) = window.renderer() else { return glib::ControlFlow::Break };
            match renderer.render_texture(node, None).save_to_png(&path) {
                Ok(()) => eprintln!("mdreader: snapshot saved to {path}"),
                Err(e) => eprintln!("mdreader: snapshot failed: {e}"),
            }
            glib::ControlFlow::Break
        });
    }
}

fn file_filters() -> gio::ListStore {
    let md = gtk::FileFilter::new();
    md.set_name(Some("Markdown"));
    md.add_mime_type("text/markdown");
    md.add_mime_type("text/x-markdown");
    for ext in ["md", "markdown", "mdown", "mkd", "mkdn", "mdx"] {
        md.add_suffix(ext);
    }
    let text = gtk::FileFilter::new();
    text.set_name(Some("Text files"));
    text.add_mime_type("text/*");
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    let store = gio::ListStore::new::<gtk::FileFilter>();
    store.append(&md);
    store.append(&text);
    store.append(&all);
    store
}

fn display_name(f: &gio::File) -> String {
    f.path().map(|p| tilde(&p)).unwrap_or_else(|| f.uri().to_string())
}

fn tilde(p: &Path) -> String {
    let home = glib::home_dir();
    match p.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}
