#![deny(
unused_import_braces,
unused_imports,
unused_variables,
unused_allocation,
unused_crate_dependencies,
unused_extern_crates
)]
#![allow(dead_code, non_upper_case_globals)]


use std::sync::{Arc, mpsc};
use druid::{AppLauncher, WindowDesc};
use druid::text::RichTextBuilder;
use serde as _;

mod data;

use data::AppState;

mod view;

use view::build_ui;
use crate::delegate::Delegate;

mod index;
mod delegate;

use crate::index::search_thread;

pub fn main() {
    let main_window = WindowDesc::new(build_ui())
        .title("Melt")
        .window_size((1000.0, 400.0));

    let (tx_send, rx_send) = mpsc::sync_channel(0);
    let (tx_res, rx_res) = mpsc::sync_channel(0);
    let (tx_search, rx_search) = mpsc::sync_channel(0);


    let launcher = AppLauncher::with_window(main_window);
    let sink = launcher.get_external_handle();
    search_thread(tx_send.clone(), rx_send, rx_search, tx_res, sink);
    launcher
        .delegate(Delegate {})
        .launch(AppState {
            new_todo: "".to_string(),
            query: "".to_string(),
            items: Default::default(),
            view: RichTextBuilder::new().build(),
            query_time: "".to_string(),
            count: "0".to_string(),
            tx: Arc::new(tx_search),
            rx: Arc::new(rx_res),
        })
        .expect("Failed to launch application");
}
