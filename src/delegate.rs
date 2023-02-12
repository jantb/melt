use std::collections::VecDeque;
use std::ops::RangeBounds;

use druid::text::RichTextBuilder;
use druid::{AppDelegate, Color, Command, DelegateCtx, Env, Handled, Selector, Target};
use jsonptr::{Pointer, ResolveMut};
use serde_json::Value;

use crate::data::{AppState, PointerState};
use crate::index::CommandMessage;

pub const SET_VIEW: Selector<String> = Selector::new("set_view");
pub const SEARCH: Selector<((String, String), bool)> = Selector::new("search");
pub const CHECK_CLICKED_FOR_POINTER: Selector<PointerState> = Selector::new("clicked");
pub const CHECK_CLICKED_FOR_POINTER_VIEW: Selector<PointerState> = Selector::new("clicked_view");
pub const CHANGE_SETTINGS: Selector<bool> = Selector::new("change_setting");
pub const SEARCH_RESULT: Selector = Selector::new("search_result");
pub const CLEAR_DB: Selector = Selector::new("clear_db");

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
                generate_pointers(&serde_json::from_str(&text.as_str()).unwrap())
                    .iter()
                    .for_each(|v| {
                        data.pointers.push_back(PointerState {
                            text: v.to_string(),
                            number: u64::MAX,
                            number_view: u64::MAX,
                            checked: false,
                            checked_view: false,
                        })
                    });
            }
            if data.view_column.is_empty() {
                data.view = parse_json(text.to_string().as_str());
            } else {
                let string1 = data.view_column.clone();
                let split = string1.split(" ").collect::<Vec<&str>>();
                let string2 = split
                    .iter()
                    .map(|s| {
                        let pointer = resolve_pointer(text, s);
                        parse_json(pointer.as_str())
                    })
                    .collect::<Vec<String>>()
                    .join(" ");
                data.view = string2
            }
            Handled::Yes
        } else if let Some(b) = cmd.get(CHANGE_SETTINGS) {
            data.settings = *b;
            Handled::Yes
        } else if let Some(_) = cmd.get(SEARCH_RESULT) {
            for x in data.items.clone() {
                let mut builder = RichTextBuilder::new();
                builder.push(x.view.as_str());
                if data.exact {
                    let ranges = find_all_ranges(x.view.as_str(), data.query.as_str());

                    for r in ranges {
                        if r.is_match {
                            builder
                                .add_attributes_for_range(r.range)
                                .text_color(Color::rgba8(255, 0, 0, 0));
                        }
                    }
                } else {
                    builder.push(x.view.as_str());
                }
            }

            Handled::Yes
        } else if let Some(pointer_state) = cmd.get(CHECK_CLICKED_FOR_POINTER) {
            data.pointers.iter_mut().for_each(|p| {
                if p.text == pointer_state.text {
                    p.checked = pointer_state.checked;
                    p.number = pointer_state.number;
                }
            });
            Handled::Yes
        } else if let Some(pointer_state) = cmd.get(CHECK_CLICKED_FOR_POINTER_VIEW) {
            data.pointers.iter_mut().for_each(|p| {
                if p.text == pointer_state.text {
                    p.checked_view = pointer_state.checked_view;
                    p.number_view = pointer_state.number_view;
                }
            });
            let mut vector = data.pointers.clone();
            vector.sort_by(|a, b| a.number_view.partial_cmp(&b.number_view).unwrap());
            data.view_column = vector
                .iter()
                .map(|s| s.clone().text)
                .collect::<Vec<String>>()
                .join(" ");
            Handled::Yes
        } else if let Some(_) = cmd.get(CLEAR_DB) {
            data.tx.send(CommandMessage::Clear).unwrap();
            Handled::Yes
        } else if let Some(q) = cmd.get(SEARCH) {
            let mut pointers = data.pointers.clone();
            pointers.sort_by(|a, b| a.number.partial_cmp(&b.number).unwrap());
            data.tx
                .send(CommandMessage::Filter(
                    q.0 .0.to_string(),
                    q.0 .1.to_string(),
                    q.1,
                    data.timelimit as u64,
                    data.viewlimit as usize,
                    pointers,
                ))
                .unwrap();
            Handled::Yes
        } else {
            Handled::No
        }
    }
}

fn resolve_pointer(text: &str, ps: &str) -> String {
    let mut json: Value = match serde_json::from_str(text) {
        Ok(json) => json,
        Err(_) => Value::Null,
    };
    let ptr = match Pointer::try_from(ps) {
        Ok(ptr) => ptr,
        Err(_) => Pointer::root(),
    };

    let string = match json.resolve_mut(&ptr) {
        Ok(v) => v,
        Err(_) => &Value::Null,
    };
    if string.is_string() {
        return string.as_str().unwrap().to_string();
    }
    if !string.is_null() {
        return string.to_string();
    }
    ps.to_string()
}

fn generate_pointers(json: &Value) -> Vec<String> {
    let mut pointers = vec![];
    let mut stack = VecDeque::new();
    stack.push_back(("".to_string(), json));

    while let Some((current_path, current_json)) = stack.pop_back() {
        match current_json {
            Value::Object(map) => {
                pointers.push(current_path.clone() + "/");
                for (key, value) in map {
                    let new_path = format!("{}/{}", current_path, key);
                    stack.push_back((new_path, value));
                }
            }
            Value::Array(arr) => {
                pointers.push(current_path.clone() + "/");
                for (i, value) in arr.iter().enumerate() {
                    let new_path = format!("{}/{}", current_path, i);
                    stack.push_back((new_path, value));
                }
            }
            _ => pointers.push(current_path),
        }
    }
    pointers
        .into_iter()
        .filter(|p| p != "/")
        .collect::<Vec<String>>()
}

struct MatchRange<R: RangeBounds<usize>> {
    range: R,
    is_match: bool,
}

fn find_all_ranges(s: &str, word: &str) -> Vec<MatchRange<std::ops::Range<usize>>> {
    let mut results = vec![];

    let mut start = 0;
    let mut i = 0;
    while let Some(j) = s[i..].find(word) {
        results.push(MatchRange {
            range: start..(i + j),
            is_match: false,
        });

        start = i + j;
        i = start + word.len();

        results.push(MatchRange {
            range: start..i,
            is_match: true,
        });
    }

    if start < s.len() {
        results.push(MatchRange {
            range: start..s.len(),
            is_match: false,
        });
    }

    results
}

fn parse_json(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(json) => serde_json::to_string_pretty(&json).unwrap(),
        Err(_) => s.to_string(),
    }
}
