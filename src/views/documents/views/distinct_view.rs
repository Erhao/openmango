//! Distinct field-value query view.
//!
//! Renders a small form (field + optional filter) and a list of distinct
//! values returned by the server.

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;

use crate::components::Button;
use crate::helpers::format_number;
use crate::state::{AppCommands, AppState, DistinctState, SessionKey};
use crate::theme::{fonts, spacing};
use crate::views::documents::CollectionView;

/// Render the Distinct sub-tab body.
#[allow(clippy::too_many_arguments)]
pub fn render_distinct_panel(
    distinct: DistinctState,
    field_state: Option<Entity<InputState>>,
    filter_state: Option<Entity<InputState>>,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let app = &*cx;
    let theme = app.theme();
    let has_session = session_key.is_some();
    let is_loading = distinct.loading;

    let toolbar = render_toolbar(
        &distinct,
        field_state,
        filter_state,
        session_key.clone(),
        state.clone(),
        is_loading,
        has_session,
        theme.border,
        theme.muted_foreground,
    );

    let body = render_body(&distinct, theme.muted_foreground, theme.danger_foreground, app);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .child(toolbar)
        .child(body)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_toolbar(
    distinct: &DistinctState,
    field_state: Option<Entity<InputState>>,
    filter_state: Option<Entity<InputState>>,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    is_loading: bool,
    has_session: bool,
    border: Hsla,
    muted_fg: Hsla,
) -> Div {
    let field_input = match field_state.clone() {
        Some(input) => Input::new(&input)
            .appearance(true)
            .bordered(true)
            .focus_bordered(true)
            .small()
            .into_any_element(),
        None => placeholder_input("Field (e.g. status)", border, muted_fg),
    };

    let filter_input = match filter_state.clone() {
        Some(input) => Input::new(&input)
            .appearance(true)
            .bordered(true)
            .focus_bordered(true)
            .small()
            .into_any_element(),
        None => placeholder_input("Filter (optional, JSON)", border, muted_fg),
    };

    let run_state = state.clone();
    let run_session = session_key.clone();
    let run_field = field_state.clone();
    let run_filter = filter_state.clone();

    let row = div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .px(spacing::lg())
        .py(spacing::sm())
        .border_b_1()
        .border_color(border)
        .child(div().w(px(60.0)).text_xs().text_color(muted_fg).child("Field"))
        .child(div().flex_1().min_w(px(160.0)).child(field_input))
        .child(div().w(px(60.0)).text_xs().text_color(muted_fg).child("Filter"))
        .child(div().flex_1().min_w(px(160.0)).child(filter_input))
        .child(
            Button::new("distinct-run")
                .primary()
                .compact()
                .label(if is_loading { "Running..." } else { "Run" })
                .disabled(!has_session || is_loading)
                .on_click(move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    let Some(session_key) = run_session.clone() else {
                        return;
                    };
                    let field = run_field
                        .as_ref()
                        .map(|input| input.read(cx).value().to_string())
                        .unwrap_or_default();
                    let filter = run_filter
                        .as_ref()
                        .map(|input| input.read(cx).value().to_string())
                        .unwrap_or_default();
                    AppCommands::run_distinct_query(
                        run_state.clone(),
                        session_key,
                        field,
                        filter,
                        cx,
                    );
                }),
        );

    if distinct.values.is_some() || distinct.loading || distinct.error.is_some() {
        row.child(
            div()
                .text_xs()
                .text_color(muted_fg)
                .child(format!("{} values", format_number(distinct.total))),
        )
    } else {
        row
    }
}

fn placeholder_input(text: &str, border: Hsla, muted_fg: Hsla) -> AnyElement {
    div()
        .h(px(24.0))
        .flex()
        .items_center()
        .px(spacing::xs())
        .rounded(px(4.0))
        .border_1()
        .border_color(border)
        .text_xs()
        .text_color(muted_fg)
        .child(text.to_string())
        .into_any_element()
}

fn render_body(distinct: &DistinctState, muted_fg: Hsla, danger_fg: Hsla, cx: &App) -> AnyElement {
    if distinct.loading {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .child(Spinner::new().small())
            .child(div().text_sm().text_color(muted_fg).child("Loading distinct values..."))
            .into_any_element();
    }

    if let Some(error) = distinct.error.clone() {
        return div()
            .flex()
            .flex_1()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .child(div().text_sm().text_color(danger_fg).child(error))
            .into_any_element();
    }

    let Some(values) = distinct.values.clone() else {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(div().text_sm().text_color(muted_fg).child(
                "Enter a field name and press Run to load distinct values for this collection.",
            ))
            .into_any_element();
    };

    if values.is_empty() {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(div().text_sm().text_color(muted_fg).child("No distinct values found"))
            .into_any_element();
    }

    let mut list = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .px(spacing::lg())
        .py(spacing::sm())
        .gap(px(2.0))
        .overflow_y_scrollbar();

    for (index, value) in values.iter().enumerate() {
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .px(spacing::sm())
                .py(px(4.0))
                .rounded(px(4.0))
                .hover(|s| s.bg(cx.theme().list_hover))
                .child(
                    div()
                        .w(px(40.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .child(format!("{}", index + 1)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .font_family(fonts::mono())
                        .text_color(cx.theme().foreground)
                        .child(value.clone()),
                ),
        );
    }

    list.into_any_element()
}
