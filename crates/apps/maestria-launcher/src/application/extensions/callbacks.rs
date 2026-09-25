use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use maestria_extensions::bundle::InstallApproval;
use maestria_extensions::{FormValue, FormValues, View};
use slint::{ComponentHandle, ModelRc, VecModel};
use tokio::task::JoinHandle;

use super::super::platform::choose_file;
use super::super::{LauncherWindow, UiWeak, lock};
use super::{broker, form, invoke, management, view};
use crate::ipc::LauncherState;

pub(super) struct PendingInstall {
    pub(super) source: PathBuf,
    pub(super) archive: bool,
    pub(super) approval: InstallApproval,
}

pub(super) struct PanelModel {
    pub(super) pending: Option<PendingInstall>,
    pub(super) selected_extension: Option<String>,
    pub(super) selected_command: Option<String>,
    pub(super) current_view: Option<View>,
    pub(super) values: FormValues,
    pub(super) selected_files: BTreeMap<String, broker::SelectedFile>,
    pub(super) next_selection: u64,
    pub(super) form_error: Option<String>,
    pub(super) worker: Option<JoinHandle<()>>,
}

pub(super) struct Controller {
    pub(super) store_root: Result<PathBuf, String>,
    pub(super) state: Arc<LauncherState>,
    pub(super) runtime: tokio::runtime::Handle,
    pub(super) epoch: AtomicU64,
    pub(super) model: Mutex<PanelModel>,
}

impl Controller {
    pub(super) fn cancel_command(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let mut model = lock(&self.model);
        if let Some(worker) = model.worker.take() {
            worker.abort();
        }
        model.current_view = None;
        model.selected_command = None;
        model.values.clear();
        model.selected_files.clear();
        model.form_error = None;
    }

    pub(super) fn store_root(&self) -> Result<PathBuf, String> {
        self.store_root.clone()
    }
}

fn data_root() -> Result<PathBuf, String> {
    let data_home = match std::env::var_os("XDG_DATA_HOME") {
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err("XDG_DATA_HOME must be absolute to manage extensions".to_owned());
            }
            path
        }
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join(".local/share"))
            .ok_or_else(|| "a private user data directory is required for extensions".to_owned())?,
    };
    Ok(data_home.join("io.github.briolabs.Maestria.Launcher/extensions"))
}

pub(super) fn install_callbacks(
    ui: &LauncherWindow,
    state: Arc<LauncherState>,
    runtime: tokio::runtime::Handle,
) {
    let controller = Arc::new(Controller {
        store_root: data_root(),
        state,
        runtime,
        epoch: AtomicU64::new(0),
        model: Mutex::new(PanelModel {
            pending: None,
            selected_extension: None,
            selected_command: None,
            current_view: None,
            values: FormValues::new(),
            selected_files: BTreeMap::new(),
            next_selection: 0,
            form_error: None,
            worker: None,
        }),
    });

    register_management_callbacks(ui, &controller);
    register_invocation_callbacks(ui, &controller);
    register_form_callbacks(ui, &controller);
}

fn register_management_callbacks(ui: &LauncherWindow, controller: &Arc<Controller>) {
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extensions_requested(move || management::refresh(&host, weak.clone()));

    let host = Arc::clone(controller);
    ui.on_extensions_closed(move || {
        host.cancel_command();
        lock(&host.model).pending = None;
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_install_requested(move |path| {
        management::preview(&host, weak.clone(), PathBuf::from(path.as_str()));
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_approve_requested(move |id| {
        management::approve(&host, weak.clone(), id.as_str());
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_selected_requested(move |id| {
        management::select(&host, weak.clone(), id.as_str());
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_disable_requested(move |id| {
        management::mutate(
            &host,
            weak.clone(),
            id.as_str(),
            management::Mutation::Disable,
        );
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_revoke_requested(move |id| {
        management::mutate(
            &host,
            weak.clone(),
            id.as_str(),
            management::Mutation::Revoke,
        );
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_remove_requested(move |id, retain_data| {
        management::mutate(
            &host,
            weak.clone(),
            id.as_str(),
            management::Mutation::Remove { retain_data },
        );
    });
}

fn register_invocation_callbacks(ui: &LauncherWindow, controller: &Arc<Controller>) {
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_invoke_requested(move |id, command| {
        invoke::command(&host, weak.clone(), id.as_str(), command.as_str());
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_action_requested(move |action, item| {
        invoke::action(&host, weak.clone(), action.as_str(), item.as_str());
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_item_selected(move |item| {
        let selected = lock(&host.model)
            .current_view
            .as_ref()
            .and_then(|view| view::item_actions(view, item.as_str()));
        if let Some(ui) = weak.upgrade() {
            match selected {
                Some(actions) => ui.set_extension_actions(actions),
                None => {
                    ui.set_extension_actions(ModelRc::new(VecModel::default()));
                    ui.set_extension_error_message(
                        "That extension item is no longer available.".into(),
                    );
                }
            }
        }
    });
}

fn register_form_callbacks(ui: &LauncherWindow, controller: &Arc<Controller>) {
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_form_changed(move |field_id, value| {
        let result = {
            let mut model = lock(&host.model);
            let PanelModel {
                current_view,
                values,
                form_error,
                ..
            } = &mut *model;
            let result = current_view
                .as_ref()
                .ok_or(form::FormInputError::NotForm)
                .and_then(|view| {
                    form::set_user_value(view, values, field_id.as_str(), value.as_str())
                });
            *form_error = result.as_ref().err().map(ToString::to_string);
            result
        };
        if let Some(ui) = weak.upgrade() {
            match result {
                Ok(()) => ui.set_extension_error_message("".into()),
                Err(error) => ui.set_extension_error_message(error.to_string().into()),
            }
        }
    });

    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_extension_file_select_requested(move |field_id| {
        request_file_selection(&host, weak.clone(), field_id.as_str());
    });
}

fn request_file_selection(controller: &Arc<Controller>, ui: UiWeak, field_id: &str) {
    let field_id = field_id.to_owned();
    {
        let model = lock(&controller.model);
        let Some(view) = model.current_view.as_ref() else {
            set_error(&ui, "No extension form is active".to_owned());
            return;
        };
        if !form::is_file_field(view, &field_id) {
            set_error(
                &ui,
                "The requested extension field is not a file selection".to_owned(),
            );
            return;
        }
    }
    let Some(window) = ui.upgrade() else { return };
    if let Err(error) = controller.state.begin_modal() {
        set_error(&ui, error.message);
        return;
    }
    #[cfg(target_os = "linux")]
    let parent = crate::platform::window_identifier(window.window(), &controller.runtime);
    #[cfg(not(target_os = "linux"))]
    let parent = None;
    let host = Arc::clone(controller);
    let epoch = controller.epoch.load(Ordering::Acquire);
    controller.runtime.spawn(async move {
        let result = choose_file(parent).await;
        host.state.end_modal();
        let result = match result {
            Ok(Some(path)) => broker::SelectedFile::from_host_selection(path)
                .map(Some)
                .map_err(|error| error.to_string()),
            Ok(None) => Ok(None),
            Err(error) => Err(error.message),
        };
        let _ = slint::invoke_from_event_loop(move || {
            if epoch != host.epoch.load(Ordering::Acquire) {
                return;
            }
            let Some(window) = ui.upgrade() else { return };
            match result {
                Ok(Some(file)) => {
                    let mut model = lock(&host.model);
                    let Some(next) = model.next_selection.checked_add(1) else {
                        window.set_extension_error_message("Too many file selections".into());
                        return;
                    };
                    model.next_selection = next;
                    let selection_id = format!("selection-{next}");
                    let PanelModel {
                        current_view,
                        values,
                        selected_files,
                        form_error,
                        ..
                    } = &mut *model;
                    let Some(view) = current_view.as_ref() else {
                        return;
                    };
                    let replaced_id = values.get(&field_id).and_then(|value| match value {
                        FormValue::Text(id) => Some(id.clone()),
                        _ => None,
                    });
                    if let Err(error) = form::set_host_file(view, values, &field_id, &selection_id)
                    {
                        window.set_extension_error_message(error.to_string().into());
                        return;
                    }
                    if let Some(replaced_id) = replaced_id {
                        selected_files.remove(&replaced_id);
                    }
                    selected_files.insert(selection_id, file);
                    *form_error = None;
                    view::sync_form_values(&window, view, values);
                    window.set_extension_error_message("".into());
                    window.set_extension_notice_message(
                        "File selected for this extension action.".into(),
                    );
                }
                Ok(None) => {}
                Err(error) => window.set_extension_error_message(error.into()),
            }
        });
    });
}

pub(super) fn set_error(ui: &UiWeak, error: String) {
    if let Some(window) = ui.upgrade() {
        window.set_extension_busy(false);
        window.set_extension_error_message(error.into());
    }
}
