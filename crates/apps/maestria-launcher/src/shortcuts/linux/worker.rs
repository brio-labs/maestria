use std::sync::Arc;

use futures_util::{StreamExt, TryStreamExt};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use tauri::{AppHandle, Manager};
use tokio::sync::{Notify, mpsc, oneshot};

use super::ConfigureOutcome;
use super::helpers::dispatch_activation;
use super::portal;
use super::state::{
    CompletionGuard, ParentReceiver, PortalCommand, PortalEvent, PortalSessionState, PortalWorker,
    ShortcutInner,
};
use super::status::{available_system_status, lock_state, set_status, unavailable_status};
use crate::errors::LauncherError;
use crate::model::LAUNCHER_WINDOW_LABEL;

pub(super) fn parent_receiver_on_main(app: &AppHandle) -> ParentReceiver {
    let (sender, receiver) = oneshot::channel();
    let dispatcher = app.clone();
    let activation_app = app.clone();
    let _ = dispatcher.run_on_main_thread(move || {
        let Some(window) = activation_app.get_webview_window(LAUNCHER_WINDOW_LABEL) else {
            let _ = sender.send(None);
            return;
        };
        let handles = match (window.window_handle(), window.display_handle()) {
            (Ok(window), Ok(display)) => Some((window.as_raw(), display.as_raw())),
            _ => None,
        };
        let Some((window, display)) = handles else {
            let _ = sender.send(None);
            return;
        };
        glib::MainContext::default().spawn_local(async move {
            let runtime = tauri::async_runtime::handle();
            let _runtime_context = runtime.inner().enter();
            let parent = ashpd::WindowIdentifier::from_raw_handle(&window, Some(&display))
                .await
                .map(Arc::new);
            let _ = sender.send(parent);
        });
    });
    receiver
}

pub(super) async fn configure_wayland(
    app: &AppHandle,
    inner: &Arc<ShortcutInner>,
    preferred: String,
    explicit: bool,
) -> Result<ConfigureOutcome, LauncherError> {
    let parent = parent_receiver_on_main(app);
    let worker_sender = ensure_portal_worker(app, inner)?;
    let (response_sender, response_receiver) = oneshot::channel();
    if worker_sender
        .send(PortalCommand::Configure {
            preferred,
            explicit,
            parent,
            response: response_sender,
        })
        .await
        .is_err()
    {
        let status = unavailable_status("The global-shortcuts portal disconnected; retry setup");
        set_status(inner, status.clone());
        return Ok(ConfigureOutcome {
            status,
            denied: false,
        });
    }
    match response_receiver.await {
        Ok(outcome) => Ok(outcome),
        Err(_) => {
            let status =
                unavailable_status("The global-shortcuts portal disconnected; retry setup");
            set_status(inner, status.clone());
            Ok(ConfigureOutcome {
                status,
                denied: false,
            })
        }
    }
}

fn ensure_portal_worker(
    app: &AppHandle,
    inner: &Arc<ShortcutInner>,
) -> Result<mpsc::Sender<PortalCommand>, LauncherError> {
    if inner
        .shutdown_state
        .load(std::sync::atomic::Ordering::Acquire)
        != super::SHUTDOWN_IDLE
    {
        return Err(LauncherError::new(
            "shortcut_unavailable",
            "shortcut lifecycle is shutting down",
            true,
        ));
    }
    let mut state = lock_state(inner);
    if let Some(worker) = state.portal_worker.as_ref() {
        if !worker.sender.is_closed() {
            return Ok(worker.sender.clone());
        }
        state.portal_worker = None;
    }

    let (sender, receiver) = mpsc::channel(super::WORKER_CAPACITY);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let completion = Arc::new(Notify::new());
    let worker = PortalWorker {
        sender: sender.clone(),
        shutdown: Some(shutdown_sender),
        completion: Arc::clone(&completion),
    };
    state.portal_worker = Some(worker);
    let app = app.clone();
    let inner = Arc::clone(inner);
    tauri::async_runtime::spawn(async move {
        run_portal_worker(app, inner, receiver, shutdown_receiver, completion).await;
    });
    Ok(sender)
}

async fn next_event(
    session: &mut Option<PortalSessionState>,
    receiver: &mut mpsc::Receiver<PortalCommand>,
    shutdown: &mut oneshot::Receiver<()>,
) -> PortalEvent {
    if let Some(active) = session.as_mut() {
        tokio::select! {
            _ = shutdown => PortalEvent::Shutdown,
            command = receiver.recv() => PortalEvent::Command(command),
            activated = active.activated.next() => PortalEvent::Activated(activated),
            changed = active.changed.next() => PortalEvent::Changed(changed),
            _ = active.closed.next() => PortalEvent::Closed,
            owner = active.owner.try_next() => PortalEvent::Owner(owner),
        }
    } else {
        tokio::select! {
            _ = shutdown => PortalEvent::Shutdown,
            command = receiver.recv() => PortalEvent::Command(command),
        }
    }
}

async fn run_portal_worker(
    app: AppHandle,
    inner: Arc<ShortcutInner>,
    mut receiver: mpsc::Receiver<PortalCommand>,
    mut shutdown: oneshot::Receiver<()>,
    completion: Arc<Notify>,
) {
    let _completion = CompletionGuard(completion);
    let mut session: Option<PortalSessionState> = None;

    loop {
        match next_event(&mut session, &mut receiver, &mut shutdown).await {
            PortalEvent::Command(Some(PortalCommand::Configure {
                preferred,
                explicit,
                parent,
                response,
            })) => {
                let parent_identifier = tokio::select! {
                    _ = &mut shutdown => {
                        portal::close_session(&mut session).await;
                        return;
                    }
                    parent = parent => parent.ok().flatten(),
                };
                let result = tokio::select! {
                    _ = &mut shutdown => None,
                    result = portal::configure(
                        &inner,
                        &mut session,
                        parent_identifier,
                        &preferred,
                        explicit,
                    ) => Some(result),
                };
                let Some(result) = result else {
                    portal::close_session(&mut session).await;
                    return;
                };
                let _ = response.send(result);
            }
            PortalEvent::Activated(Some(activated)) => {
                if activated.shortcut_id() == super::SHORTCUT_ID
                    && session.as_ref().is_some_and(|active| {
                        activated.session_handle().as_str() == active.session_path
                    })
                {
                    dispatch_activation(&app, &activated);
                }
            }
            PortalEvent::Changed(Some(changed)) => {
                if let Some(active) = session
                    .as_mut()
                    .filter(|active| changed.session_handle().as_str() == active.session_path)
                {
                    let description = changed
                        .shortcuts()
                        .iter()
                        .find(|shortcut| shortcut.id() == super::SHORTCUT_ID)
                        .map(|shortcut| shortcut.trigger_description().to_string());
                    active.trigger_description = description;
                    let status = match active.trigger_description.as_deref() {
                        Some(description) => {
                            available_system_status(description, None, active.version)
                        }
                        None => unavailable_status(
                            "The system removed the launcher shortcut; retry setup",
                        ),
                    };
                    set_status(&inner, status);
                }
            }
            PortalEvent::Owner(Ok(Some(message))) => {
                let changed = message.body().deserialize::<(String, String, String)>();
                if let Ok((_, _, owner)) = changed
                    && session
                        .as_ref()
                        .is_some_and(|active| active.owner_name == owner)
                {
                    continue;
                }
                portal::close_session(&mut session).await;
                set_status(
                    &inner,
                    unavailable_status("The global-shortcuts service changed; retry setup"),
                );
            }
            PortalEvent::Shutdown | PortalEvent::Command(None) => {
                portal::close_session(&mut session).await;
                return;
            }
            PortalEvent::Changed(None)
            | PortalEvent::Activated(None)
            | PortalEvent::Closed
            | PortalEvent::Owner(Ok(None))
            | PortalEvent::Owner(Err(_)) => {
                portal::close_session(&mut session).await;
                set_status(
                    &inner,
                    unavailable_status("The global-shortcuts portal disconnected; retry setup"),
                );
            }
        }
    }
}

pub(super) fn wait_for_portal_worker(mut worker: PortalWorker) {
    if let Some(shutdown) = worker.shutdown.take() {
        let _ = shutdown.send(());
    }
    let completion = worker.completion;
    tauri::async_runtime::block_on(async move {
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(1), completion.notified()).await;
    });
}
