use druid::{AppDelegate, Color, Command, DelegateCtx, Env, Handled, Selector, Target};
use druid::text::RichTextBuilder;
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

            data.view = builder.build();
            Handled::Yes
        } else {
            println!("cmd forwarded: {:?}", cmd);
            Handled::No
        }
    }
}