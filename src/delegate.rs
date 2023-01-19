use std::collections::VecDeque;
use std::time::Instant;
use druid::{AppDelegate, Color, Command, DelegateCtx, Env, FontFamily, Handled, Selector, Target};
use druid::text::RichTextBuilder;
use jsonptr::{Pointer, ResolveMut};
use serde_json::Value;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;


use crate::data::{AppState, Item};
use crate::index::{CommandMessage, ResultMessage};

pub const SET_VIEW: Selector<String> = Selector::new("set_view");
pub const SEARCH: Selector<String> = Selector::new("search");
pub const ADD_COLUMN: Selector<String> = Selector::new("add_column");
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
            //InspiredGitHub
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
            data.pointers.clear();
            generate_pointers(&serde_json::from_str(&text.as_str()).unwrap()).iter().for_each(|v| data.pointers.push_front(v.clone()));
            data.view = builder.build();
            Handled::Yes
        } else if let Some(param) = cmd.get(ADD_COLUMN) {
            data.items.iter_mut().for_each(|item| {
                let mut json: Value = serde_json::from_str(&item.text).unwrap();
                let ptr = Pointer::try_from(param.as_str()).unwrap();
                item.pointers.push(json.resolve_mut(&ptr).unwrap().to_string())
            });
            data.items.iter_mut().for_each(|item| {
                item.view = item.pointers.join(" ")
            }
            );

            data.properties.push_front(param.clone());
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
            data.items.clear();
            if query.is_empty() { return Handled::Yes; };
            let start = Instant::now();
            data.tx.send(CommandMessage::FilterRegex(query.to_string())).unwrap();
            match data.rx.recv().unwrap() {
                ResultMessage::Messages(m) => {
                    let duration = start.elapsed();
                    data.query_time = format!("Query took {}ms with {} results", duration.as_millis().to_string(), m.len());
                    m.iter()
                        .for_each(|m| data.items.push_front(Item::new(m.value.as_str())))
                }
            }
            if data.properties.is_empty() {
                data.items.iter_mut().for_each(|item| {
                    item.view = item.text.to_string();
                });
            } else {
                data.items.iter_mut().for_each(|item| {
                    let mut json: Value = serde_json::from_str(&item.text).unwrap();
                    data.properties.iter().for_each(|f| {
                        let ptr = Pointer::try_from(f.as_str()).unwrap();
                        item.pointers.push(json.resolve_mut(&ptr).unwrap().to_string())
                    });
                });
                data.items.iter_mut().for_each(|item| {
                    item.view = item.pointers.join(" ")
                });
            }
            if data.view_column.is_empty() {

            }else{
                data.items.iter_mut().for_each(|item| {
                    let mut json: Value = serde_json::from_str(&item.text).unwrap();
                    let ptr = Pointer::try_from(data.view_column.as_str()).unwrap();
                    item.text = json.resolve_mut(&ptr).unwrap().to_string()
                });
            }
            Handled::Yes
        } else {
            println!("cmd forwarded: {:?}", cmd);
            Handled::No
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