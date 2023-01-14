use std::time:: Instant;
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
use crate::delegate::SET_VIEW;

use crate::index::{CommandMessage, ResultMessage};

#[derive(Clone, Data, Lens)]
pub struct AppState {
    pub new_todo: String,
    pub query: String,
    pub items: Vector<Item>,
    pub view: RichText,
    pub query_time: String,
    pub count : String,
    #[data(ignore)]
    pub tx: Sender<CommandMessage>,
    #[data(ignore)]
    pub rx: Receiver<ResultMessage>,
}

impl AppState {
    fn search(&mut self) {
        self.items.clear();
        if self.query.is_empty() { return; };
        let start = Instant::now();

        self.tx.send(CommandMessage::FilterRegex(self.query.to_string())).unwrap();
        match self.rx.recv().unwrap() {
            ResultMessage::Messages(m) => {
                let duration = start.elapsed();
                self.query_time = format!("Query took {}ms with {} results",duration.as_millis().to_string(), m.len());
                m.iter()
                    .for_each(| m| self.items.push_front(Item::new(m.value.as_str())))
            }
        }
    }

    pub fn click_search(_ctx: &mut EventCtx, data: &mut Self, _env: &Env) {
        data.search();
    }
}

#[derive(Clone, Data, Lens)]
pub struct Item {
    #[data(same_fn = "PartialEq::eq")]
    pub id: Uuid,
    pub done: bool,
    pub text: String,
}

impl Item {
    pub fn new(text: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            done: false,
            text: text.into(),
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
