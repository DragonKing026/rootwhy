// SPDX-License-Identifier: GPL-3.0-or-later
// rootwhy-gtk — the password dialog of rootwhy (GTK 4, API of GTK 4.6 and older only).
//
// Not meant to be run by hand: `rootwhy` starts it from its own directory, hands over the dialog data as JSON on
// stdin and reads the password from stdout. Exit codes: 0 password printed, 1 cancelled, 2 GTK cannot open a display,
// 3 bad input.
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, glib, pango};
use rootwhy_core::dialog::{DialogData, EXIT_CANCELLED, EXIT_NO_GTK};
use rootwhy_core::modes::write_fd;
use std::cell::RefCell;
use std::io::Read;
use std::process::ExitCode;
use std::rc::Rc;
use zeroize::Zeroize;

const LOGO_SVG: &str = include_str!("../../logo.svg"); // same file as the repository's logo

const CSS: &str = "
.rw-reason { font-size: 1.1em; }
.rw-error { color: @error@; font-weight: bold; }
.rw-dim { opacity: 0.7; font-size: 0.9em; }
.rw-box { font-family: monospace; padding: 10px 12px; border-radius: 6px; background-color: @bg@; }
.rw-tok { padding: 0 2px; border-radius: 3px; }
.rw-tok:hover { background-color: @hover@; }
.rw-k-command { color: @cmd@; font-weight: bold; }
.rw-k-subcommand { color: @cmd@; }
.rw-k-option { color: @opt@; }
.rw-k-value { color: @val@; }
.rw-k-path { color: @path@; }
.rw-k-marker, .rw-cont { opacity: 0.55; }
";

struct Palette {
    cmd: &'static str,
    opt: &'static str,
    val: &'static str,
    path: &'static str,
    error: &'static str,
    bg: &'static str,
    hover: &'static str,
}

const LIGHT: Palette = Palette {
    cmd: "#1a5fb4",
    opt: "#813d9c",
    val: "#b04a00",
    path: "#1c7a43",
    error: "#c01c28",
    bg: "rgba(0,0,0,0.05)",
    hover: "rgba(0,0,0,0.10)",
};
const DARK: Palette = Palette {
    cmd: "#78aeed",
    opt: "#dc8add",
    val: "#ffa348",
    path: "#8ff0a4",
    error: "#ff7b63",
    bg: "rgba(255,255,255,0.06)",
    hover: "rgba(255,255,255,0.14)",
};

fn css_for(p: &Palette) -> String {
    CSS.replace("@cmd@", p.cmd)
        .replace("@opt@", p.opt)
        .replace("@val@", p.val)
        .replace("@path@", p.path)
        .replace("@error@", p.error)
        .replace("@bg@", p.bg)
        .replace("@hover@", p.hover)
}

fn label(text: &str, css: &str, wrap: bool) -> gtk::Label {
    let lbl = gtk::Label::new(Some(text));
    lbl.set_xalign(0.0);
    lbl.set_wrap(wrap);
    lbl.set_wrap_mode(pango::WrapMode::WordChar);
    for c in css.split_whitespace() {
        lbl.add_css_class(c);
    }
    lbl
}

fn logo_image() -> gtk::Image {
    match gdk::Texture::from_bytes(&glib::Bytes::from_static(LOGO_SVG.as_bytes())) {
        Ok(texture) => {
            let image = gtk::Image::from_paintable(Some(&texture));
            image.set_pixel_size(48);
            image
        }
        Err(_) => {
            // no SVG loader — the theme's icon will do
            let image = gtk::Image::from_icon_name("dialog-password");
            image.set_pixel_size(48);
            image
        }
    }
}

fn save_screenshot(win: &gtk::Window, path: &str) {
    let (w, h) = (win.width(), win.height());
    let paintable = gtk::WidgetPaintable::new(Some(win));
    let snap = gtk::Snapshot::new();
    paintable.snapshot(&snap, w as f64, h as f64);
    if let (Some(node), Some(renderer)) = (snap.to_node(), win.native().and_then(|n| n.renderer())) {
        let _ = renderer.render_texture(node, None).save_to_png(path);
    }
}

fn main() -> ExitCode {
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        return ExitCode::from(3);
    }
    let Ok(data) = serde_json::from_slice::<DialogData>(&input) else { return ExitCode::from(3) };
    if gtk::init().is_err() {
        return ExitCode::from(EXIT_NO_GTK as u8);
    }
    let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let main_loop = glib::MainLoop::new(None, false);

    let win = gtk::Window::builder().title(data.title.as_str()).default_width(860).default_height(-1).resizable(true).build();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.set_margin_top(18);
    outer.set_margin_bottom(18);
    outer.set_margin_start(18);
    outer.set_margin_end(18);
    win.set_child(Some(&outer));

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let icon = logo_image();
    icon.set_valign(gtk::Align::Start);
    head.append(&icon);
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 6);
    texts.set_hexpand(true);
    head.append(&texts);
    outer.append(&head);

    if !data.retry.is_empty() {
        texts.append(&label(&data.retry, "rw-error", true));
    }
    if !data.preview.is_empty() {
        texts.append(&label(&data.preview, "rw-dim", true));
    }
    texts.append(&label(&data.reason, "rw-reason", true));

    outer.append(&label(&data.command_label, "rw-dim", true));
    let cbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
    cbox.add_css_class("rw-box");
    let nlines = data.lines.len();
    for (n, line) in data.lines.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        if n > 0 {
            row.set_margin_start(28);
        }
        for (k, t) in line.iter().enumerate() {
            if k > 0 && !t.joined {
                row.append(&gtk::Label::new(Some(" ")));
            }
            let tok = gtk::Label::new(Some(&t.text));
            tok.set_xalign(0.0);
            if t.text.chars().count() > 110 {
                // only very long parts wrap; the window grows for the rest
                tok.set_wrap(true);
                tok.set_wrap_mode(pango::WrapMode::WordChar);
                tok.set_width_chars(110);
                tok.set_max_width_chars(110);
            }
            tok.set_selectable(false);
            tok.add_css_class("rw-tok");
            tok.add_css_class(&format!("rw-k-{}", t.kind));
            if !t.tooltip.is_empty() {
                tok.set_tooltip_markup(Some(&t.tooltip));
            }
            row.append(&tok);
        }
        if n + 1 < nlines {
            let cont = gtk::Label::new(Some(" \\"));
            cont.add_css_class("rw-cont");
            row.append(&cont);
        }
        cbox.append(&row);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(420);
    scroll.set_child(Some(&cbox));
    outer.append(&scroll);

    if !data.meta.is_empty() {
        outer.append(&label(&data.meta, "rw-dim", true));
    }

    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_hexpand(true);
    let pw_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    pw_row.append(&label(&data.password_label, "", false));
    pw_row.append(&entry);
    outer.append(&pw_row);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&data.cancel_label);
    let ok = gtk::Button::with_label(&data.authorize_label);
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    outer.append(&buttons);

    let finish: Rc<dyn Fn(Option<String>)> = {
        let (win, main_loop, result) = (win.clone(), main_loop.clone(), result.clone());
        Rc::new(move |password: Option<String>| {
            if main_loop.is_running() {
                *result.borrow_mut() = password;
                win.destroy();
                main_loop.quit();
            }
        })
    };
    {
        let (finish, entry) = (finish.clone(), entry.clone());
        ok.connect_clicked(move |_| finish(Some(entry.text().to_string())));
    }
    {
        let finish = finish.clone();
        entry.connect_activate(move |e| finish(Some(e.text().to_string())));
    }
    {
        let finish = finish.clone();
        cancel.connect_clicked(move |_| finish(None));
    }
    {
        let finish = finish.clone();
        win.connect_close_request(move |_| {
            finish(None);
            glib::Propagation::Proceed
        });
    }
    {
        let finish = finish.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                finish(None);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        win.add_controller(keys);
    }

    // palette from the theme's text colour: light text -> dark theme
    let probe = gtk::Label::new(None);
    outer.append(&probe);
    win.present();
    let color = probe.style_context().color();
    outer.remove(&probe);
    let dark = (0.2126 * color.red() + 0.7152 * color.green() + 0.0722 * color.blue()) > 0.5;
    let provider = gtk::CssProvider::new();
    provider.load_from_data(&css_for(if dark { &DARK } else { &LIGHT }));
    gtk::style_context_add_provider_for_display(&WidgetExt::display(&win), &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    entry.grab_focus();

    if !data.screenshot.is_empty() {
        let (win, finish, path) = (win.clone(), finish.clone(), data.screenshot.clone());
        glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
            save_screenshot(&win, &path);
            finish(None);
            glib::ControlFlow::Break
        });
    }

    main_loop.run();

    let password = result.borrow_mut().take();
    match password {
        Some(mut p) => {
            // straight to fd 1: a buffered stdout would keep a copy of the password
            let written = write_fd(1, p.as_bytes());
            p.zeroize();
            if written {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(3)
            }
        }
        None => ExitCode::from(EXIT_CANCELLED as u8),
    }
}
