#![deny(
unused_import_braces,
unused_imports,
unused_variables,
unused_allocation,
unused_crate_dependencies,
unused_extern_crates
)]
#![allow(dead_code, non_upper_case_globals)]

#![windows_subsystem = "windows"]

use crossbeam_channel::bounded;
use druid::{AppLauncher, WindowDesc};
use serde as _;

mod data;

use data::AppState;

mod view;

use view::build_ui;
use crate::delegate::Delegate;

mod index;
mod delegate;

use crate::index::{CommandMessage, search_thread};

pub fn main() {
    let main_window = WindowDesc::new(build_ui())
        .title("Melt")
        .window_size((1000.0, 400.0));
    let (tx_res, rx_res) = bounded(10);
    let (tx_search, rx_search) = bounded(10);


    let launcher = AppLauncher::with_window(main_window);
    let sink = launcher.get_external_handle();
    let handle = search_thread(rx_search, tx_search.clone(), tx_res, sink);
    launcher
        .delegate(Delegate {})
        .launch(AppState {
            query: "".to_string(),
            items: Default::default(),
            view: "".to_string(),
            pointers: Default::default(),
            query_time: "".to_string(),
            count: "0".to_string(),
            size: "0".to_string(),
            prob: "".to_string(),
            settings: false,
            properties: Default::default(),
            view_column: "".to_string(),
            tx: tx_search.clone(),
            rx: rx_res,
        })
        .expect("Failed to launch application");
    tx_search.send(CommandMessage::Quit).unwrap();
    handle.join().unwrap();
}
