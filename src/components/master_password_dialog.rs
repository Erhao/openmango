//! Modal dialog for setting or unlocking the encrypted secrets vault.
//!
//! The vault stores all connection passwords / SSH keys / proxy passwords /
//! AI provider API keys. On first launch, the user is prompted to set a master
//! password (Mode::Set). On subsequent launches, the user is prompted to enter
//! it (Mode::Unlock). Once resolved, `on_done` is invoked and the dialog
//! closes.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::components::{Button, open_confirm_dialog};
use crate::helpers::secrets_vault;
use crate::theme::spacing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MasterPasswordMode {
    /// First launch — vault file does not exist yet. User picks a master password.
    Set,
    /// File exists — user must type the existing master password to unlock.
    Unlock,
}

type DoneCallback = Box<dyn FnOnce(&mut Window, &mut App) + 'static>;

struct MasterPasswordDialog {
    mode: MasterPasswordMode,
    password_state: Entity<InputState>,
    confirm_state: Option<Entity<InputState>>,
    error: Option<SharedString>,
    on_done: Rc<RefCell<Option<DoneCallback>>>,
    submitting: bool,
}

impl MasterPasswordDialog {
    fn new(
        mode: MasterPasswordMode,
        on_done: DoneCallback,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let password_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("Master password").masked(true));
        let confirm_state =
            if mode == MasterPasswordMode::Set {
                Some(cx.new(|cx| {
                    InputState::new(window, cx).placeholder("Confirm password").masked(true)
                }))
            } else {
                None
            };

        // Treat Enter on either input as "submit".
        let view_handle = cx.entity().downgrade();
        cx.subscribe_in(
            &password_state,
            window,
            move |this: &mut Self, _state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(window, cx);
                } else if matches!(event, InputEvent::Change) && this.error.is_some() {
                    this.error = None;
                    cx.notify();
                }
            },
        )
        .detach();
        if let Some(confirm) = &confirm_state {
            cx.subscribe_in(
                confirm,
                window,
                move |this: &mut Self, _state, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.submit(window, cx);
                    } else if matches!(event, InputEvent::Change) && this.error.is_some() {
                        this.error = None;
                        cx.notify();
                    }
                },
            )
            .detach();
        }
        let _ = view_handle;

        // Auto-focus the password input when the dialog opens.
        let password_focus = password_state.clone();
        window.defer(cx, move |window, cx| {
            password_focus.update(cx, |state, cx| state.focus(window, cx));
        });

        Self {
            mode,
            password_state,
            confirm_state,
            error: None,
            on_done: Rc::new(RefCell::new(Some(on_done))),
            submitting: false,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let password = self.password_state.read(cx).value().to_string();
        if password.is_empty() {
            self.error = Some("Password cannot be empty".into());
            cx.notify();
            return;
        }

        if self.mode == MasterPasswordMode::Set {
            let confirm = self
                .confirm_state
                .as_ref()
                .map(|s| s.read(cx).value().to_string())
                .unwrap_or_default();
            if confirm != password {
                self.error = Some("Passwords do not match".into());
                cx.notify();
                return;
            }
            if password.len() < 8 {
                self.error = Some("Password must be at least 8 characters".into());
                cx.notify();
                return;
            }
        }

        let mode = self.mode;
        self.submitting = true;
        self.error = None;
        cx.notify();

        // Run the (potentially expensive) Argon2 derivation off the main thread.
        let weak = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx: &mut AsyncWindowContext| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let Some(vault) = secrets_vault::global() else {
                        return Err(anyhow::anyhow!("vault not initialised"));
                    };
                    let mut guard =
                        vault.lock().map_err(|_| anyhow::anyhow!("vault mutex poisoned"))?;
                    match mode {
                        MasterPasswordMode::Set => guard.create(&password),
                        MasterPasswordMode::Unlock => guard.unlock(&password),
                    }
                })
                .await;

            let _ = cx.update(|window, cx| {
                let Some(view) = weak.upgrade() else { return };
                view.update(cx, |this, cx| {
                    this.submitting = false;
                    match result {
                        Ok(()) => {
                            // Take callback out and invoke after closing the dialog.
                            let callback = this.on_done.borrow_mut().take();
                            window.close_dialog(cx);
                            if let Some(cb) = callback {
                                cb(window, cx);
                            }
                        }
                        Err(err) => {
                            this.error = Some(format!("{err}").into());
                            // Refocus password input for a retry.
                            let pw = this.password_state.clone();
                            window.defer(cx, move |window, cx| {
                                pw.update(cx, |state, cx| state.focus(window, cx));
                            });
                            cx.notify();
                        }
                    }
                });
            });
        })
        .detach();
    }

    fn confirm_reset(&self, window: &mut Window, cx: &mut App) {
        open_confirm_dialog(
            window,
            cx,
            "Reset vault?",
            "This will delete all saved connection passwords, SSH keys, proxy \
             passwords, and AI provider API keys. You will need to re-enter \
             them. This cannot be undone.",
            "Reset and erase",
            true,
            |window, cx| {
                if let Some(vault) = secrets_vault::global()
                    && let Ok(mut guard) = vault.lock()
                {
                    let _ = guard.reset();
                }
                // Close the (still-open) master password dialog and reopen it
                // in Set mode so the user can choose a new password.
                window.close_dialog(cx);
                open_master_password_dialog(window, cx, MasterPasswordMode::Set, |_, _| {});
            },
        );
    }
}

impl Render for MasterPasswordDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let view = cx.entity();
        let mode = self.mode;
        let submitting = self.submitting;

        let intro: SharedString = match mode {
            MasterPasswordMode::Set => "Choose a master password to encrypt your saved \
                                        connection details and API keys. You'll need to \
                                        enter it each time you open OpenMango."
                .into(),
            MasterPasswordMode::Unlock => "Enter your master password to unlock saved \
                                          connections and API keys."
                .into(),
        };

        let action_label: SharedString = match mode {
            MasterPasswordMode::Set => {
                if submitting {
                    "Setting...".into()
                } else {
                    "Set password".into()
                }
            }
            MasterPasswordMode::Unlock => {
                if submitting {
                    "Unlocking...".into()
                } else {
                    "Unlock".into()
                }
            }
        };

        let mut body = div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .p(spacing::md())
            .min_w(px(420.0))
            .child(div().text_sm().text_color(theme.secondary_foreground).child(intro))
            .child(
                Input::new(&self.password_state)
                    .appearance(true)
                    .bordered(true)
                    .focus_bordered(true)
                    .small(),
            );

        if let Some(confirm) = &self.confirm_state {
            body = body.child(
                Input::new(confirm).appearance(true).bordered(true).focus_bordered(true).small(),
            );
        }

        if let Some(error) = self.error.clone() {
            body = body.child(div().text_xs().text_color(theme.danger).child(error));
        }

        let mut button_row = div().flex().items_center().justify_between().gap(spacing::sm());

        // Left side: Reset link in Unlock mode.
        let left = if mode == MasterPasswordMode::Unlock {
            let view = view.clone();
            div().child(
                Button::new("master-pw-reset")
                    .ghost()
                    .compact()
                    .label("Forgot password — reset vault")
                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        view.update(cx, |this, cx| {
                            this.confirm_reset(window, cx);
                            cx.notify();
                        });
                    }),
            )
        } else {
            div()
        };

        let action = Button::new("master-pw-action")
            .primary()
            .label(action_label)
            .disabled(submitting)
            .on_click({
                let view = view.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| this.submit(window, cx));
                }
            });

        button_row = button_row.child(left).child(div().child(action));
        body = body.child(button_row);
        body
    }
}

/// Open a modal master password dialog. The dialog cannot be cancelled — the
/// user must either succeed or use the Reset path. `on_done` is invoked after
/// the vault is successfully unlocked or created.
pub fn open_master_password_dialog(
    window: &mut Window,
    cx: &mut App,
    mode: MasterPasswordMode,
    on_done: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let dialog_view = cx.new(|cx| MasterPasswordDialog::new(mode, Box::new(on_done), window, cx));
    let title: SharedString = match mode {
        MasterPasswordMode::Set => "Set master password".into(),
        MasterPasswordMode::Unlock => "Unlock vault".into(),
    };
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        dialog.title(title.clone()).min_w(px(440.0)).keyboard(false).child(dialog_view.clone())
    });
}
