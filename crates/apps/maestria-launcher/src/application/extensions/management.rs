use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use maestria_extensions::Permission;
use maestria_extensions::bundle::{
    BundleError, BundleStore, ExtensionHealth, ExtensionSummary, InstallApproval,
};
use slint::{ModelRc, VecModel};

use super::{Controller, PendingInstall, UiWeak, lock, set_error};
use crate::{ExtensionItemRow, ExtensionRow};

#[derive(Clone, Copy)]
pub(super) enum Mutation {
    Disable,
    Revoke,
    Remove { retain_data: bool },
}

async fn store_operation<R: Send + 'static>(
    root: PathBuf,
    operation: impl FnOnce(&BundleStore) -> Result<R, BundleError> + Send + 'static,
) -> Result<R, String> {
    tokio::task::spawn_blocking(move || {
        let store = BundleStore::open(root)?;
        operation(&store)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

pub(super) fn refresh(controller: &Arc<Controller>, ui: UiWeak) {
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(error) => {
            set_error(&ui, error);
            return;
        }
    };
    if let Some(window) = ui.upgrade() {
        window.set_extension_busy(true);
        window.set_extension_error_message("".into());
    }
    let host = Arc::clone(controller);
    let epoch = controller.epoch.load(Ordering::Acquire);
    controller.runtime.spawn(async move {
        let result = store_operation(root, BundleStore::list).await;
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch {
                return;
            }
            let Some(window) = ui.upgrade() else { return };
            window.set_extension_busy(false);
            match result.and_then(|summaries| extension_rows(&summaries)) {
                Ok(rows) => window.set_extension_rows(ModelRc::new(VecModel::from(rows))),
                Err(error) => window.set_extension_error_message(error.into()),
            }
        });
    });
}

pub(super) fn preview(controller: &Arc<Controller>, ui: UiWeak, source: PathBuf) {
    if !source.is_absolute() {
        set_error(
            &ui,
            "Choose an absolute local extension package path".to_owned(),
        );
        return;
    }
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(error) => {
            set_error(&ui, error);
            return;
        }
    };
    controller.cancel_command();
    lock(&controller.model).pending = None;
    let archive = source
        .extension()
        .is_some_and(|extension| extension == "zip");
    let epoch = controller.epoch.load(Ordering::Acquire);
    if let Some(window) = ui.upgrade() {
        window.set_extension_busy(true);
        window.set_extension_error_message("".into());
        window.set_extension_notice_message("".into());
    }
    let host = Arc::clone(controller);
    controller.runtime.spawn(async move {
        let preview_path = source.clone();
        let grant_root = root.clone();
        let result = store_operation(root, move |store| {
            if archive {
                store.preview_archive(&preview_path)
            } else {
                store.preview_directory(&preview_path)
            }
        })
        .await;
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch {
                return;
            }
            let Some(window) = ui.upgrade() else { return };
            window.set_extension_busy(false);
            match result.and_then(|approval| {
                let description = approval_detail(&approval, &grant_root)?;
                Ok((approval, description))
            }) {
                Ok((approval, description)) => {
                    window.set_extension_selected_id(approval.manifest.id.as_str().into());
                    window.set_extension_view_title(
                        format!("{} {}", approval.manifest.name, approval.package.version).into(),
                    );
                    window.set_extension_detail_content(description.into());
                    window.set_extension_view("approval".into());
                    lock(&host.model).pending = Some(PendingInstall {
                        source,
                        archive,
                        approval,
                    });
                }
                Err(error) => window.set_extension_error_message(error.into()),
            }
        });
    });
}

pub(super) fn approve(controller: &Arc<Controller>, ui: UiWeak, id: &str) {
    if id.is_empty() {
        lock(&controller.model).pending = None;
        if let Some(window) = ui.upgrade() {
            window.set_extension_view("extensions".into());
            window.set_extension_selected_id("".into());
            window.set_extension_notice_message(
                "Installation cancelled without installing a package.".into(),
            );
        }
        return;
    }
    let pending = {
        let mut model = lock(&controller.model);
        if model
            .pending
            .as_ref()
            .is_some_and(|pending| pending.approval.manifest.id == id)
        {
            model.pending.take()
        } else {
            None
        }
    };
    let Some(PendingInstall {
        source,
        archive,
        approval,
    }) = pending
    else {
        set_error(
            &ui,
            "No matching installation proposal is pending".to_owned(),
        );
        return;
    };
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(error) => {
            set_error(&ui, error);
            return;
        }
    };
    if let Some(window) = ui.upgrade() {
        window.set_extension_busy(true);
        window.set_extension_error_message("".into());
    }
    let epoch = controller.epoch.load(Ordering::Acquire);
    let host = Arc::clone(controller);
    controller.runtime.spawn(async move {
        let result = store_operation(root, move |store| {
            if archive {
                store.install_archive(&source, |actual| actual.matches(&approval))
            } else {
                store.install_directory(&source, |actual| actual.matches(&approval))
            }
        })
        .await;
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch {
                return;
            }
            let Some(window) = ui.upgrade() else { return };
            window.set_extension_busy(false);
            match result {
                Ok(receipt) => {
                    window.set_extension_view("extensions".into());
                    window.set_extension_selected_id(receipt.extension_id.as_str().into());
                    window.set_extension_notice_message(
                        format!(
                            "Installed {} {} with explicitly reviewed grants.",
                            receipt.extension_id, receipt.package.version
                        )
                        .into(),
                    );
                    refresh(&host, ui);
                }
                Err(error) => window.set_extension_error_message(error.into()),
            }
        });
    });
}

pub(super) fn mutate(controller: &Arc<Controller>, ui: UiWeak, id: &str, mutation: Mutation) {
    if id.is_empty() {
        set_error(&ui, "Select an installed extension first".to_owned());
        return;
    }
    controller.cancel_command();
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(error) => {
            set_error(&ui, error);
            return;
        }
    };
    if let Some(window) = ui.upgrade() {
        window.set_extension_busy(true);
        window.set_extension_error_message("".into());
    }
    let id = id.to_owned();
    let epoch = controller.epoch.load(Ordering::Acquire);
    let host = Arc::clone(controller);
    controller.runtime.spawn(async move {
        let operation_id = id.clone();
        let result = store_operation(root, move |store| match mutation {
            Mutation::Disable => store.disable(&operation_id),
            Mutation::Revoke => store.revoke(&operation_id),
            Mutation::Remove { retain_data } => store.uninstall(&operation_id, retain_data),
        }).await;
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch { return; }
            let Some(window) = ui.upgrade() else { return };
            window.set_extension_busy(false);
            match result {
                Ok(()) => {
                    window.set_extension_view("extensions".into());
                    window.set_extension_items(ModelRc::new(VecModel::default()));
                    window.set_extension_selected_id("".into());
                    let notice = match mutation {
                        Mutation::Disable => format!("Disabled {id}; commands cannot run."),
                        Mutation::Revoke => format!("Revoked extension grants for {id}; any separate provider search grant must be revoked by its owner."),
                        Mutation::Remove { retain_data: true } => format!("Removed {id}; private data was retained."),
                        Mutation::Remove { retain_data: false } => format!("Removed {id} and deleted its private data."),
                    };
                    window.set_extension_notice_message(notice.into());
                    refresh(&host, ui);
                }
                Err(error) => window.set_extension_error_message(error.into()),
            }
        });
    });
}

pub(super) fn select(controller: &Arc<Controller>, ui: UiWeak, id: &str) {
    controller.cancel_command();
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(error) => {
            set_error(&ui, error);
            return;
        }
    };
    let id = id.to_owned();
    let epoch = controller.epoch.load(Ordering::Acquire);
    let host = Arc::clone(controller);
    if let Some(window) = ui.upgrade() {
        window.set_extension_busy(true);
        window.set_extension_items(ModelRc::new(VecModel::default()));
    }
    controller.runtime.spawn(async move {
        let selected_id = id.clone();
        let result = store_operation(root, move |store| store.active_bundle(&selected_id)).await;
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch { return; }
            let Some(window) = ui.upgrade() else { return };
            window.set_extension_busy(false);
            match result {
                Ok(Some(bundle)) => {
                    let rows = bundle.manifest.commands.iter().map(|command| ExtensionItemRow {
                        id: command.id.as_str().into(),
                        title: command.title.as_str().into(),
                        subtitle: match command.description.as_deref() {
                            Some(description) if !description.is_empty() => description.into(),
                            _ => "".into(),
                        },
                    }).collect::<Vec<_>>();
                    window.set_extension_items(ModelRc::new(VecModel::from(rows)));
                    lock(&host.model).selected_extension = Some(id);
                }
                Ok(None) => {
                    window.set_extension_items(ModelRc::new(VecModel::default()));
                    window.set_extension_notice_message("This extension is disabled or its grants were revoked; reinstall to approve a new package.".into());
                }
                Err(error) => window.set_extension_error_message(error.into()),
            }
        });
    });
}

fn extension_rows(summaries: &[ExtensionSummary]) -> Result<Vec<ExtensionRow>, String> {
    summaries
        .iter()
        .map(|summary| {
            let status = match &summary.health {
                ExtensionHealth::Invalid(error) => format!("Invalid: {error}"),
                ExtensionHealth::Validated if summary.enabled => "Enabled".to_owned(),
                ExtensionHealth::Validated
                    if summary.granted_permissions.is_empty()
                        && !summary.requested_permissions.is_empty() =>
                {
                    "Revoked".to_owned()
                }
                ExtensionHealth::Validated => "Disabled".to_owned(),
            };
            Ok(ExtensionRow {
                id: summary.extension_id.as_str().into(),
                title: format!("{} {}", summary.name, summary.package.version).into(),
                status: status.into(),
                permissions: describe_permissions(&summary.requested_permissions)?.into(),
            })
        })
        .collect()
}

fn describe_permissions(permissions: &[Permission]) -> Result<String, String> {
    if permissions.is_empty() {
        return Ok("none".to_owned());
    }
    permissions
        .iter()
        .map(|permission| serde_json::to_string(permission).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()
        .map(|lines| lines.join("\n"))
}

fn approval_detail(approval: &InstallApproval, root: &Path) -> Result<String, String> {
    let kind = if approval.first_install {
        "New installation"
    } else {
        "Update or reapproval"
    };
    let mut description = format!(
        "{kind}\nExtension ID: {}\nVersion: {}\nSHA-256 package identity: {}\n\nRequested permissions:\n{}\n\nNewly granted permissions:\n{}\n\nPreviously granted permissions:\n{}\n\nRemoved permissions:\n{}\n\nOnly this exact validated package and permission change will be installed. If the source changes, installation is denied before any package or state write.",
        approval.manifest.id,
        approval.package.version,
        approval.package.sha256,
        describe_permissions(&approval.permissions.requested)?,
        describe_permissions(&approval.permissions.added)?,
        describe_permissions(&approval.permissions.previously_granted)?,
        describe_permissions(&approval.permissions.removed)?,
    );
    if approval
        .manifest
        .permissions
        .iter()
        .any(|permission| matches!(permission, Permission::FileSearch { .. }))
    {
        let credential = root
            .join("search-credentials")
            .join(&approval.manifest.id)
            .join("credential");
        description.push_str(&format!(
            "\n\nDocument search additionally requires a separate owner-issued, bounded daemon search grant. An owner must create a private credential directory and issue a `maestria-search owner grant create-external` grant with consumer realm {} and credential file {}. This extension never receives the launcher credential. Revoking extension grants does not revoke a separate provider grant; its owner must revoke that grant's token digest.",
            super::invoke::search_realm(&approval.manifest.id),
            credential.display(),
        ));
    }
    Ok(description)
}
