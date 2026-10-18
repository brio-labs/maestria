use std::sync::Arc;
use std::sync::atomic::Ordering;

use sillage_extensions::bundle::{ActiveBundle, BundleStore};
use sillage_extensions::{CapabilityRequest, HttpMethod, InvocationOrigin, authorize};
use slint::{ModelRc, VecModel};

use super::{
    Controller, HttpGrantPolicy, PanelState, UiWeak, grant_rows, lock, method_name, set_error,
    store,
};
use crate::http_credentials::SecretBytes;

pub(super) fn reset(controller: &Controller, ui: &UiWeak) {
    *lock(&controller.http_credentials) = PanelState::default();
    if let Some(window) = ui.upgrade() {
        window.set_http_secret("".into());
        window.set_http_reviewed_scope("".into());
        window.set_http_extension_id("".into());
        window.set_http_provider_label("".into());
        window.set_http_origin("".into());
        window.set_http_path("/".into());
        window.set_http_method("GET".into());
        window.set_http_ttl("3600".into());
        window.set_http_error_message("".into());
        window.set_http_notice_message("".into());
        window.set_http_mode("edit".into());
    }
}

pub(super) fn renew(controller: &Controller, ui: &UiWeak, handle: &str) {
    let grant = store(controller)
        .and_then(|store| store.list().map_err(|error| error.to_string()))
        .and_then(|grants| {
            grants
                .into_iter()
                .find(|grant| grant.handle == handle)
                .ok_or_else(|| "The integration grant is unavailable.".to_owned())
        });
    let grant = match grant {
        Ok(grant) => grant,
        Err(error) => {
            set_error(ui, error);
            return;
        }
    };
    reset(controller, ui);
    lock(&controller.http_credentials).renewal_handle = Some(grant.handle);
    if let Some(window) = ui.upgrade() {
        window.set_http_extension_id(grant.policy.extension_id.into());
        window.set_http_provider_label(grant.policy.provider_label.into());
        window.set_http_origin(grant.policy.origin.into());
        window.set_http_path(grant.policy.path.into());
        window.set_http_method(
            match grant.policy.method {
                HttpMethod::Get => "GET",
                HttpMethod::Post => "POST",
            }
            .into(),
        );
        window.set_http_ttl(grant.policy.expires_in_seconds.to_string().into());
        window.set_http_notice_message("Renewal needs a fresh scope review and host-only secret entry. Approval revokes the old grant before creating the renewed grant.".into());
    }
}

async fn active_package(root: std::path::PathBuf, id: String) -> Result<ActiveBundle, String> {
    tokio::task::spawn_blocking(move || BundleStore::open(root)?.active_bundle(&id))
        .await
        .map_err(|_| "The extension package could not be revalidated.".to_owned())?
        .map_err(|_| "The approved extension package is unavailable.".to_owned())?
        .ok_or_else(|| {
            "Install and approve this extension package before granting HTTP credentials."
                .to_owned()
        })
}

fn check_permission(bundle: &ActiveBundle, policy: &HttpGrantPolicy) -> Result<(), String> {
    let request = CapabilityRequest::Http {
        url: format!("{}{}", policy.origin, policy.path),
        method: policy.method,
        authentication: None,
        body: None,
    };
    authorize(
        &bundle.granted_permissions,
        &request,
        InvocationOrigin::ExplicitAction,
    )
    .map_err(|_| {
        "This HTTPS origin is outside the extension's approved HTTP permission.".to_owned()
    })
}

pub(super) fn review(controller: &Arc<Controller>, ui: UiWeak) {
    let Some(window) = ui.upgrade() else { return };
    let method = match window.get_http_method().as_str() {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        _ => {
            set_error(&ui, "Select a supported HTTP method.");
            return;
        }
    };
    let expires_in_seconds = match window.get_http_ttl().as_str().parse::<u64>() {
        Ok(value) => value,
        Err(_) => {
            set_error(&ui, "Enter a bounded whole-second credential lifetime.");
            return;
        }
    };
    let mut policy = HttpGrantPolicy {
        provider_label: window.get_http_provider_label().to_string(),
        extension_id: window.get_http_extension_id().to_string(),
        package_sha256: String::new(),
        origin: window.get_http_origin().to_string(),
        method,
        path: window.get_http_path().to_string(),
        expires_in_seconds,
    };
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(_) => {
            set_error(&ui, "Private integration metadata is unavailable.");
            return;
        }
    };
    window.set_http_secret("".into());
    window.set_http_busy(true);
    window.set_http_error_message("".into());
    let host = Arc::clone(controller);
    let epoch = controller.epoch.load(Ordering::Acquire);
    controller.runtime.spawn(async move {
        let result = async {
            let bundle = active_package(root, policy.extension_id.clone()).await?;
            policy.package_sha256 = bundle.package.sha256.clone();
            let reviewed = store(&host)?.review(&policy).map_err(|error| error.to_string())?;
            check_permission(&bundle, &reviewed)?;
            Ok::<_, String>(reviewed)
        }.await;
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch { return; }
            let Some(window) = ui.upgrade() else { return };
            if !window.get_http_integrations_open() { return; }
            window.set_http_busy(false);
            match result {
                Ok(policy) => {
                    let renewal = lock(&host.http_credentials).renewal_handle.is_some();
                    let summary = format!("Provider: {}\nExtension: {}\nExact package SHA-256: {}\nScheme: Bearer\nAudience: {}\nAction: {} at exactly {}\nExpiry: {} seconds from explicit approval.{}\nNo secret is sent to extension code; an unlocked desktop Secret Service is required.",
                        policy.provider_label, policy.extension_id,
                        policy.package_sha256, policy.origin,
                        method_name(policy.method), policy.path, policy.expires_in_seconds,
                        if renewal { " The previous grant will be revoked." } else { "" });
                    lock(&host.http_credentials).pending = Some(policy);
                    window.set_http_reviewed_scope(summary.into());
                    window.set_http_mode("review".into());
                }
                Err(error) => window.set_http_error_message(error.into()),
            }
        });
    });
}

pub(super) fn approve(controller: &Arc<Controller>, ui: UiWeak) {
    let Some(window) = ui.upgrade() else { return };
    let (policy, renewal_handle) = {
        let mut model = lock(&controller.http_credentials);
        let Some(policy) = model.pending.take() else {
            window.set_http_secret("".into());
            set_error(&ui, "Review the exact credential scope before approval.");
            return;
        };
        (policy, model.renewal_handle.take())
    };
    let secret = SecretBytes::new(window.get_http_secret().as_bytes().to_vec());
    window.set_http_secret("".into());
    window.set_http_reviewed_scope("".into());
    window.set_http_busy(true);
    let root = match controller.store_root() {
        Ok(root) => root,
        Err(_) => {
            set_error(&ui, "Private integration metadata is unavailable.");
            return;
        }
    };
    let host = Arc::clone(controller);
    let epoch = controller.epoch.load(Ordering::Acquire);
    controller.runtime.spawn(async move {
        let result = async {
            let bundle = active_package(root, policy.extension_id.clone()).await?;
            if bundle.package.sha256 != policy.package_sha256 {
                return Err("The extension package changed. Review its new identity before approval.".to_owned());
            }
            check_permission(&bundle, &policy)?;
            let store = store(&host)?;
            let replacing = renewal_handle.is_some();
            if let Some(handle) = renewal_handle {
                store.revoke(&handle).await.map_err(|error| error.to_string())?;
            }
            store.create(policy, secret).await.map_err(|error| {
                if replacing { format!("Old grant revoked; renewal was not created: {error}") } else { error.to_string() }
            })?;
            store.list().map_err(|error| error.to_string())
        }.await;
        let _ = slint::invoke_from_event_loop(move || {
            if host.epoch.load(Ordering::Acquire) != epoch { return; }
            let Some(window) = ui.upgrade() else { return };
            if !window.get_http_integrations_open() { return; }
            window.set_http_busy(false);
            window.set_http_mode("list".into());
            match result {
                Ok(grants) => {
                    window.set_http_grants(ModelRc::new(VecModel::from(grant_rows(grants))));
                    window.set_http_error_message("".into());
                    window.set_http_notice_message("Reviewed HTTP credential grant created. Copy only its opaque handle for the extension; the secret stays in the desktop credential store.".into());
                }
                Err(error) => window.set_http_error_message(error.into()),
            }
        });
    });
}
