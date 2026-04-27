//! Distinct field-value query command.

use gpui::{App, AppContext as _, Entity};
use mongodb::bson::Bson;

use crate::bson::{format_relaxed_json_value, parse_document_from_json};
use crate::state::{AppState, SessionKey, StatusMessage};

use super::AppCommands;

impl AppCommands {
    pub fn run_distinct_query(
        state: Entity<AppState>,
        session_key: SessionKey,
        field: String,
        filter_raw: String,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        let trimmed_field = field.trim().to_string();
        if trimmed_field.is_empty() {
            state.update(cx, |state, cx| {
                if let Some(session) = state.session_mut(&session_key) {
                    session.data.distinct.error = Some("Field is required".to_string());
                    session.data.distinct.values = None;
                    session.data.distinct.total = 0;
                    session.data.distinct.loading = false;
                }
                state.set_status_message(Some(StatusMessage::error("Distinct: field is required")));
                cx.notify();
            });
            return;
        }

        let filter_doc = match filter_raw.trim() {
            "" | "{}" => None,
            other => match parse_document_from_json(other) {
                Ok(doc) => Some(doc),
                Err(e) => {
                    let error = format!("Invalid filter JSON: {e}");
                    state.update(cx, |state, cx| {
                        if let Some(session) = state.session_mut(&session_key) {
                            session.data.distinct.error = Some(error.clone());
                            session.data.distinct.values = None;
                            session.data.distinct.total = 0;
                            session.data.distinct.loading = false;
                        }
                        state.set_status_message(Some(StatusMessage::error(error)));
                        cx.notify();
                    });
                    return;
                }
            },
        };

        let request_id = state.update(cx, |state, cx| {
            let session = state.ensure_session(session_key.clone());
            session.data.distinct.field_raw = trimmed_field.clone();
            session.data.distinct.filter_raw = filter_raw.clone();
            session.data.distinct.loading = true;
            session.data.distinct.error = None;
            session.data.distinct.request_id = session.data.distinct.request_id.wrapping_add(1);
            cx.notify();
            session.data.distinct.request_id
        });

        let manager = state.read(cx).connection_manager();
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();

        let task =
            cx.background_spawn({
                let field = trimmed_field.clone();
                async move {
                    manager.distinct_values(&client, &database, &collection, &field, filter_doc)
                }
            });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(values) => {
                        let formatted: Vec<String> =
                            values.iter().map(format_distinct_value).collect();
                        let total = formatted.len() as u64;
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            if session.data.distinct.request_id != request_id {
                                return;
                            }
                            session.data.distinct.loading = false;
                            session.data.distinct.values = Some(formatted);
                            session.data.distinct.total = total;
                            session.data.distinct.error = None;
                            state.set_status_message(Some(StatusMessage::info(format!(
                                "Distinct: {total} value(s)"
                            ))));
                            cx.notify();
                        });
                    }
                    Err(err) => {
                        let error = err.to_string();
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            if session.data.distinct.request_id != request_id {
                                return;
                            }
                            session.data.distinct.loading = false;
                            session.data.distinct.error = Some(error.clone());
                            session.data.distinct.values = None;
                            session.data.distinct.total = 0;
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Distinct failed: {error}"
                            ))));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }
}

/// Render a BSON value as a shell-style string (e.g. `"foo"`, `42`, `ObjectId("...")`).
fn format_distinct_value(value: &Bson) -> String {
    let json = value.clone().into_relaxed_extjson();
    format_relaxed_json_value(&json)
}
