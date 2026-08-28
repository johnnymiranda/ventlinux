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
    app.connect_activate(|app| {
        let session = Rc::new(RefCell::new(Session::new()));
        ui::build(app, session);
    });
    app.run();
}
