use std::sync::Arc;
use std::sync::atomic::Ordering;

use sillage_extensions::Permission;
use sillage_extensions::bundle::{BundleStore, ExtensionHealth};
use slint::{ComponentHandle, ModelRc, VecModel};

use super::{Controller, LauncherWindow, UiWeak, lock, management};
use crate::http_credentials::{
    HttpGrantMetadata, HttpGrantPolicy, HttpGrantState, HttpGrantStore, RevocationStatus,
};
use crate::{HttpIntegrationExtension, HttpIntegrationGrant};

mod approval;

#[derive(Default)]
pub(super) struct PanelState {
    pending: Option<HttpGrantPolicy>,
    renewal_handle: Option<String>,
}

pub(super) fn install_callbacks(ui: &LauncherWindow, controller: &Arc<Controller>) {
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_http_integrations_requested(move || open(&host, weak.clone()));
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_http_integrations_closed(move || {
        clear(&host, &weak);
        if let Some(window) = weak.upgrade() {
            window.set_extension_view("extensions".into());
        }
        management::refresh(&host, weak.clone());
    });
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_http_grant_reset_requested(move || approval::reset(&host, &weak));
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_http_grant_review_requested(move || approval::review(&host, weak.clone()));
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_http_grant_approve_requested(move || approval::approve(&host, weak.clone()));
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_http_grant_renew_requested(move |handle| approval::renew(&host, &weak, handle.as_str()));
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_http_grant_revoke_requested(move |handle| revoke(&host, weak.clone(), handle.as_str()));
    let weak = ui.as_weak();
    let host = Arc::clone(controller);
    ui.on_http_grant_copy_handle_requested(move |handle| {
        copy_handle(&host, &weak, handle.as_str())
    });
}

pub(super) fn clear(controller: &Controller, ui: &UiWeak) {
    *lock(&controller.http_credentials) = PanelState::default();
    if let Some(window) = ui.upgrade() {
        window.set_http_secret("".into());
        window.set_http_reviewed_scope("".into());
        window.set_http_integrations_open(false);
        window.set_http_busy(false);
    }
}

fn store(controller: &Controller) -> Result<HttpGrantStore, String> {
    Ok(HttpGrantStore::new(
        controller.store_root()?.join("http-credentials"),
    ))
}

fn set_error(ui: &UiWeak, error: impl Into<slint::SharedString>) {
    if let Some(window) = ui.upgrade() {
        window.set_http_busy(false);
        window.set_http_error_message(error.into());
    }
}

fn grant_rows(grants: Vec<HttpGrantMetadata>) -> Vec<HttpIntegrationGrant> {
    grants
        .into_iter()
        .map(|grant| HttpIntegrationGrant {
            handle: grant.handle.into(),
            title: grant.policy.provider_label.into(),
            scope: format!(
                "Bearer · {} {}{}",
                method_name(grant.policy.method),
                grant.policy.origin,
                grant.policy.path
            )
            .into(),
            details: format!(
                "Extension: {} · Package: {}\nCreated: Unix second {} · Expires: Unix second {}",
                grant.policy.extension_id,
                grant.policy.package_sha256,
                grant.created_at,
                grant.expires_at
            )
            .into(),
            state: match grant.state {
                HttpGrantState::Active => "Active",
                HttpGrantState::Expired => "Expired",
                HttpGrantState::Revoked => "Revoked",
            }
            .into(),
        })
        .collect()
}

fn method_name(method: sillage_extensions::HttpMethod) -> &'static str {
    match method {
        sillage_extensions::HttpMethod::Get => "GET (read)",
        sillage_extensions::HttpMethod::Post => "POST (write)",
    }
}

fn open(controller: &Arc<Controller>, ui: UiWeak) {
    controller.cancel_command();
    clear(controller, &ui);
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(_) => {
            set_error(&ui, "Private integration metadata is unavailable.");
            return;
        }
    };
    if let Some(window) = ui.upgrade() {
        window.set_http_integrations_open(true);
        window.set_extension_view("http-integrations".into());
        window.set_http_mode("list".into());
        window.set_http_error_message("".into());
        window.set_http_notice_message("".into());
        window.set_http_busy(true);
    }
    let host = Arc::clone(controller);
    let epoch = controller.epoch.load(Ordering::Acquire);
    controller.runtime.spawn(async move {
        let loaded = tokio::task::spawn_blocking(move || {
            let summaries = BundleStore::open(root.clone())
                .and_then(|store| store.list())
                .map_err(|_| "Approved extension packages are unavailable.".to_owned())?;
            let extensions = summaries
                .into_iter()
                .filter(|summary| {
                    summary.enabled
                        && matches!(summary.health, ExtensionHealth::Validated)
                        && summary
                            .granted_permissions
                            .iter()
                            .any(|permission| matches!(permission, Permission::Http { .. }))
                })
                .map(|summary| HttpIntegrationExtension {
                    id: summary.extension_id.into(),
                    title: format!("{} {}", summary.name, summary.package.version).into(),
                })
                .collect::<Vec<_>>();
            let grants = HttpGrantStore::new(root.join("http-credentials"))
                .list()
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((extensions, grants))
        })
        .await
        .map_err(|_| "Integration metadata could not be loaded.".to_owned())
        .and_then(|result| result);
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch {
                return;
            }
            let Some(window) = ui.upgrade() else { return };
            if !window.get_http_integrations_open() {
                return;
            }
            window.set_http_busy(false);
            match loaded {
                Ok((extensions, grants)) => {
                    window.set_http_extensions(ModelRc::new(VecModel::from(extensions)));
                    window.set_http_grants(ModelRc::new(VecModel::from(grant_rows(grants))));
                }
                Err(error) => window.set_http_error_message(error.into()),
            }
        });
    });
}

fn revoke(controller: &Arc<Controller>, ui: UiWeak, handle: &str) {
    let store = match store(controller) {
        Ok(store) => store,
        Err(_) => {
            set_error(&ui, "Private integration metadata is unavailable.");
            return;
        }
    };
    let handle = handle.to_owned();
    if let Some(window) = ui.upgrade() {
        window.set_http_busy(true);
    }
    let host = Arc::clone(controller);
    let epoch = controller.epoch.load(Ordering::Acquire);
    controller.runtime.spawn(async move {
        let result = store.revoke(&handle).await
            .and_then(|status| store.list().map(|grants| (status, grants)));
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch { return; }
            let Some(window) = ui.upgrade() else { return };
            if !window.get_http_integrations_open() { return; }
            window.set_http_busy(false);
            match result {
                Ok((status, grants)) => {
                    window.set_http_grants(ModelRc::new(VecModel::from(grant_rows(grants))));
                    window.set_http_error_message("".into());
                    window.set_http_notice_message(match status {
                        RevocationStatus::Deleted => "HTTP grant revoked; its stored secret was removed.",
                        RevocationStatus::Pending => "HTTP grant revoked; secure-store secret removal is pending. Requests remain denied.",
                    }.into());
                }
                Err(error) => window.set_http_error_message(error.to_string().into()),
            }
        });
    });
}

fn copy_handle(controller: &Controller, ui: &UiWeak, handle: &str) {
    let valid = store(controller)
        .and_then(|store| store.list().map_err(|error| error.to_string()))
        .is_ok_and(|grants| {
            grants
                .iter()
                .any(|grant| grant.handle == handle && grant.state == HttpGrantState::Active)
        });
    if !valid {
        set_error(
            ui,
            "Only a currently active integration reference can be copied.",
        );
        return;
    }
    match super::super::platform::copy_text(handle) {
        Ok(()) => {
            if let Some(window) = ui.upgrade() {
                window.set_http_notice_message(
                    "Opaque integration reference copied; no secret was copied.".into(),
                );
            }
        }
        Err(_) => set_error(ui, "The integration reference could not be copied."),
    }
}
