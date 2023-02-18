use std::collections::VecDeque;

use druid::im::Vector;
use druid::text::RichTextBuilder;
use druid::{AppDelegate, Color, Command, DelegateCtx, Env, FontWeight, Handled, Selector, Target};
use jsonptr::{Pointer, ResolveMut};
use serde_json::Value;

use crate::data::{AppState, ItemRich, PointerState};
use crate::index::CommandMessage;
use crate::GLOBAL_STATE;

pub const SET_VIEW: Selector<String> = Selector::new("set_view");
pub const SEARCH: Selector<((String, String), bool)> = Selector::new("search");
pub const CHECK_CLICKED_FOR_POINTER: Selector<PointerState> = Selector::new("clicked");
pub const CHECK_CLICKED_FOR_POINTER_SORT: Selector<PointerState> = Selector::new("clicked_sort");
pub const CHECK_CLICKED_FOR_POINTER_VIEW: Selector<PointerState> = Selector::new("clicked_view");
pub const CHANGE_SETTINGS: Selector<bool> = Selector::new("change_setting");
pub const SEARCH_RESULT: Selector = Selector::new("search_result");
pub const CLEAR_DB: Selector = Selector::new("clear_db");
pub const TAIL: Selector<bool> = Selector::new("tail");

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
                            checked: false,
                            checked_sort: false,
                        });
                        data.pointers_view.push_back(PointerState {
                            text: v.to_string(),
                            number: u64::MAX,
                            checked: false,
                            checked_sort: false,
                        });
                    });
            }
            if data.pointers_view.iter().filter(|p| p.checked).count() == 0 {
                data.view = parse_json(&text.to_string())
            } else {
                data.pointers_view
                    .sort_by(|left, right| left.number.partial_cmp(&right.number).unwrap());

                data.view = data
                    .pointers_view
                    .iter()
                    .filter(|p| p.checked)
                    .map(|p| resolve_pointer(&text, &p.text))
                    .collect::<Vec<String>>()
                    .join(" ")
            }
            Handled::Yes
        } else if let Some(b) = cmd.get(CHANGE_SETTINGS) {
            data.settings = *b;
            Handled::Yes
        } else if let Some(_) = cmd.get(SEARCH_RESULT) {
            let mut vec1 = vec![];
            for x in data.items.clone() {
                let mut builder = RichTextBuilder::new();
                if data.exact && !data.query.is_empty() {
                    let ranges =
                        find_all_ranges(x.view.as_str(), vec![data.query.as_str()].as_slice());
                    Self::highligt(&mut builder, ranges)
                } else if !data.query.is_empty() {
                    let ranges = find_all_ranges(
                        x.view.as_str(),
                        data.query
                            .split_whitespace()
                            .collect::<Vec<&str>>()
                            .as_slice(),
                    );
                    Self::highligt(&mut builder, ranges)
                } else {
                    builder.push(x.view.as_str());
                }
                vec1.push(ItemRich {
                    text: x.text,
                    pointers: x.pointers,
                    pointer_states: x.pointer_states,
                    view: builder.build(),
                })
            }
            data.items_rich = Vector::from(vec1);
            Handled::Yes
        } else if let Some(pointer_state) = cmd.get(CHECK_CLICKED_FOR_POINTER) {
            data.pointers.iter_mut().for_each(|p| {
                if p.text == pointer_state.text {
                    p.checked = pointer_state.checked;
                    p.number = pointer_state.number;
                }
            });
            Handled::Yes
        } else if let Some(tail) = cmd.get(TAIL) {
            GLOBAL_STATE.lock().unwrap().tail = *tail;
            Handled::Yes
        } else if let Some(pointer_state) = cmd.get(CHECK_CLICKED_FOR_POINTER_SORT) {
            data.pointers.iter_mut().for_each(|p| {
                if p.text == pointer_state.text {
                    p.checked_sort = pointer_state.checked_sort;
                } else {
                    p.checked_sort = false;
                }
            });
            GLOBAL_STATE.lock().unwrap().sort = data
                .pointers
                .iter()
                .filter(|p| p.checked_sort)
                .map(|p| p.text.to_string())
                .last()
                .unwrap_or("".to_string());
            Handled::Yes
        } else if let Some(pointer_state) = cmd.get(CHECK_CLICKED_FOR_POINTER_VIEW) {
            data.pointers_view.iter_mut().for_each(|p| {
                if p.text == pointer_state.text {
                    p.checked = pointer_state.checked;
                    p.number = pointer_state.number;
                }
            });
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

impl Delegate {
    fn highligt(builder: &mut RichTextBuilder, ranges: Vec<MatchRange>) {
        for r in ranges {
            if r.is_match {
                builder
                    .push(
                        String::from_utf8_lossy(r.string.as_slice())
                            .to_string()
                            .as_str(),
                    )
                    .weight(FontWeight::new(1000))
                    .text_color(Color::rgb8(255, 180, 90));
            } else {
                builder
                    .push(
                        String::from_utf8_lossy(r.string.as_slice())
                            .to_string()
                            .as_str(),
                    )
                    .weight(FontWeight::THIN)
                    .text_color(Color::rgb8(150, 150, 150));
            }
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
struct MatchRange {
    string: Vec<u8>,
    is_match: bool,
}

fn find_all_ranges(s: &str, words: &[&str]) -> Vec<MatchRange> {
    let mut results = vec![];
    let mut zero_array: Vec<_> = (0..s.len()).map(|_| false).collect();
    let s = s.to_lowercase();
    for word in words {
        for (start, end) in s
            .match_indices(&word.to_lowercase())
            .map(|(start, matched)| (start, start + matched.len()))
        {
            for x in start..end {
                zero_array[x] = true
            }
        }
    }

    let mut last_was_match = false;
    let mut current_range = MatchRange {
        string: vec![],
        is_match: false,
    };
    let string_bytes = s.as_bytes();
    for (n, &is_match) in zero_array.iter().enumerate() {
        let c = string_bytes[n];
        if is_match != last_was_match {
            results.push(current_range);
            current_range = MatchRange {
                string: vec![c],
                is_match,
            };
            last_was_match = is_match;
        } else {
            current_range.string.push(c);
        }
    }
    results.push(current_range);

    results
}

fn parse_json(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(json) => serde_json::to_string_pretty(&json).unwrap(),
        Err(_) => s.to_string(),
    }
}
