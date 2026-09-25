use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use maestria_extensions::bundle::{ActiveBundle, BundleStore, PackageIdentity};
use maestria_extensions::{
    CapabilityError, CapabilityFailure, CapabilityRequest, CapabilityResponse, FormValues,
    HostMessage, InvocationOrigin, PROTOCOL_VERSION, View, WorkerMessage,
};
use sha2::{Digest, Sha256};
use slint::{ModelRc, VecModel};

use super::broker::{BrokerContext, CapabilityBroker, SearchConsumerConfig, SelectedFile};
use super::form;
use super::sandbox;
use super::transport::WorkerSession;
use super::view;
use super::{Controller, UiWeak, lock, set_error};

struct Invocation {
    extension_id: String,
    command_id: String,
    message: HostMessage,
    origin: InvocationOrigin,
    selected_files: BTreeMap<String, SelectedFile>,
}

pub(super) fn command(controller: &Arc<Controller>, ui: UiWeak, id: &str, command_id: &str) {
    if id.is_empty() || command_id.is_empty() {
        set_error(&ui, "Select an installed extension command".to_owned());
        return;
    }
    begin(
        controller,
        ui,
        Invocation {
            extension_id: id.to_owned(),
            command_id: command_id.to_owned(),
            message: HostMessage::CommandInvoke {
                protocol_version: PROTOCOL_VERSION,
                command_id: command_id.to_owned(),
                input: FormValues::new(),
            },
            origin: InvocationOrigin::Command,
            selected_files: BTreeMap::new(),
        },
    );
}

pub(super) fn action(controller: &Arc<Controller>, ui: UiWeak, action_id: &str, item_id: &str) {
    let invocation = {
        let model = lock(&controller.model);
        let Some(view) = model.current_view.as_ref() else {
            set_error(&ui, "No active extension view has an action".to_owned());
            return;
        };
        let item = if item_id.is_empty() {
            None
        } else {
            Some(item_id)
        };
        if !view::allowed_action(view, action_id, item) {
            set_error(
                &ui,
                "This action is not present in the active extension view".to_owned(),
            );
            return;
        }
        if let Some(error) = model.form_error.as_ref() {
            set_error(&ui, error.clone());
            return;
        }
        let values = if matches!(view, View::Form { .. }) {
            let selected_ids = model
                .selected_files
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>();
            if let Err(error) = form::validate_submission(view, &model.values, &selected_ids) {
                set_error(&ui, error.to_string());
                return;
            }
            model.values.clone()
        } else {
            FormValues::new()
        };
        let Some(extension_id) = model.selected_extension.as_ref() else {
            set_error(&ui, "No active extension is selected".to_owned());
            return;
        };
        let Some(command_id) = model.selected_command.as_ref() else {
            set_error(&ui, "No active extension command is selected".to_owned());
            return;
        };
        Invocation {
            extension_id: extension_id.clone(),
            command_id: command_id.clone(),
            message: HostMessage::ActionInvoke {
                protocol_version: PROTOCOL_VERSION,
                command_id: command_id.clone(),
                action_id: action_id.to_owned(),
                item_id: item.map(str::to_owned),
                values,
            },
            origin: InvocationOrigin::ExplicitAction,
            selected_files: model.selected_files.clone(),
        }
    };
    begin(controller, ui, invocation);
}

fn begin(controller: &Arc<Controller>, ui: UiWeak, invocation: Invocation) {
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(error) => {
            if let Some(window) = ui.upgrade() {
                window.set_extension_actions(ModelRc::new(VecModel::default()));
                window.set_extension_items(ModelRc::new(VecModel::default()));
            }
            set_error(&ui, error);
            return;
        }
    };
    let Some(window) = ui.upgrade() else { return };
    controller.cancel_command();
    let epoch = controller.epoch.load(Ordering::Acquire);
    window.set_extension_busy(true);
    window.set_extension_actions(ModelRc::new(VecModel::default()));
    window.set_extension_items(ModelRc::new(VecModel::default()));
    window.set_extension_error_message("".into());
    window.set_extension_notice_message("".into());
    window.set_extension_status_message("Running isolated extension…".into());
    window.set_extension_view("loading".into());
    let host = Arc::clone(controller);
    let ui_for_worker = ui.clone();
    let handle = controller.runtime.spawn(async move {
        let command_id = invocation.command_id.clone();
        let extension_id = invocation.extension_id.clone();
        let result = run_worker(&host, root, invocation).await;
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch {
                return;
            }
            let Some(window) = ui_for_worker.upgrade() else {
                return;
            };
            window.set_extension_busy(false);
            match result {
                Ok(view) => {
                    let mut model = lock(&host.model);
                    model.values = form::initial_values(&view);
                    model.selected_files.clear();
                    model.form_error = None;
                    model.selected_extension = Some(extension_id);
                    model.selected_command = Some(command_id);
                    model.current_view = Some(view.clone());
                    drop(model);
                    view::present(&window, &view);
                }
                Err(error) => {
                    window.set_extension_actions(ModelRc::new(VecModel::default()));
                    window.set_extension_items(ModelRc::new(VecModel::default()));
                    window.set_extension_error_message(error.clone().into());
                    window.set_extension_status_message(error.into());
                    window.set_extension_view("error".into());
                }
            }
        });
    });
    lock(&controller.model).worker = Some(handle);
}

async fn run_worker(
    controller: &Controller,
    root: PathBuf,
    invocation: Invocation,
) -> Result<View, String> {
    let bundle = active_bundle(root.clone(), invocation.extension_id.clone())
        .await?
        .ok_or_else(|| "Extension is disabled, revoked or not installed".to_owned())?;
    let entrypoint = bundle
        .manifest
        .commands
        .iter()
        .find(|command| command.id == invocation.command_id)
        .ok_or_else(|| "The requested command is not declared in the active package".to_owned())?
        .entrypoint_id
        .clone();
    let context = broker_context(controller, &root, &bundle, invocation.selected_files)?;
    let broker = CapabilityBroker::new(context);
    let worker_binary = installed_binary("maestria-extension-worker")?;
    let args = [
        OsString::from("--bundle-root"),
        OsString::from("/extension"),
        OsString::from("--entrypoint-id"),
        OsString::from(entrypoint),
        OsString::from("--timeout-ms"),
        OsString::from("30000"),
    ];
    let args = args
        .iter()
        .map(OsString::as_os_str)
        .collect::<Vec<&OsStr>>();
    let command = sandbox::worker_command(&worker_binary, &bundle.code_directory, &args)
        .map_err(|error| format!("Cannot start extension OS sandbox: {error}"))?;
    let mut session = WorkerSession::spawn(command).map_err(|error| error.to_string())?;
    session
        .send(&invocation.message)
        .await
        .map_err(|error| error.to_string())?;
    let mut native_effect_used = false;
    let mut view = None;
    loop {
        let message = session.receive().await.map_err(|error| error.to_string())?;
        match message {
            WorkerMessage::CapabilityRequest {
                request_id,
                request,
                ..
            } => {
                let response = authorize_and_execute(
                    &root,
                    &bundle.extension_id,
                    &bundle.package,
                    &broker,
                    &request,
                    invocation.origin,
                    &mut native_effect_used,
                )
                .await;
                session
                    .send(&HostMessage::CapabilityResponse {
                        protocol_version: PROTOCOL_VERSION,
                        request_id,
                        response,
                    })
                    .await
                    .map_err(|error| error.to_string())?;
            }
            WorkerMessage::ViewUpdate { view: update, .. } if view.is_none() => view = Some(update),
            WorkerMessage::ViewUpdate { .. } => {
                return Err("Extension worker sent more than one final view".to_owned());
            }
            WorkerMessage::CommandComplete { .. } => {
                return view.ok_or_else(|| "Extension worker completed without a view".to_owned());
            }
        }
    }
}

async fn active_bundle(
    root: PathBuf,
    extension_id: String,
) -> Result<Option<ActiveBundle>, String> {
    tokio::task::spawn_blocking(move || BundleStore::open(root)?.active_bundle(&extension_id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

async fn authorize_and_execute(
    root: &Path,
    extension_id: &str,
    package: &PackageIdentity,
    broker: &CapabilityBroker,
    request: &CapabilityRequest,
    origin: InvocationOrigin,
    native_effect_used: &mut bool,
) -> CapabilityResponse {
    if origin == InvocationOrigin::ExplicitAction
        && matches!(
            request,
            CapabilityRequest::Open { .. } | CapabilityRequest::Copy { .. }
        )
        && std::mem::replace(native_effect_used, true)
    {
        return failure(
            request,
            "permission_denied",
            "An explicit action may perform only one native open or copy effect.",
        );
    }
    let active = active_bundle(root.to_owned(), extension_id.to_owned()).await;
    match active {
        Ok(Some(active)) if active.package == *package => {
            broker
                .execute(&active.granted_permissions, request, origin)
                .await
        }
        Ok(Some(_)) | Ok(None) => failure(
            request,
            "permission_denied",
            "Extension grants or package identity changed during invocation.",
        ),
        Err(_) => failure(
            request,
            "unavailable",
            "Extension grants could not be revalidated.",
        ),
    }
}

fn failure(request: &CapabilityRequest, code: &str, message: &str) -> CapabilityResponse {
    CapabilityResponse::Failure(CapabilityFailure {
        ok: false,
        capability: request.kind().to_owned(),
        error: CapabilityError {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    })
}

fn broker_context(
    controller: &Controller,
    root: &Path,
    bundle: &ActiveBundle,
    selected_files: BTreeMap<String, SelectedFile>,
) -> Result<BrokerContext, String> {
    let search = controller
        .state
        .settings()
        .map_err(|error| error.message)?
        .search_service();
    let launcher_realm = search.as_ref().map(|config| config.consumer_realm.clone());
    let mut context = BrokerContext::new(
        bundle.extension_id.clone(),
        launcher_realm,
        bundle.data_directory.clone(),
    )
    .map_err(|error| error.to_string())?;
    for (selection_id, file) in selected_files {
        context = context
            .with_selected_file(selection_id, file)
            .map_err(|error| error.to_string())?;
    }
    if let Some(search) = search {
        let credential = root
            .join("search-credentials")
            .join(&bundle.extension_id)
            .join("credential");
        if credential.exists() {
            let consumer = SearchConsumerConfig::for_extension(
                bundle.extension_id.clone(),
                search_realm(&bundle.extension_id),
                installed_binary("maestria-search")?,
                search.socket_path,
                credential,
            )
            .map_err(|error| error.to_string())?;
            context = context
                .with_search_consumer(consumer)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(context)
}

pub(super) fn search_realm(extension_id: &str) -> String {
    let digest = Sha256::digest(format!("sillage-extension-search-v1:{extension_id}").as_bytes());
    format!("{digest:x}")
}

fn installed_binary(name: &str) -> Result<PathBuf, String> {
    let installed = Path::new("/usr/bin").join(name);
    if installed.is_file() {
        return Ok(installed);
    }
    let sibling = std::env::current_exe()
        .map_err(|error| format!("Locate {name}: {error}"))?
        .with_file_name(name);
    if sibling.is_file() {
        Ok(sibling)
    } else {
        Err(format!(
            "{name} is not installed beside the launcher or in /usr/bin"
        ))
    }
}
