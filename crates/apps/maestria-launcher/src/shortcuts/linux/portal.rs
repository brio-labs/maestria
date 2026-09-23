use std::sync::Arc;

use super::ConfigureOutcome;
use super::helpers::{portal_error_was_denial, portal_trigger};
use super::state::{ExportedParent, Portal, PortalSessionState};
use super::status::{
    available_system_status, lock_state, set_status, unavailable_status, unconfigured_status,
};
use crate::model::{ShortcutConfigureAction, ShortcutControl};

pub(super) async fn configure(
    inner: &Arc<super::state::ShortcutInner>,
    session: &mut Option<PortalSessionState>,
    parent_identifier: Option<ExportedParent>,
    preferred: &str,
    explicit: bool,
) -> ConfigureOutcome {
    let trigger = match portal_trigger(preferred) {
        Ok(trigger) => trigger,
        Err(error) => {
            let status = unavailable_status(&error.message);
            set_status(inner, status.clone());
            return ConfigureOutcome {
                status,
                denied: false,
            };
        }
    };

    if explicit
        && session
            .as_ref()
            .is_some_and(|active| active.version < 2 && active.bound)
    {
        close_session(session).await;
    }

    if session.is_none() {
        match create_session(inner).await {
            Ok(active) => *session = Some(active),
            Err(message) => {
                let status = unavailable_status(&message);
                set_status(inner, status.clone());
                return ConfigureOutcome {
                    status,
                    denied: false,
                };
            }
        }
    }

    if session.is_none() {
        let status = unavailable_status("The global-shortcuts portal session is unavailable");
        set_status(inner, status.clone());
        return ConfigureOutcome {
            status,
            denied: false,
        };
    }

    let previously_bound = session.as_ref().is_some_and(|active| active.bound);
    if let Some(outcome) = bind_if_needed(
        inner,
        session,
        &trigger,
        parent_identifier.as_deref(),
        preferred,
    )
    .await
    {
        return outcome;
    }

    if explicit
        && previously_bound
        && session.as_ref().is_some_and(|active| active.version >= 2)
        && let Some(active) = session.as_mut()
    {
        return configure_existing(inner, active, parent_identifier, preferred).await;
    }

    let status = match session.as_ref().and_then(|active| {
        active
            .trigger_description
            .as_deref()
            .map(|description| (active, description))
    }) {
        Some((active, description)) => available_system_status(description, None, active.version),
        None => unavailable_status("The system has no launcher shortcut; retry setup"),
    };
    set_status(inner, status.clone());
    ConfigureOutcome {
        status,
        denied: false,
    }
}

async fn bind_if_needed(
    inner: &Arc<super::state::ShortcutInner>,
    session: &mut Option<PortalSessionState>,
    trigger: &str,
    parent: Option<&ashpd::WindowIdentifier>,
    preferred: &str,
) -> Option<ConfigureOutcome> {
    if session.as_ref().is_some_and(|active| active.bound) {
        return None;
    }
    let result = match session.as_mut() {
        Some(active) => bind_shortcut(active, trigger, parent).await,
        None => {
            let status = unavailable_status("The global-shortcuts portal session is unavailable");
            set_status(inner, status.clone());
            return Some(ConfigureOutcome {
                status,
                denied: false,
            });
        }
    };
    match result {
        Ok(description) => {
            if let Some(active) = session.as_mut() {
                active.bound = true;
                active.trigger_description = Some(description.clone());
                set_status(
                    inner,
                    available_system_status(&description, None, active.version),
                );
            }
            None
        }
        Err(error) => {
            let denied = portal_error_was_denial(&error);
            let status = if denied {
                unconfigured_status(
                    ShortcutControl::System,
                    preferred,
                    Some("Shortcut setup was cancelled"),
                    ShortcutConfigureAction::Setup,
                )
            } else {
                unavailable_status(&format!("Global shortcut setup failed: {error}"))
            };
            set_status(inner, status.clone());
            close_session(session).await;
            Some(ConfigureOutcome { status, denied })
        }
    }
}

async fn configure_existing(
    inner: &Arc<super::state::ShortcutInner>,
    active: &mut PortalSessionState,
    parent_identifier: Option<ExportedParent>,
    preferred: &str,
) -> ConfigureOutcome {
    if let Err(error) = active
        .portal
        .configure_shortcuts(
            &active.session,
            parent_identifier.as_deref(),
            None::<ashpd::ActivationToken>,
        )
        .await
    {
        let was_denied = portal_error_was_denial(&error);
        let (status, denied) = if was_denied {
            match active.trigger_description.as_deref() {
                Some(description) if !description.trim().is_empty() => (
                    available_system_status(
                        description,
                        Some("Shortcut configuration was cancelled"),
                        active.version,
                    ),
                    true,
                ),
                Some(_) => (
                    available_system_status(
                        "",
                        Some("Shortcut configuration was cancelled"),
                        active.version,
                    ),
                    false,
                ),
                None => (
                    unconfigured_status(
                        ShortcutControl::System,
                        preferred,
                        Some("Shortcut configuration was cancelled"),
                        ShortcutConfigureAction::Setup,
                    ),
                    true,
                ),
            }
        } else {
            (
                unavailable_status(&format!("Global shortcut configuration failed: {error}")),
                false,
            )
        };
        set_status(inner, status.clone());
        return ConfigureOutcome { status, denied };
    }

    match list_shortcut(active).await {
        Ok(Some(description)) => {
            active.trigger_description = Some(description.clone());
            let status = available_system_status(&description, None, active.version);
            set_status(inner, status.clone());
            ConfigureOutcome {
                status,
                denied: false,
            }
        }
        Ok(None) => {
            active.trigger_description = None;
            let status = unavailable_status("The system has no launcher shortcut; retry setup");
            set_status(inner, status.clone());
            ConfigureOutcome {
                status,
                denied: false,
            }
        }
        Err(error) => {
            let status = unavailable_status(&format!("Global shortcut status failed: {error}"));
            set_status(inner, status.clone());
            ConfigureOutcome {
                status,
                denied: false,
            }
        }
    }
}

pub(super) async fn close_session(session: &mut Option<PortalSessionState>) {
    if let Some(active) = session.take() {
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(1), active.session.close()).await;
    }
}

async fn create_session(
    inner: &Arc<super::state::ShortcutInner>,
) -> Result<PortalSessionState, String> {
    // Registry registration is once per portal service owner, before any portal
    // call on ashpd's shared connection. Rebinding is not a new D-Bus peer.
    let connection = ashpd::zbus::Connection::session()
        .await
        .map_err(|error| error.to_string())?;
    let rule = ashpd::zbus::MatchRule::builder()
        .msg_type(ashpd::zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|error| error.to_string())?
        .interface("org.freedesktop.DBus")
        .map_err(|error| error.to_string())?
        .member("NameOwnerChanged")
        .map_err(|error| error.to_string())?
        .add_arg("org.freedesktop.portal.Desktop")
        .map_err(|error| error.to_string())?
        .build();
    let owner = ashpd::zbus::MessageStream::for_match_rule(rule, &connection, Some(8))
        .await
        .map_err(|error| format!("portal owner monitoring failed: {error}"))?;
    let bus = ashpd::zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|error| error.to_string())?;
    let name = ashpd::zbus::names::WellKnownName::try_from("org.freedesktop.portal.Desktop")
        .map_err(|error| error.to_string())?;
    bus.start_service_by_name(name.clone(), 0)
        .await
        .map_err(|error| format!("the portal service could not start: {error}"))?;
    let owner_name = bus
        .get_name_owner(name.into())
        .await
        .map_err(|error| error.to_string())?
        .to_string();
    let needs_registration =
        lock_state(inner).registered_owner.as_deref() != Some(owner_name.as_str());
    if needs_registration {
        let app_id = ashpd::AppID::try_from(super::APP_ID).map_err(|error| error.to_string())?;
        ashpd::register_host_app(app_id).await.map_err(|error| {
            format!("the portal host application could not be registered: {error}")
        })?;
        lock_state(inner).registered_owner = Some(owner_name.clone());
    }
    let portal: Portal = ashpd::desktop::global_shortcuts::GlobalShortcuts::new()
        .await
        .map_err(|error| format!("the global-shortcuts portal is unavailable: {error}"))?;
    let version = portal
        .get_property::<u32>("version")
        .await
        .map_err(|error| format!("the portal version could not be read: {error}"))?;
    let activated = Box::pin(
        portal
            .receive_activated()
            .await
            .map_err(|error| format!("portal activation subscription failed: {error}"))?,
    );
    let changed = Box::pin(
        portal
            .receive_shortcuts_changed()
            .await
            .map_err(|error| format!("portal shortcut subscription failed: {error}"))?,
    );
    let session = portal
        .create_session()
        .await
        .map_err(|error| format!("the global-shortcuts portal session could not start: {error}"))?;
    let closed = match session.receive_closed().await {
        Ok(stream) => Box::pin(stream),
        Err(error) => {
            let _ = session.close().await;
            return Err(format!("portal session subscription failed: {error}"));
        }
    };
    // ashpd exposes Session's identity through Serialize, not a public path getter.
    let session_path = match serde_json::to_value(&session) {
        Ok(serde_json::Value::String(path)) => path,
        _ => {
            let _ = session.close().await;
            return Err("The portal session identity could not be read".to_string());
        }
    };
    Ok(PortalSessionState {
        portal,
        version,
        owner_name,
        session_path,
        session,
        activated,
        changed,
        closed,
        owner,
        bound: false,
        trigger_description: None,
    })
}

async fn bind_shortcut(
    active: &mut PortalSessionState,
    trigger: &str,
    identifier: Option<&ashpd::WindowIdentifier>,
) -> Result<String, ashpd::Error> {
    let shortcut = ashpd::desktop::global_shortcuts::NewShortcut::new(
        super::SHORTCUT_ID,
        "Activate Maestria Launcher",
    )
    .preferred_trigger(Some(trigger));
    let request = active
        .portal
        .bind_shortcuts(&active.session, &[shortcut], identifier)
        .await?;
    let response = request.response()?;
    response
        .shortcuts()
        .iter()
        .find(|shortcut| shortcut.id() == super::SHORTCUT_ID)
        .map(|shortcut| shortcut.trigger_description().to_string())
        .ok_or(ashpd::Error::NoResponse)
}

async fn list_shortcut(active: &mut PortalSessionState) -> Result<Option<String>, ashpd::Error> {
    let request = active.portal.list_shortcuts(&active.session).await?;
    let response = request.response()?;
    Ok(response
        .shortcuts()
        .iter()
        .find(|shortcut| shortcut.id() == super::SHORTCUT_ID)
        .map(|shortcut| shortcut.trigger_description().to_string()))
}
