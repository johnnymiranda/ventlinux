mod config;
mod state;
mod ui;

use gtk::prelude::*;
use state::Session;
use std::cell::RefCell;
use std::rc::Rc;

fn main() {
    let app = adw::Application::builder()
        .application_id("com.cryptexlabs.ventlinux")
        .build();
    // GApplication activates again whenever the app is launched while already
    // running. Building a second Session would start a duplicate PTT watcher
    // and a second poller competing for the one libventrilo3 connection, so
    // re-activation just raises the window we already have.
    let window: Rc<RefCell<Option<adw::ApplicationWindow>>> = Rc::new(RefCell::new(None));
    app.connect_activate(move |app| {
        if let Some(existing) = window.borrow().clone() {
            existing.present();
            return;
        }
        let session = Rc::new(RefCell::new(Session::new()));
        *window.borrow_mut() = Some(ui::build(app, session));
    });
    app.run();
}
