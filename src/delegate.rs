use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::Instant;
use druid::{AppDelegate, Color, Command, DelegateCtx, Env, FontFamily, Handled, Selector, Target};
use druid::im::Vector;
use druid::text::RichTextBuilder;
use jsonptr::{Pointer, ResolveMut};
use serde_json::Value;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;


use crate::data::{AppState, Item, PointerState};
use crate::GLOBAL_COUNT;
use crate::index::{CommandMessage, ResultMessage};

pub const SET_VIEW: Selector<String> = Selector::new("set_view");
pub const SEARCH: Selector<String> = Selector::new("search");
pub const CHECK_CLICKED_FOR_POINTER: Selector<PointerState> = Selector::new("clicked");
pub const SET_VIEW_COLUMN: Selector<String> = Selector::new("set_view_column");

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
            let ps = SyntaxSet::load_defaults_newlines();
            let ts = ThemeSet::load_defaults();

            let syntax = ps.find_syntax_by_extension("json").unwrap();
            // InspiredGitHub
            // Solarized (dark)
            // Solarized (light)
            // base16-eighties.dark
            // base16-mocha.dark
            // base16-ocean.dark
            // base16-ocean.light
            let mut h = HighlightLines::new(syntax, &ts.themes["Solarized (dark)"]);

            let mut builder = RichTextBuilder::new();
            for line in LinesWithEndings::from(text.as_str()) { // LinesWithEndings enables use of newlines mode
                let ranges: Vec<(Style, &str)> = h.highlight_line(line, &ps).unwrap();
                for x in ranges {
                    let foreground_color = x.0.foreground;

                    builder.push(x.1)
                        .font_family(FontFamily::MONOSPACE)
                        .text_color(Color::rgba8(foreground_color.r, foreground_color.g, foreground_color.b, foreground_color.a));
                }
            }
            if data.pointers.is_empty() {
                generate_pointers(&serde_json::from_str(&text.as_str()).unwrap()).iter().for_each(|v| data.pointers.push_front(PointerState {
                    text: v.to_string(),
                    checked: false,
                }));
            }
            data.view = builder.build();
            Handled::Yes
        } else if let Some(pointer_state) = cmd.get(CHECK_CLICKED_FOR_POINTER) {
            data.pointers.iter_mut().for_each(|p| if p.text == pointer_state.text { p.checked = pointer_state.checked });
            Handled::Yes
        } else if let Some(param) = cmd.get(SET_VIEW_COLUMN) {
            data.items.iter_mut().for_each(|item| {
                let mut json: Value = serde_json::from_str(&item.text).unwrap();
                let ptr = Pointer::try_from(param.as_str()).unwrap();
                item.text = json.resolve_mut(&ptr).unwrap().to_string()
            });

            data.view_column = param.to_string();
            Handled::Yes
        } else if let Some(query) = cmd.get(SEARCH) {
            data.count_from_index = GLOBAL_COUNT.load(Ordering::SeqCst);
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
                for (key, value) in map {
                    let new_path = format!("{}/{}", current_path, key);
                    stack.push_back((new_path, value));
                }
            }
            Value::Array(arr) => {
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