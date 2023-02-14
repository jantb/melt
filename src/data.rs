use std::fs;

use crossbeam_channel::Sender;
use druid::im::Vector;
use druid::piet::{PietTextLayoutBuilder, TextStorage as PietTextStorage};
use druid::text::{RichText, TextStorage};
use druid::Data;
use druid::Env;
use druid::EventCtx;
use druid::Lens;
use serde::{Deserialize, Serialize};

use crate::delegate::{SEARCH, SET_VIEW};
use crate::index::CommandMessage;
use crate::GLOBAL_STATE;

#[derive(Clone, Data, Lens)]
pub struct AppState {
    pub query: String,
    pub timelimit: f64,
    pub viewlimit: f64,
    pub not_query: String,
    pub exact: bool,
    pub items: Vector<Item>,
    pub items_rich: Vector<ItemRich>,
    pub view: String,
    pub pointers: Vector<PointerState>,
    pub pointers_view: Vector<PointerState>,
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
    pub sort: bool,
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
            indexed_data_in_bytes: self.indexed_data_in_bytes,
            pointer_state: self
                .pointers
                .iter()
                .map(|p| p.clone())
                .collect::<Vec<PointerState>>(),
            pointer_state_view: self
                .pointers
                .iter()
                .map(|p| p.clone())
                .collect::<Vec<PointerState>>(),
            sort: GLOBAL_STATE.lock().unwrap().sort.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SerializableParameters {
    pub pointer_state: Vec<PointerState>,
    pub pointer_state_view: Vec<PointerState>,
    pub indexed_data_in_bytes: u64,
    pub sort: String,
}

impl Default for SerializableParameters {
    fn default() -> Self {
        SerializableParameters {
            indexed_data_in_bytes: 0,
            pointer_state: vec![],
            pointer_state_view: vec![],
            sort: "".to_string(),
        }
    }
}

#[derive(Clone, Data, Lens, Serialize, Deserialize)]
pub struct PointerState {
    pub text: String,
    pub number: u64,
    pub checked: bool,
    pub checked_sort: bool,
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
    pub text: String,
    #[data(ignore)]
    pub pointers: Vec<String>,
    #[data(ignore)]
    pub pointer_states: Vec<PointerStateItem>,
    pub view: String,
}

#[derive(Clone, Data, Lens)]
pub struct ItemRich {
    pub text: String,
    #[data(ignore)]
    pub pointers: Vec<String>,
    #[data(ignore)]
    pub pointer_states: Vec<PointerStateItem>,
    pub view: RichText,
}

impl PietTextStorage for ItemRich {
    fn as_str(&self) -> &str {
        self.view.as_str()
    }
}

impl TextStorage for ItemRich {
    fn add_attributes(&self, builder: PietTextLayoutBuilder, env: &Env) -> PietTextLayoutBuilder {
        self.view.add_attributes(builder, env)
    }
}

impl Item {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            pointers: Default::default(),
            pointer_states: vec![],
            view: "".to_string(),
        }
    }

    pub fn click_view(ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        ctx.submit_command(SET_VIEW.with(data.text.to_string()));
    }
}
impl ItemRich {
    pub fn click_view(ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        ctx.submit_command(SET_VIEW.with(data.text.as_str().to_string()));
    }
}
