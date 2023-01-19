use druid::{widget::TextBox, widget::{Button, Flex, Label, List}, Widget, WidgetExt};
use druid::widget::{Container, Either, LineBreaking, Scroll, Split};

use crate::data::*;

fn new_search_textbox() -> impl Widget<AppState> {
    let new_search_textbox = TextBox::new()
        .with_placeholder("Filter messages")
        .expand_width()
        .lens(AppState::query);

    let search_button = Button::new("Search").on_click(AppState::click_search);

    Flex::row()
        .with_flex_child(new_search_textbox, 1.)
        .with_child(search_button)
        .padding(8.0)
}

fn documents() -> impl Widget<Item> {
    let label = Label::raw().with_line_break_mode(LineBreaking::Clip)
        .expand_width().lens(Item::text).on_click(Item::click_view);
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
            Label::new(|item: &String, _env: &_| format!("{item}"))
        }))
            .vertical()
            .lens(AppState::pointers));


    let either = Either::new(
        |data, _env| data.settings,
        flex_settings
        ,
        container,
    );
    either
}


