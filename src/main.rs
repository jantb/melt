#![deny(
    unused_import_braces,
    unused_imports,
    unused_variables,
    unused_allocation,
    unused_extern_crates
)]
#![allow(dead_code, non_upper_case_globals)]
#![windows_subsystem = "windows"]

use std::sync::atomic::Ordering::SeqCst;
use std::sync::Mutex;

use bincode::deserialize;
use crossbeam_channel::bounded;
use druid::im::Vector;
use druid::{AppLauncher, WindowDesc, WindowState};
use once_cell::sync::Lazy;
use serde as _;

use data::AppState;
use view::build_ui;

use crate::data::SerializableParameters;
use crate::delegate::Delegate;
use crate::index::{get_file_as_byte_vec, search_thread, CommandMessage, GLOBAL_DATA_SIZE};

mod data;

mod view;

mod delegate;
mod index;

pub struct GlobalState {
    query: String,
    query_neg: String,
}

impl Default for GlobalState {
    fn default() -> Self {
        GlobalState {
            query: "".to_string(),
            query_neg: "".to_string(),
        }
    }
}

pub static GLOBAL_STATE: Lazy<Mutex<GlobalState>> =
    Lazy::new(|| Mutex::new(GlobalState::default()));

#[tokio::main]
async fn main() -> () {
    let main_window = WindowDesc::new(build_ui())
        .title("Melt listening on socket://localhost:7999 expected format is JSON Lines https://jsonlines.org")
        .window_size((1024.0, 768.0))
        .set_window_state(WindowState::Maximized);
    let (tx_search, rx_search) = bounded(0);

    let launcher = AppLauncher::with_window(main_window);
    let sink = launcher.get_external_handle();
    let parameters = load_from_json();
    let state = AppState {
        query: "".to_string(),
        timelimit: 50.0,
        index_prob: 0.6,
        viewlimit: 500.0,
        not_query: "".to_string(),
        exact: false,
        items: Default::default(),
        view: "".to_string(),
        pointers: Vector::from(parameters.pointer_state),
        query_time: "".to_string(),
        count: "0".to_string(),
        size: "0".to_string(),
        prob: "".to_string(),
        indexed_data_in_bytes: parameters.indexed_data_in_bytes,
        indexed_data_in_bytes_string: "".to_string(),
        settings: false,
        ongoing_search: false,
        properties: Default::default(),
        view_column: parameters.view_column,
        tx: tx_search.clone(),
    };
    let handle = search_thread(rx_search, tx_search.clone(), sink).await;
    launcher
        .delegate(Delegate {})
        .launch(state)
        .expect("Failed to launch application");
    tx_search.send(CommandMessage::Quit).unwrap();
    handle.join().unwrap();
}

pub fn load_from_json() -> SerializableParameters {
    //   let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
    let path = ".melt_state.dat";
    let file = get_file_as_byte_vec(&path);
    match file {
        Ok(file) => {
            let parameters: SerializableParameters =
                deserialize(&file).unwrap_or(SerializableParameters::default());
            GLOBAL_DATA_SIZE.store(parameters.indexed_data_in_bytes, SeqCst);
            parameters
        }
        Err(_) => SerializableParameters::default(),
    }
}
