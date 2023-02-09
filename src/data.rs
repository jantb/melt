use std::fs;

use clipboard::{ClipboardContext, ClipboardProvider};
use crossbeam_channel::Sender;
use druid::im::Vector;
use druid::Data;
use druid::Env;
use druid::EventCtx;
use druid::Lens;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::delegate::{SEARCH, SET_VIEW};
use crate::index::CommandMessage;

#[derive(Clone, Data, Lens)]
pub struct AppState {
    pub query: String,
    pub timelimit: f64,
    pub viewlimit: f64,
    pub not_query: String,
    pub exact: bool,
    pub items: Vector<Item>,
    pub view: String,
    pub pointers: Vector<PointerState>,
    pub query_time: String,
    pub count: String,
    pub size: String,
    #[data(ignore)]
    pub indexed_data_in_bytes: u64,
    pub indexed_data_in_bytes_string: String,
    #[data(ignore)]
    pub settings: bool,
    #[data(ignore)]
    pub ongoing_search: bool,
    pub properties: Vector<String>,
    pub view_column: String,

    #[data(ignore)]
    pub tx: Sender<CommandMessage>,
}

impl Drop for AppState {
    fn drop(&mut self) {
        let parameters = self.get_serializable_parameters();
        let serialized: Vec<u8> = bincode::serialize(&parameters).unwrap();
        //let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
        let path = ".melt_state.dat";
        fs::write(path, serialized).unwrap();
    }
}

impl AppState {
    fn get_serializable_parameters(&self) -> SerializableParameters {
        SerializableParameters {
            view_column: self.view_column.to_string(),
            indexed_data_in_bytes: self.indexed_data_in_bytes,
            pointer_state: self
                .pointers
                .iter()
                .map(|p| p.clone())
                .collect::<Vec<PointerState>>(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SerializableParameters {
    pub view_column: String,
    pub pointer_state: Vec<PointerState>,
    pub indexed_data_in_bytes: u64,
}

impl Default for SerializableParameters {
    fn default() -> Self {
        SerializableParameters {
            view_column: "".to_string(),
            indexed_data_in_bytes: 0,
            pointer_state: vec![],
        }
    }
}

#[derive(Clone, Data, Lens, Serialize, Deserialize)]
pub struct PointerState {
    pub text: String,
    pub number: u64,
    pub checked: bool,
}

#[derive(Clone, Data, Lens, Serialize, Deserialize)]
pub struct PointerStateItem {
    pub text: String,
    pub resolved: String,
    pub checked: bool,
}

impl AppState {
    pub fn click_search(ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        ctx.submit_command(SEARCH.with((
            (data.query.to_string(), data.not_query.to_string()),
            data.exact,
        )));
    }
}

#[derive(Clone, Data, Lens)]
pub struct Item {
    #[data(same_fn = "PartialEq::eq")]
    pub id: Uuid,
    pub done: bool,
    pub text: String,
    #[data(ignore)]
    pub pointers: Vec<String>,
    #[data(ignore)]
    pub pointer_states: Vec<PointerStateItem>,
    pub view: String,
}

impl Item {
    pub fn new(text: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            done: false,
            text: text.into(),
            pointers: Default::default(),
            pointer_states: vec![],
            view: "".to_string(),
        }
    }
    pub fn click_copy(_ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        let mut ctx: ClipboardContext = ClipboardProvider::new().unwrap();
        ctx.set_contents(data.text.clone().to_string()).unwrap();
    }

    pub fn click_view(ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        ctx.submit_command(SET_VIEW.with(data.text.to_string()));
    }
}
