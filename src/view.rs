use druid::{widget::TextBox, widget::{Button, Flex, Label, List}, Widget, WidgetExt, FontDescriptor, FontFamily, FontWeight, EventCtx, Event, Env};
use druid::keyboard_types::Key::Enter;
use druid::widget::{Checkbox, Container, Controller, Either, LineBreaking, Scroll, Split};

use crate::data::*;
use crate::delegate::{CHECK_CLICKED_FOR_POINTER, SEARCH, SET_VIEW_COLUMN};

fn new_search_textbox() -> impl Widget<AppState> {
    let new_search_textbox = TextBox::new()
        .with_placeholder("Filter messages")
        .expand_width()
        .lens(AppState::query)
        .controller(TakeFocus);

    Flex::row()
        .with_flex_child(new_search_textbox, 1.)
        .padding(8.0)
}

struct TakeFocus;

impl<W: Widget<AppState>> Controller<AppState, W> for TakeFocus {
    fn event(&mut self, child: &mut W, ctx: &mut EventCtx, event: &Event, data: &mut AppState, env: &Env) {
        if let Event::WindowConnected = event {
            ctx.request_focus();
        }
        if let Event::KeyUp(key) = event {
            if key.key == Enter {
                if trigram(data.query.as_str()).len() > 7 {
                    ctx.submit_command(SEARCH.with(data.query.to_string()));
                }
            }
        }

        child.event(ctx, event, data, env)
    }
}

fn documents() -> impl Widget<Item> {
    let font = FontDescriptor::new(FontFamily::MONOSPACE)
        .with_weight(FontWeight::BOLD);
    let label = Label::raw().with_font(font).with_line_break_mode(LineBreaking::Clip)
        .expand_width().lens(Item::view).on_click(Item::click_view);
    let copy = Button::new("Copy").on_click(Item::click_copy);

    Flex::row()
        .with_flex_child(label, 1.)
        .with_child(copy)
}

pub fn build_ui() -> impl Widget<AppState> {
    let items = List::new(documents).lens(AppState::items);
    let flex = Flex::column()
        .with_child(Button::new("Settings").on_click(|_, data: &mut AppState, _env| {
            data.settings = !data.settings;
        }).align_left())
        .with_child(Label::raw().lens(AppState::query_time))
        .with_child(Label::raw().lens(AppState::count))
        .with_child(Label::raw().lens(AppState::size))
        .with_child(new_search_textbox())
        .with_flex_child(Scroll::new(items).vertical(), 1.);

    let container = Container::new(
        Split::columns(
            flex, Scroll::new(Label::raw().with_line_break_mode(LineBreaking::WordWrap).lens(AppState::view)).vertical(),
        ).min_size(300., 700.).split_point(0.2).draggable(true).solid_bar(true)
    );

    let flex_settings = Flex::column()
        .with_child(
            Button::new("Close settings").on_click(|_, data: &mut AppState, _env| {
                data.settings = !data.settings;
            }).align_left())
        .with_child(Scroll::new(List::new(|| {
            Flex::row()
                .with_child(Checkbox::new("").lens(PointerState::checked)
                    .on_click(|ctx, pointer_state, _env| {
                        pointer_state.checked = !pointer_state.checked;
                        let state = pointer_state.clone();
                        ctx.submit_command(CHECK_CLICKED_FOR_POINTER.with(state));
                    })
                )
                .with_child(Label::new(|item: &PointerState, _env: &_| format!("{}", item.text))
                )
        }))
            .vertical()
            .lens(AppState::pointers).align_left())
        .with_child(Label::new("Select view tag:").padding(8.0).align_left())
        .with_child(Scroll::new(List::new(|| {
            Label::new(|item: &PointerState, _env: &_| format!("{}", item.text)).on_click(|ctx, item, _env| {
                ctx.submit_command(SET_VIEW_COLUMN.with(item.text.to_string()));
            })
        }))
            .vertical()
            .lens(AppState::pointers).align_left());

    let either = Either::new(
        |data, _env| data.settings,
        flex_settings
        ,
        container,
    );
    either
}

pub fn trigram(word: &str) -> Vec<String> {
    let mut word = word.to_string();
    word.make_ascii_lowercase();
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 3 {
        return vec![];
    }

    let mut trigrams = Vec::with_capacity(chars.len() - 2);
    let mut seen = std::collections::HashSet::new();

    for i in 1..chars.len() - 1 {
        let trigram = &chars[i - 1..i + 2];
        if !seen.contains(trigram) {
            seen.insert(trigram);
            trigrams.push(trigram.into_iter().collect());
        }
    }

    trigrams
}
