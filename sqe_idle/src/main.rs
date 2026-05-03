use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Button, Box as GtkBox, TextView, ScrolledWindow, Orientation,
    Paned, WrapMode, FileChooserAction, ResponseType, MessageDialog, ButtonsType, MessageType,
    DialogFlags, FileChooserDialog, Revealer, CssProvider, StyleContext, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use webkit6::WebView;
use webkit6::prelude::WebViewExt; // for load_uri() and settings()
use std::process::Command;
use std::fs;
use std::path::{PathBuf, Path};
use std::time::{SystemTime, UNIX_EPOCH};
use std::thread;
use std::rc::Rc;
use std::cell::Cell;
use glib;

fn main() {
    let app = Application::builder()
        .application_id("com.my8oss.sqe_idle")
        .build();

    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("SQE IDLE — editor + preview")
        .default_width(1100)
        .default_height(750)
        .build();

    // --- CSS: set background color to #33101c ---
    let provider = CssProvider::new();
    let css = r#"
    .root-bg {
        background-color: #33101c;
    }
    /* Also try to make the ApplicationWindow itself use the color */
    applicationwindow, window {
        background-color: #33101c;
    }
    "#;
    // load_from_data takes &str and returns (), no expect().
    provider.load_from_data(css);

    // Root layout
    let outer = GtkBox::new(Orientation::Vertical, 6);
    outer.set_margin_end(6);
    outer.add_css_class("root-bg");

    // Attach provider to this widget's style context so the CSS applies.
    outer.style_context().add_provider(&provider, STYLE_PROVIDER_PRIORITY_APPLICATION);

    // Toolbar
    let toolbar = GtkBox::new(Orientation::Horizontal, 6);
    let new_button = Button::with_label("New");
    let open_button = Button::with_label("Open");
    let save_button = Button::with_label("Save");
    let run_button = Button::with_label("Run");
    let live_button = Button::with_label("Live: OFF");
    let info_button = Button::with_label("Info");
    let toggle_term_button = Button::with_label("Terminal");

    for b in [
        &new_button,
        &open_button,
        &save_button,
        &run_button,
        &live_button,
        &info_button,
        &toggle_term_button,
    ] {
        toolbar.append(b);
    }
    outer.append(&toolbar);

    // Editor | Preview (horizontal paned)
    let paned_horizontal = Paned::new(Orientation::Horizontal);

    // Left: editor
    let editor_box = GtkBox::new(Orientation::Vertical, 6);
    let text_view = TextView::new();
    text_view.set_wrap_mode(WrapMode::Word);
    text_view.set_monospace(true);
    let scrolled_editor = ScrolledWindow::builder()
        .child(&text_view)
        .vexpand(true)
        .hexpand(true)
        .build();
    editor_box.append(&scrolled_editor);

    // Right: WebView preview
    let webview = WebView::new();
    if let Some(settings) = WebViewExt::settings(&webview) {
        settings.set_enable_developer_extras(true);
        // Uncomment if you want to force-enable JS:
        // settings.set_enable_javascript(true);
    }
    let scrolled_preview = ScrolledWindow::builder()
        .child(&webview)
        .vexpand(true)
        .hexpand(true)
        .build();

    paned_horizontal.set_start_child(Some(&editor_box));
    paned_horizontal.set_end_child(Some(&scrolled_preview));
    paned_horizontal.set_resize_start_child(true);
    paned_horizontal.set_resize_end_child(true);

    // Terminal (wrapped in a Revealer so it can be toggled)
    let terminal_view = TextView::new();
    terminal_view.set_editable(false);
    terminal_view.set_wrap_mode(WrapMode::WordChar);
    terminal_view.set_monospace(true);
    let scrolled_terminal = ScrolledWindow::builder()
        .child(&terminal_view)
        .vexpand(true)
        .hexpand(true)
        .min_content_height(120) // small default intrinsic height
        .build();

    let terminal_revealer = Revealer::new();
    terminal_revealer.set_child(Some(&scrolled_terminal));
    terminal_revealer.set_reveal_child(true); // start visible

    // Vertical paned: top (editor+preview) | bottom (terminal)
    let paned_vertical = Paned::new(Orientation::Vertical);
    paned_vertical.set_start_child(Some(&paned_horizontal));
    paned_vertical.set_end_child(Some(&terminal_revealer));
    paned_vertical.set_resize_start_child(true);
    paned_vertical.set_resize_end_child(true);
    paned_vertical.set_position(560); // default split (from top, pixels) -> terminal starts smaller

    outer.append(&paned_vertical);
    window.set_child(Some(&outer));

    // Channel: HTML updates -> WebView
    let (sender_html, receiver_html) = glib::MainContext::channel::<Option<String>>(glib::Priority::default());
    let webview_clone = webview.clone();
    receiver_html.attach(None, move |html_path_opt| {
        if let Some(path_str) = html_path_opt {
            let uri = format!("file://{}", path_str);
            webview_clone.load_uri(&uri);
        }
        glib::Continue(true)
    });

    // Channel: terminal lines -> terminal TextView
    let (term_sender, term_receiver) = glib::MainContext::channel::<String>(glib::Priority::default());
    let terminal_clone = terminal_view.clone();
    term_receiver.attach(None, move |line| {
        let buf = terminal_clone.buffer();
        buf.insert(&mut buf.end_iter(), &format!("{}\n", line));
        // Auto-scroll to bottom
        let mark = buf.create_mark(None, &buf.end_iter(), true);
        terminal_clone.scroll_mark_onscreen(&mark);
        glib::Continue(true)
    });

    // Live mode state
    let live_mode = Rc::new(Cell::new(false));

    // Run button
    {
        let tv = text_view.clone();
        let s_html = sender_html.clone();
        let s_term = term_sender.clone();
        run_button.connect_clicked(move |_| {
            let buf = tv.buffer();
            let start = buf.start_iter();
            let end = buf.end_iter();
            let code = buf.text(&start, &end, true).to_string();
            run_sqe_core(code, s_html.clone(), s_term.clone());
        });
    }

    // Live toggle
    {
        let live_mode = live_mode.clone();
        let tv = text_view.clone();
        let s_html = sender_html.clone();
        let s_term = term_sender.clone();
        live_button.connect_clicked(move |btn| {
            let new_state = !live_mode.get();
            live_mode.set(new_state);
            btn.set_label(if new_state { "Live: ON" } else { "Live: OFF" });
            if new_state {
                // Trigger an immediate run on toggle ON
                let buf = tv.buffer();
                let start = buf.start_iter();
                let end = buf.end_iter();
                let code = buf.text(&start, &end, true).to_string();
                run_sqe_core(code, s_html.clone(), s_term.clone());
            }
        });
    }

    // Live: run on each buffer change when enabled
    {
        let live_mode = live_mode.clone();
        let s_html = sender_html.clone();
        let s_term = term_sender.clone();
        text_view.buffer().connect_changed(move |buf| {
            if live_mode.get() {
                let start = buf.start_iter();
                let end = buf.end_iter();
                let code = buf.text(&start, &end, true).to_string();
                run_sqe_core(code, s_html.clone(), s_term.clone());
            }
        });
    }

    // New
    {
        let tv = text_view.clone();
        new_button.connect_clicked(move |_| {
            tv.buffer().set_text("");
        });
    }

    // Open
    {
        let win = window.clone();
        let tv = text_view.clone();
        open_button.connect_clicked(move |_| {
            let dialog = FileChooserDialog::new(
                Some("Open file"),
                Some(&win),
                FileChooserAction::Open,
                &[
                    ("Open", ResponseType::Accept),
                    ("Cancel", ResponseType::Cancel),
                ],
            );
            dialog.connect_response({
                let dialog = dialog.clone();
                let tv = tv.clone();
                move |d, resp| {
                    if resp == ResponseType::Accept {
                        if let Some(path) = d.file().and_then(|f| f.path()) {
                            match fs::read_to_string(&path) {
                                Ok(content) => tv.buffer().set_text(&content),
                                Err(e) => eprintln!("Open failed: {}", e),
                            }
                        }
                    }
                    dialog.close();
                }
            });
            dialog.show();
        });
    }

    // Save
    {
        let win = window.clone();
        let tv = text_view.clone();
        save_button.connect_clicked(move |_| {
            let dialog = FileChooserDialog::new(
                Some("Save file"),
                Some(&win),
                FileChooserAction::Save,
                &[
                    ("Save", ResponseType::Accept),
                    ("Cancel", ResponseType::Cancel),
                ],
            );
            dialog.connect_response({
                let dialog = dialog.clone();
                let tv = tv.clone();
                move |d, resp| {
                    if resp == ResponseType::Accept {
                        if let Some(path) = d.file().and_then(|f| f.path()) {
                            let buf = tv.buffer();
                            let start = buf.start_iter();
                            let end = buf.end_iter();
                            let text = buf.text(&start, &end, true).to_string();
                            if let Err(e) = fs::write(&path, text.as_bytes()) {
                                eprintln!("Save failed: {}", e);
                            }
                        }
                    }
                    dialog.close();
                }
            });
            dialog.show();
        });
    }

    // Info
    {
        let win = window.clone();
        info_button.connect_clicked(move |_| {
            let dialog = MessageDialog::new(
                Some(&win),
                DialogFlags::MODAL,
                MessageType::Info,
                ButtonsType::Ok,
                "SQE IDLE\n\nA simple SQE editor with live HTML preview.\n\
                 - New/Open/Save files\n\
                 - Run or Live preview via sqe-core\n                 - Terminal shows stdout/stderr\n                 - Resizable terminal (drag divider) + toggle button",
            );
            dialog.connect_response(|d, _| d.close());
            dialog.show();
        });
    }

    // Toggle terminal visibility
    {
        let revealer = terminal_revealer.clone();
        toggle_term_button.connect_clicked(move |_| {
            revealer.set_reveal_child(!revealer.reveals_child());
        });
    }

    window.present();
}

fn run_sqe_core(code: String, sender_html: glib::Sender<Option<String>>, term_sender: glib::Sender<String>) {
    // Create a unique temp directory
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let tmp_path = std::env::temp_dir().join(format!("sqe_idle_{}_{}", ts, std::process::id()));
    if let Err(e) = fs::create_dir_all(&tmp_path) {
        let _ = term_sender.send(format!("Failed to create temp dir: {}", e));
        let _ = sender_html.send(None);
        return;
    }

    // Write input
    let sqe_file = tmp_path.join("temp.sqe");
    if let Err(e) = fs::write(&sqe_file, code.as_bytes()) {
        let _ = term_sender.send(format!("Failed to write temp.sqe: {}", e));
        let _ = sender_html.send(None);
        return;
    }

    // Run sqe-core in a thread
    let sender_html_cl = sender_html.clone();
    let term_cl = term_sender.clone();
    let tmp_path_cl = tmp_path.clone();
    thread::spawn(move || {
        match Command::new("sqe-core")
            .arg("--input").arg(&sqe_file)
            .arg("--output").arg(&tmp_path_cl)
            .output()
        {
            Ok(o) => {
                let _ = term_cl.send(format!("> sqe-core exited with {}", o.status));
                if !o.stdout.is_empty() {
                    let _ = term_cl.send(String::from_utf8_lossy(&o.stdout).to_string());
                }
                if !o.stderr.is_empty() {
                    let _ = term_cl.send(String::from_utf8_lossy(&o.stderr).to_string());
                }

                if o.status.success() {
                    if let Some(html) = find_first_html(&tmp_path_cl) {
                        let _ = sender_html_cl.send(Some(html.to_string_lossy().to_string()));
                    } else {
                        let _ = term_cl.send("No HTML output found".into());
                        let _ = sender_html_cl.send(None);
                    }
                } else {
                    let _ = term_cl.send("sqe-core failed".into());
                    let _ = sender_html_cl.send(None);
                }
            }
            Err(e) => {
                let _ = term_cl.send(format!("Failed to execute sqe-core: {}", e));
                let _ = sender_html_cl.send(None);
            }
        }
    });
}

/// Recursive search for the first .html/.htm file
fn find_first_html(dir: &Path) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&path) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_file() {
                    if let Some(ext) = p.extension() {
                        if ext.eq_ignore_ascii_case("html")
                            || ext.eq_ignore_ascii_case("htm")
                        {
                            return Some(p);
                        }
                    }
                } else if p.is_dir() {
                    stack.push(p);
                }
            }
        }
    }
    None
}
