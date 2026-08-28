use crate::config::SavedServer;
use crate::state::{Session, Status, TransmitMode};
use adw::prelude::*;
use gtk::glib;
use gtk::{
    Align, Box as GtkBox, Button, CheckButton, DropDown, Entry, HeaderBar, Label, ListBox,
    ListBoxRow, Orientation, PasswordEntry, PolicyType, Scale, ScrolledWindow, Separator,
    SpinButton, Stack, ToggleButton, Window,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use vent_audio::AudioDevice;
use vent_core::TreeNode;

fn new_id() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{n:x}")
}

#[derive(Clone)]
struct ConnectView {
    list: ListBox,
    hint: Label,
    err: Label,
    status: Label,
}

pub fn build(app: &adw::Application, session: Rc<RefCell<Session>>) {
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some("VentLinux"));
    window.set_default_size(480, 680);

    let outer = GtkBox::new(Orientation::Vertical, 0);
    let header = HeaderBar::new();
    header.set_title_widget(Some(&Label::new(Some("VentLinux"))));

    let add_btn = Button::with_label("Add Server");
    add_btn.add_css_class("suggested-action");
    add_btn.set_widget_name("hdr-add");
    let prefs_btn = Button::from_icon_name("emblem-system-symbolic");
    prefs_btn.set_tooltip_text(Some("Preferences"));
    header.pack_start(&add_btn);
    header.pack_end(&prefs_btn);
    outer.append(&header);

    let stack = Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    let (connect_page, connect_view) =
        build_connect(session.clone(), window.clone(), stack.clone());
    let main = build_main(session.clone(), window.clone());
    stack.add_named(&connect_page, Some("connect"));
    stack.add_named(&main, Some("main"));
    outer.append(&stack);

    window.set_content(Some(&outer));
    window.present();

    let sess = session.clone();
    let win = window.clone();
    let stack_add = stack.clone();
    let cv_add = connect_view.clone();
    add_btn.connect_clicked(move |_| {
        server_editor(&win, &sess, None, &stack_add, &cv_add);
    });

    let sess = session.clone();
    let win = window.clone();
    prefs_btn.connect_clicked(move |_| {
        preferences(&win, &sess);
    });

    let sess = session.clone();
    let stack2 = stack.clone();
    let add2 = add_btn.clone();
    let cv_poll = connect_view.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let dirty = sess.borrow_mut().poll();
        let st = sess.borrow().status;
        add2.set_visible(matches!(st, Status::Disconnected | Status::Connecting));
        if dirty {
            refresh(&sess, &stack2, &cv_poll);
        }
        maybe_password_prompt(&sess, &window);
        glib::ControlFlow::Continue
    });
}

fn maybe_password_prompt(session: &Rc<RefCell<Session>>, parent: &adw::ApplicationWindow) {
    let prompt = session.borrow_mut().password_prompt.take();
    let Some((id, name)) = prompt else {
        return;
    };
    let dlg = Window::new();
    dlg.set_transient_for(Some(parent));
    dlg.set_modal(true);
    dlg.set_title(Some("Channel password"));
    dlg.set_default_width(320);
    dlg.set_default_height(160);

    let page = GtkBox::new(Orientation::Vertical, 8);
    page.set_margin_top(16);
    page.set_margin_bottom(16);
    page.set_margin_start(16);
    page.set_margin_end(16);
    page.append(&Label::new(Some(&format!("“{name}” requires a password"))));
    let pass = PasswordEntry::new();
    pass.set_show_peek_icon(true);
    page.append(&pass);
    let btns = GtkBox::new(Orientation::Horizontal, 8);
    let cancel = Button::with_label("Cancel");
    let join = Button::with_label("Join");
    join.add_css_class("suggested-action");
    btns.append(&cancel);
    btns.append(&join);
    page.append(&btns);
    dlg.set_child(Some(&page));

    let d2 = dlg.clone();
    cancel.connect_clicked(move |_| d2.close());
    let sess = session.clone();
    let d2 = dlg.clone();
    join.connect_clicked(move |_| {
        sess.borrow_mut().join_with_password(id, &pass.text());
        d2.close();
    });
    dlg.present();
}

fn refresh(session: &Rc<RefCell<Session>>, stack: &Stack, connect: &ConnectView) {
    let st = session.borrow().status;
    match st {
        Status::Connected | Status::Reconnecting => stack.set_visible_child_name("main"),
        _ => stack.set_visible_child_name("connect"),
    }
    if let Some(main) = stack.child_by_name("main") {
        if let Ok(box_) = main.downcast::<GtkBox>() {
            rebuild_main(&box_, session);
        }
    }
    connect
        .err
        .set_text(session.borrow().last_error.as_deref().unwrap_or(""));
    connect.status.set_text(&session.borrow().connect_status);
    fill_servers(&connect.list, session, &connect.hint);
}

fn build_connect(
    session: Rc<RefCell<Session>>,
    parent: adw::ApplicationWindow,
    stack: Stack,
) -> (GtkBox, ConnectView) {
    let page = GtkBox::new(Orientation::Vertical, 10);
    page.set_margin_top(20);
    page.set_margin_bottom(20);
    page.set_margin_start(24);
    page.set_margin_end(24);

    page.append(&Label::new(Some("Connect to a Ventrilo 3 server")));

    let hint = Label::new(Some("No servers yet — click Add Server in the header."));
    hint.add_css_class("dim-label");
    hint.set_widget_name("empty-hint");
    hint.set_wrap(true);
    page.append(&hint);

    let list = ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    let scroll = ScrolledWindow::new();
    scroll.set_min_content_height(180);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    scroll.set_widget_name("server-list");
    page.append(&scroll);

    let err = Label::new(None);
    err.add_css_class("error");
    err.set_wrap(true);
    err.set_widget_name("connect-error");
    page.append(&err);

    let status = Label::new(None);
    status.add_css_class("dim-label");
    status.set_widget_name("connect-status");
    page.append(&status);

    let buttons = GtkBox::new(Orientation::Horizontal, 8);
    let edit = Button::with_label("Edit");
    let delete = Button::with_label("Delete");
    delete.add_css_class("destructive-action");
    let connect_btn = Button::with_label("Connect");
    connect_btn.add_css_class("suggested-action");
    buttons.append(&edit);
    buttons.append(&delete);
    buttons.append(&connect_btn);
    page.append(&buttons);

    let connect_view = ConnectView {
        list: list.clone(),
        hint: hint.clone(),
        err: err.clone(),
        status: status.clone(),
    };

    let sess = session.clone();
    let parent_c = parent.clone();
    let stack_c = stack.clone();
    let cv = connect_view.clone();
    edit.connect_clicked(move |_| {
        let srv = sess.borrow().selected_server();
        if let Some(s) = srv {
            server_editor(&parent_c, &sess, Some(s), &stack_c, &cv);
        } else {
            sess.borrow_mut().last_error = Some("Select a server first.".into());
            refresh(&sess, &stack_c, &cv);
        }
    });

    let sess = session.clone();
    let stack_c = stack.clone();
    let cv = connect_view.clone();
    delete.connect_clicked(move |_| {
        sess.borrow_mut().remove_selected();
        refresh(&sess, &stack_c, &cv);
    });

    let sess = session.clone();
    let stack_c = stack.clone();
    let cv = connect_view.clone();
    connect_btn.connect_clicked(move |_| {
        let srv = sess.borrow().selected_server();
        if let Some(s) = srv {
            sess.borrow_mut().connect(&s);
        } else {
            sess.borrow_mut().last_error = Some("Select a server first.".into());
            refresh(&sess, &stack_c, &cv);
        }
    });

    fill_servers(&list, &session, &hint);

    let sess = session.clone();
    list.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            let id = row.widget_name().to_string();
            if !id.is_empty() && !id.starts_with("Gtk") {
                sess.borrow_mut().select_server(id);
            }
        }
    });
    (page, connect_view)
}

fn fill_servers(list: &ListBox, session: &Rc<RefCell<Session>>, hint: &Label) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let (servers, selected_id) = {
        let s = session.borrow();
        (s.servers.clone(), s.selected_server_id.clone())
    };
    hint.set_visible(servers.is_empty());
    let mut to_select: Option<ListBoxRow> = None;
    for s in &servers {
        let row = ListBoxRow::new();
        row.set_activatable(true);
        row.set_selectable(true);
        row.set_widget_name(&s.id);
        let col = GtkBox::new(Orientation::Vertical, 2);
        col.set_margin_top(8);
        col.set_margin_bottom(8);
        col.set_margin_start(10);
        col.set_margin_end(10);
        let n = Label::new(Some(&s.name));
        n.set_halign(Align::Start);
        n.add_css_class("heading");
        let sub = Label::new(Some(&format!("{}  ·  {}", s.display_address(), s.username)));
        sub.set_halign(Align::Start);
        sub.add_css_class("dim-label");
        col.append(&n);
        col.append(&sub);
        row.set_child(Some(&col));

        let sess = session.clone();
        let id = s.id.clone();
        let g = gtk::GestureClick::new();
        g.set_button(1);
        g.connect_released(move |_, n_press, _, _| {
            sess.borrow_mut().select_server(id.clone());
            sess.borrow_mut().last_error = None;
            if n_press >= 2 {
                let server = sess.borrow().selected_server();
                if let Some(srv) = server {
                    sess.borrow_mut().connect(&srv);
                }
            }
        });
        row.add_controller(g);

        if selected_id.as_deref() == Some(s.id.as_str()) {
            to_select = Some(row.clone());
        }
        list.append(&row);
    }
    if let Some(row) = to_select {
        list.select_row(Some(&row));
    } else if let Some(row) = list.row_at_index(0) {
        list.select_row(Some(&row));
        if let Some(first) = servers.first() {
            session.borrow_mut().select_server(first.id.clone());
        }
    }
}

fn server_editor(
    parent: &impl IsA<gtk::Window>,
    session: &Rc<RefCell<Session>>,
    existing: Option<SavedServer>,
    stack: &Stack,
    connect: &ConnectView,
) {
    let dlg = Window::new();
    dlg.set_transient_for(Some(parent));
    dlg.set_modal(true);
    dlg.set_destroy_with_parent(true);
    dlg.set_title(Some(if existing.is_some() {
        "Edit Server"
    } else {
        "Add Server"
    }));
    dlg.set_default_width(400);
    dlg.set_default_height(380);

    let page = GtkBox::new(Orientation::Vertical, 8);
    page.set_margin_top(16);
    page.set_margin_bottom(16);
    page.set_margin_start(16);
    page.set_margin_end(16);

    let name = Entry::new();
    name.set_placeholder_text(Some("Friends' server"));
    let host = Entry::new();
    host.set_placeholder_text(Some("vent.example.com"));
    let port = SpinButton::with_range(1.0, 65535.0, 1.0);
    port.set_digits(0);
    port.set_numeric(true);
    port.set_snap_to_ticks(true);
    port.set_increments(1.0, 10.0);
    let user = Entry::new();
    user.set_placeholder_text(Some("username"));
    let pass = PasswordEntry::new();
    pass.set_show_peek_icon(true);
    let err = Label::new(None);
    err.add_css_class("error");
    err.set_wrap(true);

    if let Some(s) = &existing {
        name.set_text(&s.name);
        host.set_text(&s.host);
        port.set_value(s.port as f64);
        user.set_text(&s.username);
        pass.set_text(&s.password);
    } else {
        port.set_value(3784.0);
    }

    for (cap, w) in [
        ("Name", name.clone().upcast::<gtk::Widget>()),
        ("Host", host.clone().upcast::<gtk::Widget>()),
        ("Port", port.clone().upcast::<gtk::Widget>()),
        ("Username", user.clone().upcast::<gtk::Widget>()),
        ("Password (optional)", pass.clone().upcast::<gtk::Widget>()),
    ] {
        let l = Label::new(Some(cap));
        l.set_halign(Align::Start);
        page.append(&l);
        page.append(&w);
    }
    page.append(&err);

    let btns = GtkBox::new(Orientation::Horizontal, 8);
    btns.set_halign(Align::End);
    let cancel = Button::with_label("Cancel");
    let save = Button::with_label("Save");
    save.add_css_class("suggested-action");
    btns.append(&cancel);
    btns.append(&save);
    page.append(&btns);
    dlg.set_child(Some(&page));

    let d2 = dlg.clone();
    cancel.connect_clicked(move |_| d2.close());

    let sess = session.clone();
    let d2 = dlg.clone();
    let stack = stack.clone();
    let connect = connect.clone();
    let existing_id = existing.map(|s| s.id);
    save.connect_clicked(move |_| {
        let host_s = host.text().trim().to_string();
        let user_s = user.text().trim().to_string();
        if host_s.is_empty() || user_s.is_empty() {
            err.set_text("Host and username are required.");
            return;
        }
        let port_n = port.value().clamp(1.0, 65535.0) as u16;
        let mut s = sess.borrow_mut();
        let id = existing_id.clone().unwrap_or_else(new_id);
        let srv = SavedServer {
            id: id.clone(),
            name: {
                let n = name.text().trim().to_string();
                if n.is_empty() {
                    host_s.clone()
                } else {
                    n
                }
            },
            host: host_s,
            port: port_n,
            username: user_s,
            password: pass.text().to_string(),
        };
        if let Some(i) = s.servers.iter().position(|x| x.id == id) {
            s.servers[i] = srv;
        } else {
            s.servers.push(srv);
        }
        s.selected_server_id = Some(id);
        s.last_error = None;
        s.persist();
        drop(s);
        d2.close();
        refresh(&sess, &stack, &connect);
    });

    dlg.present();
}

fn preferences(parent: &impl IsA<gtk::Window>, session: &Rc<RefCell<Session>>) {
    let dlg = Window::new();
    dlg.set_transient_for(Some(parent));
    dlg.set_modal(true);
    dlg.set_destroy_with_parent(true);
    dlg.set_title(Some("Preferences"));
    dlg.set_default_width(420);
    dlg.set_default_height(460);

    let page = GtkBox::new(Orientation::Vertical, 10);
    page.set_margin_top(16);
    page.set_margin_bottom(16);
    page.set_margin_start(16);
    page.set_margin_end(16);

    let heading = |t: &str| {
        let l = Label::new(Some(t));
        l.set_halign(Align::Start);
        l.add_css_class("heading");
        l
    };

    page.append(&heading("Push-to-talk"));
    let ptt_lbl = Label::new(None);
    ptt_lbl.set_halign(Align::Start);
    let ptt_btn = Button::with_label("Click, then press a key or mouse button");
    {
        let s = session.borrow();
        ptt_lbl.set_text(&format!("Current: {}", s.config.ptt.display));
    }
    page.append(&ptt_lbl);
    page.append(&ptt_btn);

    page.append(&heading("Transmit mode"));
    let ptt_mode = CheckButton::with_label("Push-to-talk");
    let vox_mode = CheckButton::with_label("Voice activation");
    vox_mode.set_group(Some(&ptt_mode));
    {
        let s = session.borrow();
        if s.transmit_mode == TransmitMode::Vox {
            vox_mode.set_active(true);
        } else {
            ptt_mode.set_active(true);
        }
    }
    page.append(&ptt_mode);
    page.append(&vox_mode);

    page.append(&heading("Voice-activation sensitivity"));
    let scale = Scale::with_range(Orientation::Horizontal, -60.0, -15.0, 1.0);
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.set_value(session.borrow().config.vox_sensitivity as f64);
    page.append(&scale);

    let (ins, outs) = vent_audio::list_devices();
    page.append(&heading("Microphone"));
    let (in_combo, in_ids) = device_dropdown(&ins, true, &session.borrow().config.input_device);
    page.append(&in_combo);
    page.append(&heading("Speakers / headset"));
    let (out_combo, out_ids) =
        device_dropdown(&outs, false, &session.borrow().config.output_device);
    page.append(&out_combo);

    let close = Button::with_label("Close");
    close.add_css_class("suggested-action");
    close.set_halign(Align::End);
    page.append(&close);

    dlg.set_child(Some(&page));

    let sess = session.clone();
    let ptt_lbl_c = ptt_lbl.clone();
    ptt_btn.connect_clicked(move |b| {
        b.set_label("Press a key or mouse button…");
        sess.borrow().begin_ptt_capture();
        let sess = sess.clone();
        let b = b.clone();
        let ptt_lbl_c = ptt_lbl_c.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let s = sess.borrow();
            if s.ptt.as_ref().is_some_and(|p| p.is_capturing()) {
                glib::ControlFlow::Continue
            } else {
                ptt_lbl_c.set_text(&format!("Current: {}", s.config.ptt.display));
                b.set_label("Click, then press a key or mouse button");
                glib::ControlFlow::Break
            }
        });
    });

    let sess = session.clone();
    ptt_mode.connect_toggled(move |b| {
        if b.is_active() {
            sess.borrow_mut().set_transmit_mode(TransmitMode::Ptt);
        }
    });
    let sess = session.clone();
    vox_mode.connect_toggled(move |b| {
        if b.is_active() {
            sess.borrow_mut().set_transmit_mode(TransmitMode::Vox);
        }
    });

    let sess = session.clone();
    scale.connect_value_changed(move |sc| {
        sess.borrow_mut().set_vox_sensitivity(sc.value() as f32);
    });

    let sess = session.clone();
    in_combo.connect_selected_notify(move |c| {
        let id = in_ids
            .get(c.selected() as usize)
            .cloned()
            .unwrap_or_default();
        let mut s = sess.borrow_mut();
        s.config.input_device = id;
        s.persist();
    });
    let sess = session.clone();
    out_combo.connect_selected_notify(move |c| {
        let id = out_ids
            .get(c.selected() as usize)
            .cloned()
            .unwrap_or_default();
        let mut s = sess.borrow_mut();
        s.config.output_device = id;
        s.persist();
    });

    let d2 = dlg.clone();
    close.connect_clicked(move |_| d2.close());
    dlg.present();
}

fn device_dropdown(
    devs: &[AudioDevice],
    input: bool,
    current: &str,
) -> (DropDown, Rc<Vec<String>>) {
    let mut ids = vec![String::new()];
    let mut labels = vec!["System default (Pulse/PipeWire)".to_string()];
    for d in devs {
        if (input && d.is_input) || (!input && d.is_output) {
            ids.push(d.name.clone());
            labels.push(d.name.clone());
        }
    }
    let label_refs: Vec<_> = labels.iter().map(String::as_str).collect();
    let dropdown = DropDown::from_strings(&label_refs);
    let selected = ids.iter().position(|id| id == current).unwrap_or(0);
    dropdown.set_selected(selected as u32);
    (dropdown, Rc::new(ids))
}

fn build_main(session: Rc<RefCell<Session>>, _parent: adw::ApplicationWindow) -> GtkBox {
    let page = GtkBox::new(Orientation::Vertical, 0);
    page.set_widget_name("main-page");

    let header = GtkBox::new(Orientation::Horizontal, 8);
    header.set_margin_top(10);
    header.set_margin_bottom(10);
    header.set_margin_start(12);
    header.set_margin_end(12);
    let title = Label::new(Some(""));
    title.set_halign(Align::Start);
    title.set_hexpand(true);
    title.add_css_class("heading");
    title.set_widget_name("main-title");
    let chat_btn = ToggleButton::with_label("Chat");
    let disc = Button::with_label("Disconnect");
    header.append(&title);
    header.append(&chat_btn);
    header.append(&disc);
    page.append(&header);
    page.append(&Separator::new(Orientation::Horizontal));

    let list = ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_widget_name("tree");
    let scroll = ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_child(Some(&list));
    page.append(&scroll);

    let chat_box = GtkBox::new(Orientation::Vertical, 4);
    chat_box.set_widget_name("chat-pane");
    chat_box.set_visible(false);
    chat_box.set_margin_start(8);
    chat_box.set_margin_end(8);
    let chat_log = Label::new(None);
    chat_log.set_halign(Align::Start);
    chat_log.set_wrap(true);
    chat_log.set_widget_name("chat-log");
    let chat_scroll = ScrolledWindow::new();
    chat_scroll.set_min_content_height(120);
    chat_scroll.set_child(Some(&chat_log));
    let chat_entry = Entry::new();
    chat_entry.set_placeholder_text(Some("Message"));
    chat_box.append(&chat_scroll);
    chat_box.append(&chat_entry);
    page.append(&chat_box);

    page.append(&Separator::new(Orientation::Horizontal));
    let footer = GtkBox::new(Orientation::Vertical, 6);
    footer.set_margin_top(8);
    footer.set_margin_bottom(8);
    footer.set_margin_start(12);
    footer.set_margin_end(12);
    let ptt_lbl = Label::new(Some("Hold Left Ctrl to talk"));
    ptt_lbl.set_halign(Align::Start);
    ptt_lbl.set_widget_name("ptt-status");
    footer.append(&ptt_lbl);

    let mutes = GtkBox::new(Orientation::Horizontal, 16);
    let sound = CheckButton::with_label("Mute Sound");
    let mic = CheckButton::with_label("Mute Microphone");
    mutes.append(&sound);
    mutes.append(&mic);
    footer.append(&mutes);
    page.append(&footer);

    let sess = session.clone();
    disc.connect_clicked(move |_| sess.borrow_mut().disconnect());

    let sess = session.clone();
    let chat_box_c = chat_box.clone();
    chat_btn.connect_toggled(move |b| {
        let open = b.is_active();
        chat_box_c.set_visible(open);
        sess.borrow_mut().set_chat_open(open);
    });

    let sess = session.clone();
    chat_entry.connect_activate(move |e| {
        let t = e.text().to_string();
        e.set_text("");
        sess.borrow_mut().send_chat(&t);
    });

    let sess = session.clone();
    sound.connect_toggled(move |b| sess.borrow_mut().set_sound_muted(b.is_active()));
    let sess = session.clone();
    mic.connect_toggled(move |b| sess.borrow_mut().set_mic_muted(b.is_active()));

    rebuild_main(&page, &session);
    page
}

fn rebuild_main(page: &GtkBox, session: &Rc<RefCell<Session>>) {
    let (title, ptt_text, chat) = {
        let s = session.borrow();
        let ping = s.ping.map(|p| format!("  ·  {p} ms")).unwrap_or_default();
        let title = format!("{}{ping}", s.server_name);
        let ptt_text = if s.mic_muted {
            "Microphone muted".into()
        } else if s.transmitting {
            "Transmitting".into()
        } else if s.transmit_mode == TransmitMode::Vox {
            "Voice activation on — speak to talk".into()
        } else {
            format!("Hold {} to talk", s.config.ptt.display)
        };
        let chat = s
            .chat_log
            .iter()
            .rev()
            .take(40)
            .cloned()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        (title, ptt_text, chat)
    };
    let mut tree: Option<ListBox> = None;
    walk(page, &mut |w| match w.widget_name().as_str() {
        "main-title" => {
            if let Some(l) = w.downcast_ref::<Label>() {
                l.set_text(&title);
            }
        }
        "ptt-status" => {
            if let Some(l) = w.downcast_ref::<Label>() {
                l.set_text(&ptt_text);
            }
        }
        "chat-log" => {
            if let Some(l) = w.downcast_ref::<Label>() {
                l.set_text(&chat);
            }
        }
        "tree" => {
            if let Some(list) = w.downcast_ref::<ListBox>() {
                tree = Some(list.clone());
            }
        }
        _ => {}
    });
    if let Some(list) = tree {
        fill_tree(&list, session);
    }
}

fn fill_tree(list: &ListBox, session: &Rc<RefCell<Session>>) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let s = session.borrow();
    let own_ch = s.own_channel_id;
    let own_id = s.own_user_id;
    let talking = s.roster.talking.clone();
    let transmitting = s.transmitting;
    let rows = s.roster.flattened_tree();
    drop(s);

    for (depth, node) in rows {
        let row = ListBoxRow::new();
        let line = GtkBox::new(Orientation::Horizontal, 6);
        line.set_margin_start(8 + 16 * depth as i32);
        line.set_margin_top(4);
        line.set_margin_bottom(4);
        match node {
            TreeNode::Channel(ch) => {
                let lock = if ch.password_protected { "🔒 " } else { "" };
                let here = if ch.id == own_ch { "  ← you" } else { "" };
                let l = Label::new(Some(&format!("{lock}▸ {}{here}", ch.name)));
                l.set_halign(Align::Start);
                if ch.id == own_ch {
                    l.add_css_class("heading");
                }
                line.append(&l);
                let sess = session.clone();
                let id = ch.id;
                let pw = ch.password_protected;
                let g = gtk::GestureClick::new();
                g.connect_pressed(move |_, n, _, _| {
                    if n == 2 {
                        sess.borrow_mut().join(id, pw);
                    }
                });
                row.add_controller(g);
            }
            TreeNode::User(u) => {
                let me = u.id == own_id;
                let is_talk = talking.contains(&u.id) || (me && transmitting);
                let mic = if is_talk { "🎙 " } else { "• " };
                let you = if me { " (you)" } else { "" };
                let comment = if u.comment.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", u.comment)
                };
                let l = Label::new(Some(&format!("{mic}{}{you}{comment}", u.name)));
                l.set_halign(Align::Start);
                if is_talk {
                    l.add_css_class("success");
                }
                line.append(&l);
            }
        }
        row.set_child(Some(&line));
        list.append(&row);
    }
}

fn walk(w: &impl IsA<gtk::Widget>, f: &mut impl FnMut(&gtk::Widget)) {
    let w = w.upcast_ref::<gtk::Widget>();
    f(w);
    let mut c = w.first_child();
    while let Some(n) = c {
        walk(&n, f);
        c = n.next_sibling();
    }
}
