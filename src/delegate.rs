use std::collections::VecDeque;
use std::time::Instant;
use druid::{AppDelegate, Command, DelegateCtx, Env, Handled, Selector, Target};
use druid::im::Vector;
use jsonptr::{Pointer, ResolveMut};
use serde_json::Value;


use crate::data::{AppState, Item, PointerState};
use crate::index::{CommandMessage, ResultMessage};

pub const SET_VIEW: Selector<String> = Selector::new("set_view");
pub const SEARCH: Selector<String> = Selector::new("search");
pub const CHECK_CLICKED_FOR_POINTER: Selector<PointerState> = Selector::new("clicked");
pub const SET_VIEW_COLUMN: Selector<String> = Selector::new("set_view_column");
pub const CHANGE_SETTINGS: Selector<bool> = Selector::new("set_view_column");

pub struct Delegate;

impl AppDelegate<AppState> for Delegate {
    fn command(
        &mut self,
        _ctx: &mut DelegateCtx,
        _target: Target,
        cmd: &Command,
        data: &mut AppState,
        _env: &Env,
    ) -> Handled {
        if let Some(text) = cmd.get(SET_VIEW) {
            if data.pointers.is_empty() {
                generate_pointers(&serde_json::from_str(&text.as_str()).unwrap()).iter().for_each(|v| data.pointers.push_front(PointerState {
                    text: v.to_string(),
                    checked: false,
                }));
            }
            data.view = text.to_string();
            Handled::Yes
        } else if let Some(b) = cmd.get(CHANGE_SETTINGS) {
            data.settings = *b;
            Handled::Yes
        } else if let Some(pointer_state) = cmd.get(CHECK_CLICKED_FOR_POINTER) {
            data.pointers.iter_mut().for_each(|p| if p.text == pointer_state.text { p.checked = pointer_state.checked });
            Handled::Yes
        } else if let Some(param) = cmd.get(SET_VIEW_COLUMN) {
            data.view_column = param.to_string();
            Handled::Yes
        } else if let Some(query) = cmd.get(SEARCH) {
           data.items.clear();
            if query.is_empty() { return Handled::Yes; };
            let start = Instant::now();
            data.tx.send(CommandMessage::FilterRegex(query.to_string())).unwrap();
            match data.rx.recv().unwrap() {
                ResultMessage::Messages(m) => {
                    let duration = start.elapsed();
                    data.query_time = format!("Query took {}ms with {} results", duration.as_millis().to_string(), m.len());
                    m.iter()
                        .for_each(|m| data.items.push_front(Item::new(m.as_str())))
                }
            }

            Self::sort_and_resolve(data);

            Handled::Yes
        } else {
            println!("cmd forwarded: {:?}", cmd);
            Handled::No
        }
    }
}

impl Delegate {
    fn resolve_pointers(data: &mut AppState) -> bool {
        let mut empty_pointer: bool = true;
        data.pointers.iter().for_each(|ps| {
            if ps.checked {
                data.items.iter_mut().for_each(|item| {
                    let mut json: Value = match serde_json::from_str(&item.text) {
                        Ok(json) => json,
                        Err(_) => {
                            Value::Null
                        }
                    };
                    let ptr = match Pointer::try_from(ps.text.as_str()) {
                        Ok(ptr) => ptr,
                        Err(_) => {
                            Pointer::root()
                        }
                    };

                    let string = match json.resolve_mut(&ptr) {
                        Ok(v) => { v }
                        Err(_) => { &Value::Null }
                    };
                    if string != &Value::Null {
                        item.pointers.push(string.to_string());
                    }
                });
                empty_pointer = false;
            }
        });
        empty_pointer.clone()
    }

    fn sort_and_resolve(data: &mut AppState) {
        if !Self::resolve_pointers(data) {
            let mut vec1 = vec![];
            data.items.iter().for_each(|v| vec1.push(v.clone()));

            vec1.sort_by(|left, right| {
                right.pointers.first().unwrap_or(&"".to_string())
                    .cmp(left.pointers.first().unwrap_or(&"".to_string()))
            });
            let mut sorted_vec: Vector<Item> = Vector::new();
            vec1.iter().for_each(|v| sorted_vec.push_back(v.clone()));
            data.items = sorted_vec;
            data.items.iter_mut().for_each(|item| {
                item.view = item.pointers.join(" ")
            }
            );
        } else {
            data.items.iter_mut().for_each(|i| i.view = i.text.to_string())
        }
    }
}

fn generate_pointers(json: &Value) -> Vec<String> {
    let mut pointers = vec![];
    let mut stack = VecDeque::new();
    stack.push_back(("".to_string(), json));

    while let Some((current_path, current_json)) = stack.pop_back() {
        match current_json {
            Value::Object(map) => {
                pointers.push(current_path.clone());
                for (key, value) in map {
                    let new_path = format!("{}/{}", current_path, key);
                    stack.push_back((new_path, value));
                }
            }
            Value::Array(arr) => {
                pointers.push(current_path.clone());
                for (i, value) in arr.iter().enumerate() {
                    let new_path = format!("{}/{}", current_path, i);
                    stack.push_back((new_path, value));
                }
            }
            _ => pointers.push(current_path),
        }
    }
    pointers
}