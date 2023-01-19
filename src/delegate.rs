use std::collections::VecDeque;
use druid::{AppDelegate, Color, Command, DelegateCtx, Env, Handled, Selector, Target};
use druid::text::RichTextBuilder;
use serde_json::Value;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;


use crate::data::AppState;

pub const SET_VIEW: Selector<String> = Selector::new("set_view");

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
                        .text_color(Color::rgba8(foreground_color.r, foreground_color.g, foreground_color.b, foreground_color.a));
                }
            }
            data.pointers.clear();
            generate_pointers(&serde_json::from_str(&text.as_str()).unwrap()).iter().for_each(|v| data.pointers.push_front(v.clone()));
            data.view = builder.build();
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