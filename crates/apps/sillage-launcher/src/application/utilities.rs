use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, VecModel};

use super::{LauncherWindow, UiWeak, lock};
use crate::LauncherError;
use crate::utilities::{Utilities, UtilityEffect, UtilityKind};

mod autostart;
mod retention;

enum StoreState {
    Loading,
    Ready(Utilities),
    Unavailable(LauncherError),
}

struct Controller {
    store: Mutex<StoreState>,
    runtime: tokio::runtime::Handle,
    epoch: AtomicU64,
    capture: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

struct Row {
    id: String,
    title: String,
    preview: String,
}

struct Editor {
    id: String,
    title: String,
    content: String,
}

pub(super) fn install(ui: &LauncherWindow, runtime: tokio::runtime::Handle) {
    let controller = Arc::new(Controller {
        store: Mutex::new(StoreState::Loading),
        runtime: runtime.clone(),
        epoch: AtomicU64::new(0),
        capture: Mutex::new(None),
    });
    install_navigation(ui, &controller);
    install_mutations(ui, &controller);
    install_expansion(ui, &controller);
    install_capture(ui, &controller);
    autostart::install(ui, runtime);
    controller.load(ui.as_weak());
    retention::install(ui, controller);
}

fn kind(value: &str) -> Result<UtilityKind, LauncherError> {
    match value {
        "quicklinks" => Ok(UtilityKind::Quicklink),
        "snippets" => Ok(UtilityKind::Snippet),
        "clipboard" => Ok(UtilityKind::Clipboard),
        _ => Err(LauncherError::invalid_request("Unknown utility collection")),
    }
}

fn rows(store: &mut Utilities, collection: UtilityKind, query: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    store.visit_entries(collection, |entry| {
        if !entry.matches(query) {
            return;
        }
        let end = entry
            .content
            .char_indices()
            .nth(120)
            .map_or(entry.content.len(), |(offset, _)| offset);
        rows.push(Row {
            id: entry.id.clone(),
            title: entry.title.clone(),
            preview: entry.content[..end].to_owned(),
        });
    });
    rows
}

fn show_rows(ui: &LauncherWindow, rows: Vec<Row>) {
    let rows = rows
        .into_iter()
        .map(|row| crate::UtilityRow {
            id: row.id.into(),
            title: row.title.into(),
            preview: row.preview.into(),
        })
        .collect::<Vec<_>>();
    ui.set_utility_rows(ModelRc::new(VecModel::from(rows)));
}

fn clear_editor(ui: &LauncherWindow) {
    ui.set_utility_editing(false);
    ui.set_utility_selected_id("".into());
    ui.set_utility_title("".into());
    ui.set_utility_content("".into());
    ui.set_utility_saved_content("".into());
    ui.set_utility_argument("".into());
}

impl Controller {
    fn with_store<T>(
        &self,
        operation: impl FnOnce(&mut Utilities) -> Result<T, LauncherError>,
    ) -> Result<T, LauncherError> {
        let mut guard = lock(&self.store);
        match &mut *guard {
            StoreState::Ready(store) => operation(store),
            StoreState::Unavailable(error) => Err(error.clone()),
            StoreState::Loading => Err(LauncherError::file_unavailable("Utilities are loading.")),
        }
    }

    fn cancel(&self) -> u64 {
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(capture) = lock(&self.capture).take() {
            capture.abort();
        }
        epoch
    }

    fn load(self: &Arc<Self>, weak: UiWeak) {
        let host = Arc::clone(self);
        self.runtime.spawn_blocking(move || {
            let loaded = Utilities::load(super::platform::config_dir());
            let available = loaded.is_ok();
            let message = loaded.as_ref().err().map(|error| error.message.clone());
            *lock(&host.store) = match loaded {
                Ok(store) => StoreState::Ready(store),
                Err(error) => StoreState::Unavailable(error),
            };
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                ui.set_utility_available(available);
                if let Some(message) = message {
                    ui.set_utility_message(message.into());
                } else if ui.get_utilities_open() {
                    host.refresh(&ui);
                }
            });
        });
    }

    fn submit<T: Send + 'static>(
        self: &Arc<Self>,
        ui: &LauncherWindow,
        operation: impl FnOnce(&mut Utilities) -> Result<T, LauncherError> + Send + 'static,
        apply: impl FnOnce(&LauncherWindow, T) + Send + 'static,
        success: &'static str,
    ) {
        let epoch = self.cancel();
        ui.set_utility_busy(true);
        let host = Arc::clone(self);
        let weak = ui.as_weak();
        self.runtime.spawn_blocking(move || {
            let result = host.with_store(operation);
            let _ = slint::invoke_from_event_loop(move || {
                if host.epoch.load(Ordering::Acquire) != epoch {
                    return;
                }
                let Some(ui) = weak.upgrade() else { return };
                ui.set_utility_busy(false);
                match result {
                    Ok(value) => {
                        apply(&ui, value);
                        ui.set_utility_message(success.into());
                    }
                    Err(error) => ui.set_utility_message(error.message.into()),
                }
            });
        });
    }

    fn refresh(self: &Arc<Self>, ui: &LauncherWindow) {
        let collection = kind(ui.get_utility_kind().as_str());
        let query = ui.get_utility_query().to_lowercase();
        self.submit(
            ui,
            move |store| Ok(rows(store, collection?, &query)),
            show_rows,
            "",
        );
    }
}

fn install_navigation(ui: &LauncherWindow, controller: &Arc<Controller>) {
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_utility_kind_requested(move |collection| {
        if let Some(ui) = weak.upgrade() {
            ui.set_utility_kind(collection);
            ui.set_utility_query("".into());
            clear_editor(&ui);
            host.refresh(&ui);
        }
    });
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_utility_filter_requested(move |_| {
        if let Some(ui) = weak.upgrade() {
            host.refresh(&ui);
        }
    });
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_utility_selected_requested(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_utility_busy() {
            return;
        }
        let collection = kind(ui.get_utility_kind().as_str());
        let id = id.to_string();
        clear_editor(&ui);
        host.submit(
            &ui,
            move |store| {
                let entry = store.entry(collection?, &id)?;
                Ok(Editor {
                    id: entry.id.clone(),
                    title: entry.title.clone(),
                    content: entry.content.clone(),
                })
            },
            |ui, entry| {
                ui.set_utility_selected_id(entry.id.into());
                ui.set_utility_title(entry.title.into());
                let content: slint::SharedString = entry.content.into();
                ui.set_utility_content(content.clone());
                ui.set_utility_saved_content(content);
                ui.set_utility_argument("".into());
                ui.set_utility_editing(true);
                ui.invoke_focus_utility_editor();
            },
            "",
        );
    });
}

fn install_mutations(ui: &LauncherWindow, controller: &Arc<Controller>) {
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_utility_save_requested(move |id, title, content| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_utility_busy() {
            return;
        }
        let collection = kind(ui.get_utility_kind().as_str());
        let query = ui.get_utility_query().to_lowercase();
        let (id, title, content) = (id.to_string(), title.to_string(), content.to_string());
        host.submit(
            &ui,
            move |store| {
                let collection = collection?;
                store.upsert(collection, &id, &title, &content)?;
                Ok(rows(store, collection, &query))
            },
            |ui, result| {
                show_rows(ui, result);
                clear_editor(ui);
            },
            "Saved utility entry.",
        );
    });
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_utility_remove_requested(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_utility_busy() {
            return;
        }
        let collection = kind(ui.get_utility_kind().as_str());
        let query = ui.get_utility_query().to_lowercase();
        let id = id.to_string();
        host.submit(
            &ui,
            move |store| {
                let collection = collection?;
                store.remove(collection, &id)?;
                Ok(rows(store, collection, &query))
            },
            |ui, result| {
                show_rows(ui, result);
                clear_editor(ui);
            },
            "Deleted utility entry.",
        );
    });
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_utility_clear_requested(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_utility_busy() || ui.get_utility_kind().as_str() != "clipboard" {
            return;
        }
        host.submit(
            &ui,
            |store| {
                store.clear_clipboard();
                Ok(rows(store, UtilityKind::Clipboard, ""))
            },
            |ui, result| {
                show_rows(ui, result);
                clear_editor(ui);
            },
            "Cleared clipboard history.",
        );
    });
}

fn install_expansion(ui: &LauncherWindow, controller: &Arc<Controller>) {
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_utility_expand_requested(move |id, argument| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_utility_busy() || !ui.get_utility_editing() || id != ui.get_utility_selected_id()
        {
            return;
        }
        let collection = kind(ui.get_utility_kind().as_str());
        let (id, argument) = (id.to_string(), argument.to_string());
        let draft = ui.get_utility_content().to_string();
        host.submit(
            &ui,
            move |store| {
                if store.entry(collection?, &id)?.content != draft {
                    return Err(LauncherError::invalid_request(
                        "Save the changed template before expanding it.",
                    ));
                }
                match store.expand(&id, &argument)? {
                    UtilityEffect::Copy(text) => super::platform::copy_text(&text),
                    UtilityEffect::OpenUri(uri) => crate::platform::open_uri(&uri),
                }
            },
            |_, ()| {},
            "Completed explicit utility action.",
        );
    });
}

fn install_capture(ui: &LauncherWindow, controller: &Arc<Controller>) {
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_utility_capture_requested(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_utility_busy() || ui.get_utility_kind().as_str() != "clipboard" {
            return;
        }
        let epoch = host.cancel();
        ui.set_utility_busy(true);
        ui.set_utility_message("Reading clipboard for this explicit save…".into());
        let capture_host = Arc::clone(&host);
        let capture_ui = weak.clone();
        let query = ui.get_utility_query().to_lowercase();
        let task = host.runtime.spawn(async move {
            let captured = crate::clipboard::read_text().await;
            let worker_host = Arc::clone(&capture_host);
            let result = capture_host
                .runtime
                .spawn_blocking(move || {
                    captured.and_then(|text| {
                        worker_host.with_store(|store| {
                            if worker_host.epoch.load(Ordering::Acquire) != epoch {
                                return Err(LauncherError::stale_result(
                                    "Clipboard save was cancelled",
                                ));
                            }
                            store.capture_clipboard(&text)?;
                            Ok(rows(store, UtilityKind::Clipboard, &query))
                        })
                    })
                })
                .await;
            let _ = slint::invoke_from_event_loop(move || {
                if capture_host.epoch.load(Ordering::Acquire) != epoch {
                    return;
                }
                let Some(ui) = capture_ui.upgrade() else {
                    return;
                };
                ui.set_utility_busy(false);
                match result {
                    Ok(Ok(rows)) => {
                        show_rows(&ui, rows);
                        ui.set_utility_message(
                            "Explicitly saved clipboard text for one hour.".into(),
                        );
                    }
                    Ok(Err(error)) => ui.set_utility_message(error.message.into()),
                    Err(_) => {
                        ui.set_utility_message("Clipboard save worker could not finish.".into())
                    }
                }
            });
        });
        *lock(&host.capture) = Some(task);
    });
}
