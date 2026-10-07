use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc as async_mpsc;

use super::{
    APP_ID, DEFAULT_SHORTCUT, PORTAL_DEFAULT_TRIGGER, PortalCommand, SHORTCUT_ID,
    available_system_status, lock, unavailable_status, unconfigured_status,
};
use crate::model::{ShortcutConfigureAction, ShortcutControl, ShortcutStatus};
struct PortalActive {
    portal: ashpd::desktop::global_shortcuts::GlobalShortcuts<'static>,
    version: u32,
    session: ashpd::desktop::Session<
        'static,
        ashpd::desktop::global_shortcuts::GlobalShortcuts<'static>,
    >,
    session_path: String,
    activated: Pin<
        Box<dyn futures_util::Stream<Item = ashpd::desktop::global_shortcuts::Activated> + Send>,
    >,
    changed: Pin<
        Box<
            dyn futures_util::Stream<Item = ashpd::desktop::global_shortcuts::ShortcutsChanged>
                + Send,
        >,
    >,
    closed: Pin<Box<dyn futures_util::Stream<Item = ()> + Send>>,
    bound: bool,
    description: Option<String>,
}

enum PortalEvent {
    Command(Option<PortalCommand>),
    Activated(Option<ashpd::desktop::global_shortcuts::Activated>),
    Changed(Option<ashpd::desktop::global_shortcuts::ShortcutsChanged>),
    Closed,
}
fn portal_trigger(value: &str) -> Result<String, String> {
    super::x11::validate_accelerator(value)?;
    if value.eq_ignore_ascii_case(DEFAULT_SHORTCUT) {
        return Ok(PORTAL_DEFAULT_TRIGGER.to_string());
    }
    let tokens = value.split('+').map(str::trim).collect::<Vec<_>>();
    let mut output = Vec::with_capacity(tokens.len());
    for (index, token) in tokens.iter().enumerate() {
        if index + 1 == tokens.len() {
            output.push(match token.to_ascii_lowercase().as_str() {
                "space" => "space".to_string(),
                other => other.to_string(),
            });
        } else {
            output.push(match token.to_ascii_uppercase().as_str() {
                "CTRL" | "CONTROL" => "CTRL".to_string(),
                "ALT" | "OPTION" => "ALT".to_string(),
                "SHIFT" => "SHIFT".to_string(),
                "SUPER" | "META" | "CMD" | "COMMAND" => "SUPER".to_string(),
                _ => return Err("unknown shortcut modifier".to_string()),
            });
        }
    }
    Ok(output.join("+"))
}
pub(super) async fn portal_worker(
    mut receiver: async_mpsc::Receiver<PortalCommand>,
    status: Arc<Mutex<ShortcutStatus>>,
    activation: mpsc::SyncSender<()>,
) {
    let mut active: Option<PortalActive> = None;
    loop {
        match next_portal_event(&mut active, &mut receiver).await {
            PortalEvent::Command(Some(PortalCommand::Configure {
                preferred,
                explicit,
                parent,
                response,
            })) => {
                let configured =
                    configure_portal(&mut active, &preferred, explicit, parent.as_deref()).await;
                *lock(&status) = configured.clone();
                let _ = response.send(configured);
            }
            PortalEvent::Command(Some(PortalCommand::Clear { response })) => {
                close_portal(&mut active).await;
                let cleared = unconfigured_status(
                    ShortcutControl::System,
                    DEFAULT_SHORTCUT,
                    Some("The desktop controls shortcut approval and activation."),
                    ShortcutConfigureAction::Setup,
                );
                *lock(&status) = cleared.clone();
                let _ = response.send(cleared);
            }
            PortalEvent::Command(Some(PortalCommand::Shutdown)) | PortalEvent::Command(None) => {
                close_portal(&mut active).await;
                return;
            }
            PortalEvent::Activated(Some(event)) => {
                if event.shortcut_id() == SHORTCUT_ID
                    && active.as_ref().is_some_and(|session| {
                        event.session_handle().as_str() == session.session_path
                    })
                {
                    let _ = activation.try_send(());
                }
            }
            PortalEvent::Changed(Some(event)) => {
                if let Some(session) = active.as_mut()
                    && event.session_handle().as_str() == session.session_path
                {
                    session.description = event
                        .shortcuts()
                        .iter()
                        .find(|shortcut| shortcut.id() == SHORTCUT_ID)
                        .map(|shortcut| shortcut.trigger_description().to_string());
                    let updated = session.description.as_deref().map_or_else(
                        || {
                            unavailable_status(
                                "The system removed the launcher shortcut; retry setup",
                            )
                        },
                        |description| available_system_status(description, session.version),
                    );
                    *lock(&status) = updated;
                }
            }
            PortalEvent::Changed(None) | PortalEvent::Activated(None) | PortalEvent::Closed => {
                close_portal(&mut active).await;
                *lock(&status) =
                    unavailable_status("The global-shortcuts portal disconnected; retry setup");
            }
        }
    }
}
async fn next_portal_event(
    active: &mut Option<PortalActive>,
    receiver: &mut async_mpsc::Receiver<PortalCommand>,
) -> PortalEvent {
    if let Some(session) = active.as_mut() {
        tokio::select! {
            command = receiver.recv() => PortalEvent::Command(command),
            activated = session.activated.next() => PortalEvent::Activated(activated),
            changed = session.changed.next() => PortalEvent::Changed(changed),
            _ = session.closed.next() => PortalEvent::Closed,
        }
    } else {
        PortalEvent::Command(receiver.recv().await)
    }
}
async fn reconfigure_bound_portal(
    session: &mut PortalActive,
    parent: Option<&ashpd::WindowIdentifier>,
) -> ShortcutStatus {
    if let Err(error) = session
        .portal
        .configure_shortcuts(&session.session, parent, None::<ashpd::ActivationToken>)
        .await
    {
        if portal_error_was_denial(&error) {
            return available_system_status(
                session
                    .description
                    .as_deref()
                    .map_or("", |description| description),
                session.version,
            );
        }
        return unavailable_status(&format!("Global shortcut configuration failed: {error}"));
    }
    match list_portal_shortcut(session).await {
        Ok(Some(description)) => {
            session.description = Some(description.clone());
            available_system_status(&description, session.version)
        }
        Ok(None) => unavailable_status("The system has no launcher shortcut; retry setup"),
        Err(error) => unavailable_status(&format!("Global shortcut status failed: {error}")),
    }
}

async fn configure_portal(
    active: &mut Option<PortalActive>,
    preferred: &str,
    explicit: bool,
    parent: Option<&ashpd::WindowIdentifier>,
) -> ShortcutStatus {
    let trigger = match portal_trigger(preferred) {
        Ok(trigger) => trigger,
        Err(error) => return unavailable_status(&error),
    };
    if explicit
        && active
            .as_ref()
            .is_some_and(|session| session.bound && session.version < 2)
    {
        close_portal(active).await;
    }
    if active.is_none() {
        match create_portal_session().await {
            Ok(session) => *active = Some(session),
            Err(error) => return unavailable_status(&error),
        }
    }
    let Some(session) = active.as_mut() else {
        return unavailable_status("The global-shortcuts portal session is unavailable");
    };
    if explicit && session.bound && session.version >= 2 {
        return reconfigure_bound_portal(session, parent).await;
    }
    if session.bound {
        return available_system_status(
            session
                .description
                .as_deref()
                .map_or("", |description| description),
            session.version,
        );
    }
    let shortcut = ashpd::desktop::global_shortcuts::NewShortcut::new(
        SHORTCUT_ID,
        "Activate Sillage Launcher",
    )
    .preferred_trigger(Some(trigger.as_str()));
    let response = match session
        .portal
        .bind_shortcuts(&session.session, &[shortcut], parent)
        .await
        .and_then(|request| request.response())
    {
        Ok(response) => response,
        Err(error) => {
            let was_denied = portal_error_was_denial(&error);
            close_portal(active).await;
            return if was_denied {
                unconfigured_status(
                    ShortcutControl::System,
                    preferred,
                    Some("Shortcut setup was cancelled"),
                    ShortcutConfigureAction::Setup,
                )
            } else {
                unavailable_status(&format!("Global shortcut setup failed: {error}"))
            };
        }
    };
    let description = response
        .shortcuts()
        .iter()
        .find(|shortcut| shortcut.id() == SHORTCUT_ID)
        .map(|shortcut| shortcut.trigger_description().to_string());
    match description {
        Some(description) => {
            session.bound = true;
            session.description = Some(description.clone());
            available_system_status(&description, session.version)
        }
        None => {
            close_portal(active).await;
            unavailable_status("The portal did not return a launcher shortcut")
        }
    }
}
async fn create_portal_session() -> Result<PortalActive, String> {
    let app_id = ashpd::AppID::try_from(APP_ID).map_err(|error| error.to_string())?;
    ashpd::register_host_app(app_id)
        .await
        .map_err(|error| format!("The portal host application could not be registered: {error}"))?;
    let portal = ashpd::desktop::global_shortcuts::GlobalShortcuts::new()
        .await
        .map_err(|error| format!("The global-shortcuts portal is unavailable: {error}"))?;
    let version = portal
        .get_property::<u32>("version")
        .await
        .map_err(|error| format!("The portal version could not be read: {error}"))?;
    let activated = Box::pin(
        portal
            .receive_activated()
            .await
            .map_err(|error| format!("Portal activation subscription failed: {error}"))?,
    );
    let changed = Box::pin(
        portal
            .receive_shortcuts_changed()
            .await
            .map_err(|error| format!("Portal shortcut subscription failed: {error}"))?,
    );
    let session = portal
        .create_session()
        .await
        .map_err(|error| format!("The global-shortcuts portal session could not start: {error}"))?;
    let closed = match session.receive_closed().await {
        Ok(stream) => Box::pin(stream),
        Err(error) => {
            let _ = session.close().await;
            return Err(format!("Portal session subscription failed: {error}"));
        }
    };
    let session_path = match serde_json::to_value(&session) {
        Ok(serde_json::Value::String(path)) => path,
        _ => {
            let _ = session.close().await;
            return Err("The portal session identity could not be read".to_string());
        }
    };
    Ok(PortalActive {
        portal,
        version,
        session,
        session_path,
        activated,
        changed,
        closed,
        bound: false,
        description: None,
    })
}
async fn list_portal_shortcut(session: &mut PortalActive) -> Result<Option<String>, ashpd::Error> {
    let request = session.portal.list_shortcuts(&session.session).await?;
    let response = request.response()?;
    Ok(response
        .shortcuts()
        .iter()
        .find(|shortcut| shortcut.id() == SHORTCUT_ID)
        .map(|shortcut| shortcut.trigger_description().to_string()))
}
async fn close_portal(active: &mut Option<PortalActive>) {
    if let Some(session) = active.take() {
        let _ = tokio::time::timeout(Duration::from_secs(1), session.session.close()).await;
    }
}
fn portal_error_was_denial(error: &ashpd::Error) -> bool {
    matches!(
        error,
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)
            | ashpd::Error::Portal(ashpd::PortalError::Cancelled(_))
            | ashpd::Error::Portal(ashpd::PortalError::NotAllowed(_))
    )
}
