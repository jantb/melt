use druid::widget::{
    Checkbox, Container, Controller, Either, LineBreaking, Painter, Scroll, Slider, Split,
};
use druid::{
    theme,
    widget::TextBox,
    widget::{Button, Flex, Label, List},
    Color, Env, Event, EventCtx, FontDescriptor, FontFamily, RenderContext, Widget, WidgetExt,
};

use crate::data::*;
use crate::delegate::{
    CHANGE_SETTINGS, CHECK_CLICKED_FOR_POINTER, CHECK_CLICKED_FOR_POINTER_VIEW, CLEAR_DB, SEARCH,
};
use crate::index::CommandMessage;
use crate::GLOBAL_STATE;

fn new_search_textbox() -> impl Widget<AppState> {
    let new_search_textbox = TextBox::new()
        .with_placeholder("Filter documents")
        .expand_width()
        .lens(AppState::query)
        .controller(SearchController);
    let new_search_textbox_neq = TextBox::new()
        .with_placeholder("Filter away documents")
        .expand_width()
        .lens(AppState::not_query)
        .controller(ControllerForNegSearch);

    Flex::row()
        .with_flex_child(new_search_textbox.padding(5.), 1.)
        .with_flex_child(new_search_textbox_neq.padding(5.), 1.)
        .with_child(Checkbox::new("Exact").lens(AppState::exact))
        .on_click(|ctx, data: &mut AppState, _env| {
            ctx.submit_command(SEARCH.with((
                (data.query.to_string(), data.not_query.to_string()),
                data.exact,
            )));
            ctx.request_update()
        })
}

struct SearchController;

struct ControllerForNegSearch;

impl<W: Widget<AppState>> Controller<AppState, W> for SearchController {
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut AppState,
        env: &Env,
    ) {
        if let Event::WindowConnected = event {
            ctx.request_focus();
        }

        if let Event::KeyUp(_) = event {
            GLOBAL_STATE.lock().unwrap().query = data.query.to_string();
            GLOBAL_STATE.lock().unwrap().query_neg = data.not_query.to_string();
            ctx.submit_command(SEARCH.with((
                (data.query.to_string(), data.not_query.to_string()),
                data.exact,
            )));
        }

        child.event(ctx, event, data, env)
    }
}

impl<W: Widget<AppState>> Controller<AppState, W> for ControllerForNegSearch {
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut AppState,
        env: &Env,
    ) {
        if let Event::KeyUp(_) = event {
            ctx.submit_command(SEARCH.with((
                (data.query.to_string(), data.not_query.to_string()),
                data.exact,
            )));
        }

        child.event(ctx, event, data, env)
    }
}

fn documents() -> impl Widget<Item> {
    let painter = Painter::new(|ctx, _, env| {
        let bounds = ctx.size().to_rect();

        ctx.fill(bounds, &env.get(theme::BACKGROUND_LIGHT));

        if ctx.is_hot() {
            ctx.stroke(bounds.inset(-0.5), &Color::WHITE, 1.0);
        }

        if ctx.is_active() {
            ctx.fill(bounds, &Color::rgb8(0x71, 0x71, 0x71));
        }
    });

    let label = Label::dynamic(|value: &Item, _| format!("{}", value.view))
        .with_font(FontDescriptor::new(FontFamily::MONOSPACE))
        .with_line_break_mode(LineBreaking::Clip)
        .expand_width()
        .background(painter)
        .on_click(Item::click_view);

    Flex::row().with_flex_child(label, 1.)
}

pub fn build_ui() -> impl Widget<AppState> {
    let items = List::new(documents).lens(AppState::items);
    let flex = Flex::column()
        .with_child(
            Flex::row()
                .with_child(
                    Button::new("Settings")
                        .on_click(|ctx, data: &mut AppState, _env| {
                            ctx.submit_command(CHANGE_SETTINGS.with(!data.settings));
                            ctx.request_update()
                        })
                        .align_left(),
                )
                .with_child(
                    Button::new("Pods")
                        .on_click(|ctx, data: &mut AppState, _env| {
                            data.tx.send(CommandMessage::Pod).unwrap();
                            ctx.set_disabled(true);
                        })
                        .align_left(),
                )
                .with_child(
                    Button::new("Clear")
                        .on_click(|ctx, _: &mut AppState, _env| {
                            ctx.submit_command(CLEAR_DB);
                            ctx.request_update()
                        })
                        .align_right(),
                ),
        )
        .with_child(
            Label::raw()
                .with_font(FontDescriptor::new(FontFamily::MONOSPACE))
                .lens(AppState::query_time)
                .align_left(),
        )
        .with_child(
            Label::raw()
                .with_font(FontDescriptor::new(FontFamily::MONOSPACE))
                .lens(AppState::count)
                .align_left(),
        )
        .with_child(
            Label::raw()
                .with_font(FontDescriptor::new(FontFamily::MONOSPACE))
                .lens(AppState::size)
                .align_left(),
        )
        .with_child(
            Label::raw()
                .with_font(FontDescriptor::new(FontFamily::MONOSPACE))
                .lens(AppState::indexed_data_in_bytes_string)
                .align_left(),
        )
        .with_child(
            Label::dynamic(|value: &AppState, _| {
                format!("Time limit   {:?} ms", value.timelimit as u64)
            })
            .with_font(FontDescriptor::new(FontFamily::MONOSPACE))
            .align_left(),
        )
        .with_child(
            Slider::new()
                .with_range(10.0, 10000.0)
                .with_step(1.0)
                .lens(AppState::timelimit)
                .align_left(),
        )
        .with_child(
            Label::dynamic(|value: &AppState, _| {
                format!("View limit   {:?}", value.viewlimit as u64)
            })
            .with_font(FontDescriptor::new(FontFamily::MONOSPACE))
            .align_left(),
        )
        .with_child(
            Slider::new()
                .with_range(100.0, 50000.0)
                .with_step(1.0)
                .lens(AppState::viewlimit)
                .align_left(),
        )
        .with_child(new_search_textbox())
        .with_flex_child(Scroll::new(items).vertical(), 1.);

    let container = Container::new(
        Split::columns(
            flex,
            Scroll::new(
                TextBox::multiline()
                    .with_font(FontDescriptor::new(FontFamily::MONOSPACE))
                    .with_line_wrapping(true)
                    .lens(AppState::view)
                    .expand_width(),
            )
            .vertical(),
        )
        .split_point(0.2)
        .draggable(true)
        .solid_bar(true),
    );

    let flex_settings = Flex::column()
        .with_child(
            Button::new("Close settings")
                .on_click(|ctx, data: &mut AppState, _env| {
                    ctx.submit_command(CHANGE_SETTINGS.with(!data.settings));
                    ctx.request_update();
                })
                .align_left(),
        )
        .with_child(
            Button::new("Clear settings")
                .on_click(|ctx, data: &mut AppState, _env| {
                    data.pointers.clear();
                    data.view_column = "".to_string();
                    ctx.request_update();
                })
                .align_left(),
        )
        .with_child(
            Scroll::new(List::new(|| {
                Flex::row()
                    .with_child(Checkbox::new("").lens(PointerState::checked).on_click(
                        |ctx, pointer_state, _env| {
                            pointer_state.checked = !pointer_state.checked;
                            if pointer_state.checked {
                                GLOBAL_STATE.lock().unwrap().label_num += 1;
                                pointer_state.number = GLOBAL_STATE.lock().unwrap().label_num;
                            } else {
                                pointer_state.number = u64::MAX;
                            }
                            ctx.submit_command(
                                CHECK_CLICKED_FOR_POINTER.with(pointer_state.clone()),
                            );
                        },
                    ))
                    .with_child(Label::new(|item: &PointerState, _env: &_| {
                        format!("{}", item.text)
                    }))
            }))
            .vertical()
            .lens(AppState::pointers)
            .align_left(),
        )
        .with_child(Label::new("Select view tag:").padding(8.0).align_left())
        .with_child(
            Scroll::new(List::new(|| {
                Flex::row()
                    .with_child(Checkbox::new("").lens(PointerState::checked_view).on_click(
                        |ctx, pointer_state, _env| {
                            pointer_state.checked_view = !pointer_state.checked_view;
                            if pointer_state.checked_view {
                                GLOBAL_STATE.lock().unwrap().label_num += 1;
                                pointer_state.number = GLOBAL_STATE.lock().unwrap().label_num;
                            } else {
                                pointer_state.number = u64::MAX;
                            }
                            ctx.submit_command(
                                CHECK_CLICKED_FOR_POINTER_VIEW.with(pointer_state.clone()),
                            );
                        },
                    ))
                    .with_child(Label::new(|item: &PointerState, _env: &_| {
                        format!("{}", item.text)
                    }))
            }))
            .vertical()
            .lens(AppState::pointers)
            .align_left(),
        );

    let either = Either::new(
        |data: &AppState, _env| data.settings,
        flex_settings,
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
