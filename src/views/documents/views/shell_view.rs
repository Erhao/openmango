//! Embedded Mongo Shell sub-tab.
//!
//! This sub-tab provides quick access to the Forge query shell scoped to the
//! current collection. The actual interactive shell renders in the dedicated
//! Forge view (an external `mongosh-sidecar` process).

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::{Icon, IconName};

use crate::components::Button;
use crate::state::{AppState, SessionKey};
use crate::theme::{fonts, spacing};

/// Render the Shell sub-tab.
pub fn render_shell_panel(
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let muted_fg = theme.muted_foreground;
    let foreground = theme.foreground;
    let primary = theme.primary;
    let border = theme.border;
    let surface = theme.tab_bar.opacity(0.5);

    let collection = session_key.as_ref().map(|sk| sk.collection.clone());
    let database = session_key.as_ref().map(|sk| sk.database.clone());

    let mut card = div()
        .flex()
        .flex_col()
        .gap(spacing::md())
        .max_w(px(640.0))
        .p(spacing::lg())
        .rounded(px(8.0))
        .border_1()
        .border_color(border)
        .bg(surface)
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(Icon::new(IconName::SquareTerminal).small().text_color(primary))
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(foreground)
                        .child("Mongo Shell"),
                ),
        )
        .child(div().text_sm().text_color(muted_fg).child(
            "Run JavaScript and MongoDB shell commands in an interactive \
                     session powered by mongosh.",
        ));

    if let (Some(database), Some(collection)) = (database.as_ref(), collection.as_ref()) {
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .text_xs()
                .text_color(muted_fg)
                .child(div().child("Scope:"))
                .child(
                    div()
                        .px(spacing::xs())
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(theme.background)
                        .border_1()
                        .border_color(border)
                        .font_family(fonts::mono())
                        .text_color(foreground)
                        .child(format!("{database}.{collection}")),
                ),
        );
    }

    let example = div()
        .px(spacing::sm())
        .py(spacing::xs())
        .rounded(px(6.0))
        .bg(theme.background)
        .border_1()
        .border_color(border)
        .font_family(fonts::mono())
        .text_xs()
        .text_color(foreground)
        .child(match collection.as_deref() {
            Some(coll) => format!("db.getCollection(\"{coll}\").find({{}})"),
            None => "db.runCommand({ ping: 1 })".to_string(),
        });
    card = card.child(div().flex().flex_col().gap(spacing::xs()).child(example));

    let open_btn = Button::new("shell-open-forge")
        .primary()
        .label("Open Mongo Shell")
        .icon(Icon::new(IconName::SquareTerminal))
        .disabled(session_key.is_none())
        .on_click({
            let state = state.clone();
            let session_key = session_key.clone();
            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                let Some(session_key) = session_key.clone() else {
                    return;
                };
                state.update(cx, |state, cx| {
                    state.open_forge_tab(
                        session_key.connection_id,
                        session_key.database.clone(),
                        Some(session_key.collection.clone()),
                        cx,
                    );
                });
            }
        });

    card =
        card.child(
            div().flex().items_center().gap(spacing::sm()).child(open_btn).child(
                div().text_xs().text_color(muted_fg).child("Tip: ⌘⌥F also opens the shell."),
            ),
        );

    div()
        .flex()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .p(spacing::lg())
        .child(card)
        .into_any_element()
}
