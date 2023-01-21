use clipboard::{ClipboardContext, ClipboardProvider};
use crossbeam_channel::{Receiver, Sender};
use druid::Lens;
use druid::EventCtx;
use druid::Env;
use druid::Data;
use druid::im::Vector;
use druid::text::RichText;
use serde_json::Value;
use uuid:: Uuid;
use crate::delegate::{SEARCH, SET_VIEW};

use crate::index::{CommandMessage, ResultMessage};
#[derive(Clone, Data, Lens)]
pub struct AppState {
    pub new_todo: String,
    pub query: String,
    pub items: Vector<Item>,
    pub view: RichText,
    pub pointers: Vector<String>,
    pub query_time: String,
    pub count : String,
    #[data(ignore)]
    pub count_from_index : usize,
    pub settings : bool,
    pub properties: Vector<String>,
    pub view_column: String,


    #[data(ignore)]
    pub tx: Sender<CommandMessage>,
    #[data(ignore)]
    pub rx: Receiver<ResultMessage>,
}

impl AppState {
    pub fn click_search(ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        ctx.submit_command(SEARCH.with(data.query.to_string()));
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
    pub view: String,
}

impl Item {
    pub fn new(text: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            done: false,
            text: text.into(),
            pointers: Default::default(),
            view: "".to_string(),
        }
    }
    pub fn click_copy(_ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        let mut ctx: ClipboardContext = ClipboardProvider::new().unwrap();
        ctx.set_contents(data.text.clone().to_string()).unwrap();
    }

    pub fn click_view(ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        let json_value: Value = serde_json::from_str(&data.text).unwrap();
        let pretty_json_string = serde_json::to_string_pretty(&json_value).unwrap();
        ctx.submit_command(SET_VIEW.with(pretty_json_string));
    }
}
