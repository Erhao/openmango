use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{App, AppContext as _, Entity};

use crate::components::file_picker::{FilePickerMode, open_file_dialog_async};
use crate::connection::types::{
    CancellationToken, ExportQueryOptions, ExtendedJsonMode, JsonExportOptions, JsonTransferFormat,
};
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};
use crate::views::documents::export::FileExportFormat;

impl AppCommands {
    pub fn save_as_file(
        state: Entity<AppState>,
        session_key: SessionKey,
        format: FileExportFormat,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        let (filter, sort, projection, column_widths, column_order, manager) = {
            let st = state.read(cx);
            let (filter, sort, projection) = match st.session(&session_key) {
                Some(session) => (
                    session.data.filter.clone(),
                    session.data.sort.clone(),
                    session.data.projection.clone(),
                ),
                None => (None, None, None),
            };
            let widths = st.table_column_widths(&session_key);
            let order = st.table_column_order(&session_key);
            let mgr = st.connection_manager();
            (filter, sort, projection, widths, order, mgr)
        };

        let collection = session_key.collection.clone();
        let database = session_key.database.clone();
        let filters = format.file_filters();
        let now = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let default_name = format!("{}_{}.{}", collection, now, format.extension());

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let path =
                    open_file_dialog_async(FilePickerMode::Save, filters, Some(default_name)).await;

                let Some(path) = path else {
                    return;
                };

                Self::run_export(
                    state,
                    client,
                    manager,
                    database,
                    collection,
                    path,
                    format,
                    filter,
                    sort,
                    projection,
                    column_widths,
                    column_order,
                    cx,
                );
            }
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn run_export(
        state: Entity<AppState>,
        client: mongodb::Client,
        manager: std::sync::Arc<crate::connection::ConnectionManager>,
        database: String,
        collection: String,
        path: PathBuf,
        format: FileExportFormat,
        filter: Option<mongodb::bson::Document>,
        sort: Option<mongodb::bson::Document>,
        projection: Option<mongodb::bson::Document>,
        column_widths: HashMap<String, f32>,
        column_order: Vec<String>,
        cx: &mut gpui::AsyncApp,
    ) {
        let cancellation = CancellationToken::new();
        let cancellation_for_task = cancellation.clone();

        let _ = cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.set_export_progress(Some(ExportProgress {
                    count: 0,
                    format,
                    cancellation: cancellation.clone(),
                }));
                state.set_status_message(Some(StatusMessage::info("Exporting...")));
                cx.notify();
            });
        });

        let query = ExportQueryOptions { filter, projection, sort };
        let (tx, rx) = futures::channel::mpsc::unbounded::<u64>();

        let state_for_task = state.clone();
        let _ = cx.update(|cx| {
            let task = cx.background_spawn({
                let database = database.clone();
                let collection = collection.clone();
                let path = path.clone();
                async move {
                    match format {
                        FileExportFormat::JsonArray => {
                            let options = JsonExportOptions {
                                format: JsonTransferFormat::JsonArray,
                                json_mode: ExtendedJsonMode::Relaxed,
                                pretty_print: true,
                                gzip: false,
                                cancellation: Some(cancellation_for_task),
                            };
                            manager.export_collection_json_with_query_and_progress(
                                &client,
                                &database,
                                &collection,
                                &path,
                                options,
                                query,
                                move |count| {
                                    let _ = tx.unbounded_send(count);
                                },
                            )
                        }
                        FileExportFormat::JsonLines => {
                            let options = JsonExportOptions {
                                format: JsonTransferFormat::JsonLines,
                                json_mode: ExtendedJsonMode::Relaxed,
                                pretty_print: false,
                                gzip: false,
                                cancellation: Some(cancellation_for_task),
                            };
                            manager.export_collection_json_with_query_and_progress(
                                &client,
                                &database,
                                &collection,
                                &path,
                                options,
                                query,
                                move |count| {
                                    let _ = tx.unbounded_send(count);
                                },
                            )
                        }
                        FileExportFormat::Csv => manager
                            .export_collection_csv_with_query_and_progress(
                                &client,
                                &database,
                                &collection,
                                &path,
                                false,
                                query,
                                move |count| {
                                    let _ = tx.unbounded_send(count);
                                },
                            ),
                        FileExportFormat::Excel => manager.export_collection_excel_with_query(
                            &client,
                            &database,
                            &collection,
                            &path,
                            query,
                            column_widths,
                            column_order,
                            Some(cancellation_for_task),
                            move |count| {
                                let _ = tx.unbounded_send(count);
                            },
                        ),
                    }
                }
            });

            cx.spawn({
                let state = state_for_task.clone();
                async move |cx: &mut gpui::AsyncApp| {
                    use futures::StreamExt;
                    let mut rx = rx;
                    let progress_task = cx.spawn({
                        let state = state.clone();
                        async move |cx: &mut gpui::AsyncApp| {
                            while let Some(count) = rx.next().await {
                                let _ = cx.update(|cx| {
                                    state.update(cx, |state, cx| {
                                        if let Some(progress) = state.export_progress_mut() {
                                            progress.count = count;
                                        }
                                        cx.notify();
                                    });
                                });
                            }
                        }
                    });

                    let result = task.await;
                    progress_task.detach();

                    let _ = cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            state.set_export_progress(None);
                            match result {
                                Ok(count) => {
                                    state.set_status_message(Some(StatusMessage::info(format!(
                                        "Exported {} documents",
                                        count
                                    ))));
                                }
                                Err(e) => {
                                    let msg = e.to_string();
                                    if !msg.contains("cancelled") {
                                        state.set_status_message(Some(StatusMessage::error(
                                            format!("Export failed: {}", msg),
                                        )));
                                    } else {
                                        state.set_status_message(Some(StatusMessage::info(
                                            "Export cancelled",
                                        )));
                                    }
                                }
                            }
                            cx.notify();
                        });
                    });
                }
            })
            .detach();
        });
    }
}

#[derive(Clone)]
pub struct ExportProgress {
    pub count: u64,
    pub format: FileExportFormat,
    pub cancellation: CancellationToken,
}
