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

use std::fs;
use std::fs::File;
use std::io::{Error, Read};

use bincode::deserialize;
use crossbeam_channel::bounded;
use druid::{AppLauncher, WindowDesc, WindowState};
use druid::im::Vector;
use serde as _;

use data::AppState;
use view::build_ui;

use crate::data::SerializableParameters;
use crate::delegate::Delegate;
use crate::index::{CommandMessage, search_thread};

mod data;

mod view;

mod index;
mod delegate;

pub fn main() {
    let main_window = WindowDesc::new(build_ui())
        .title("Melt listening on socket://localhost:7999 expected format is JSON Lines https://jsonlines.org")
        .window_size((1024.0, 768.0))
        .set_window_state(WindowState::Maximized);
    let (tx_res, rx_res) = bounded(10);
    let (tx_search, rx_search) = bounded(10);


    let launcher = AppLauncher::with_window(main_window);
    let sink = launcher.get_external_handle();
    let handle = search_thread(rx_search, tx_search.clone(), tx_res, sink);
    let parameters = load_from_json();
    let state = AppState {
        query: "".to_string(),
        timelimit: 50.0,
        not_query: "".to_string(),
        exact: false,
        items: Default::default(),
        view: "".to_string(),
        pointers: Vector::from(parameters.pointer_state),
        query_time: "".to_string(),
        count: "0".to_string(),
        size: "0".to_string(),
        prob: "".to_string(),
        settings: false,
        properties: Default::default(),
        view_column: parameters.view_column,
        tx: tx_search.clone(),
        rx: rx_res,
    };


    launcher
        .delegate(Delegate {})
        .launch(state)
        .expect("Failed to launch application");
    tx_search.send(CommandMessage::Quit).unwrap();
    handle.join().unwrap();
}


pub fn load_from_json() -> SerializableParameters {
    let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
    let path = format!("{}/.melt_state.dat", buf);
    let file = get_file_as_byte_vec(&path);
    match file {
        Ok(file) => {
            deserialize(&file).unwrap()
        }
        Err(_) => {
            SerializableParameters{ view_column: "".to_string(), pointer_state: vec![] }
        }
    }
}
fn get_file_as_byte_vec(filename: &String) -> Result<Vec<u8>, Error> {
    let mut f = File::open(&filename)?;
    let metadata = fs::metadata(&filename)?;
    let mut buffer = vec![0; metadata.len() as usize];
    f.read(&mut buffer)?;

    Ok(buffer)
}
