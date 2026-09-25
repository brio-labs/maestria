use std::{future::Future, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use parking_lot::{Mutex as ParkingMutex, RwLock};
use tokio::{
    io::AsyncWriteExt,
    net::{UnixListener, UnixStream},
    sync::{Mutex, Semaphore},
    task::{JoinHandle, JoinSet},
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::error;

use super::protocol::{
    ClientAuthentication, ClientErrorCode, ClientRequest, FederationCredential,
    read_capped_ndjson_line,
};
use super::{
    ClientReplyOut, MAX_REQUEST_BYTES, dispatch, load_or_create_token, remove_stale_socket,
    set_private_permissions, socket_path, token_path,
};

pub struct ApiServer {
    socket_path: std::path::PathBuf,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
    connections: ConnectionTasks,
}

impl ApiServer {
    /// Bind the Unix socket and start the request acceptor task.
    ///
    /// # Cancellation
    /// If the future is dropped after binding but before returning, the spawned acceptor task
    /// is aborted and the socket file may be left on disk.
    pub async fn start(
        layout: InstanceLayout,
        runtime: maestria_runtime::RuntimeHandle,
        source_manifest: Arc<RwLock<InstanceManifest>>,
    ) -> Result<Self> {
        let socket = socket_path(&layout);
        super::set_private_directory_permissions(&layout.system_dir)?;
        let token = load_or_create_token(&token_path(&layout))?;
        let manifest_contents =
            std::fs::read_to_string(&layout.manifest_path).with_context(|| {
                format!("read instance manifest {}", layout.manifest_path.display())
            })?;
        let realm_id = InstanceManifest::decode(&manifest_contents)
            .context("decode instance manifest for daemon realm identity")?
            .realm_id;
        remove_stale_socket(&socket)?;
        let listener = UnixListener::bind(&socket)
            .map_err(|error| anyhow!("bind daemon socket {}: {error}", socket.display()))?;
        set_private_permissions(&socket)?;
        let context = Arc::new(ApiContext {
            layout,
            token,
            socket_path: socket,
            runtime: Some(runtime),
            realm_id,
            source_manifest,
            interactive_searches: Arc::new(InteractiveSearchCoordinator::default()),
        });
        let shutdown = CancellationToken::new();
        let connections = ConnectionTasks::default();
        let task = tokio::spawn(serve(
            listener,
            context.clone(),
            shutdown.clone(),
            connections.clone(),
        ));
        Ok(Self {
            socket_path: context.socket_path.clone(),
            shutdown,
            task,
            connections,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Signal shutdown and await the acceptor and all connection handler tasks.
    ///
    /// # Cancellation
    /// Once called, the shutdown token is cancelled. If this future is dropped before the tasks
    /// join, the acceptor and connection handlers continue in the background until the acceptor
    /// observes the token; completion is not awaited.
    pub async fn shutdown(self) -> Result<()> {
        self.shutdown.cancel();
        let task_result = self
            .task
            .await
            .map_err(|error| anyhow!("daemon API task failed: {error}"));
        let connections_result = self.connections.join_all().await;

        task_result?;
        connections_result?;
        remove_stale_socket(&self.socket_path)
    }
}

#[derive(Clone, Default)]
struct ConnectionTasks {
    tasks: Arc<Mutex<JoinSet<()>>>,
}

impl ConnectionTasks {
    async fn spawn<F>(&self, handler: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tasks.lock().await.spawn(handler);
    }

    async fn join_all(&self) -> Result<()> {
        let mut tasks = self.tasks.lock().await;
        let mut first_error = None;
        while let Some(result) = tasks.join_next().await {
            if let (true, Err(error)) = (first_error.is_none(), result) {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(anyhow!("daemon API connection task failed: {error}"));
        }
        Ok(())
    }

    #[cfg(test)]
    async fn len(&self) -> usize {
        self.tasks.lock().await.len()
    }

    async fn reap_finished(&self) {
        let mut tasks = self.tasks.lock().await;
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result {
                error!(%error, "daemon API connection task failed");
            }
        }
    }
}

pub(crate) struct ApiContext {
    pub(crate) layout: InstanceLayout,
    pub(crate) token: String,
    pub(crate) socket_path: std::path::PathBuf,
    pub(crate) runtime: Option<maestria_runtime::RuntimeHandle>,
    pub(crate) realm_id: maestria_domain::RealmId,
    pub(crate) source_manifest: Arc<RwLock<InstanceManifest>>,
    pub(crate) interactive_searches: Arc<InteractiveSearchCoordinator>,
}
#[derive(Default)]
struct InteractiveSearchState {
    generation: u64,
    active: std::collections::BTreeMap<maestria_domain::RealmId, (u64, InteractiveSearchControl)>,
}

#[derive(Default)]
pub(crate) struct InteractiveSearchCoordinator {
    state: ParkingMutex<InteractiveSearchState>,
}

#[derive(Clone, Default)]
pub(crate) struct InteractiveSearchControl {
    pub(crate) signal: CancellationToken,
    pub(crate) cancellation: maestria_retrieval::SearchCancellation,
}

impl InteractiveSearchControl {
    pub(crate) fn cancel(&self) {
        self.cancellation.cancel();
        self.signal.cancel();
    }
}

pub(crate) struct InteractiveSearchRequest {
    coordinator: Arc<InteractiveSearchCoordinator>,
    consumer_realm: maestria_domain::RealmId,
    generation: u64,
}

impl InteractiveSearchCoordinator {
    pub(crate) fn begin(
        self: &Arc<Self>,
        consumer_realm: maestria_domain::RealmId,
        control: InteractiveSearchControl,
    ) -> InteractiveSearchRequest {
        let mut state = self.state.lock();
        if let Some((_, previous)) = state.active.remove(&consumer_realm) {
            previous.cancel();
        }
        state.generation = state.generation.wrapping_add(1).max(1);
        let generation = state.generation;
        state
            .active
            .insert(consumer_realm.clone(), (generation, control));
        InteractiveSearchRequest {
            coordinator: self.clone(),
            consumer_realm,
            generation,
        }
    }
}

impl Drop for InteractiveSearchRequest {
    fn drop(&mut self) {
        let mut state = self.coordinator.state.lock();
        if state
            .active
            .get(&self.consumer_realm)
            .is_some_and(|(generation, _)| *generation == self.generation)
        {
            state.active.remove(&self.consumer_realm);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RequestPrincipal {
    Instance,
    Federation {
        consumer_realm: maestria_domain::RealmId,
        credential: FederationCredential,
    },
}

async fn serve(
    listener: UnixListener,
    context: Arc<ApiContext>,
    shutdown: CancellationToken,
    connections: ConnectionTasks,
) {
    let permits = Arc::new(Semaphore::new(32));
    loop {
        connections.reap_finished().await;
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { break };
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue };
                let context = context.clone();
                let shutdown = shutdown.clone();
                connections.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_connection(stream, context, shutdown).await {
                        error!(%error, "api connection handler failed");
                    }
                }).await;
            }
        }
    }
}

async fn handle_connection(
    mut stream: UnixStream,
    context: Arc<ApiContext>,
    shutdown: CancellationToken,
) -> Result<()> {
    let line = match run_until_shutdown(
        &shutdown,
        timeout(Duration::from_secs(5), read_request_line(&mut stream)),
    )
    .await
    {
        Some(Ok(Ok(line))) => line,
        Some(Ok(Err(error))) => {
            return write_reply_until_shutdown(
                &shutdown,
                &mut stream,
                None,
                Some(error.to_string()),
                Some(ClientErrorCode::InvalidInput),
            )
            .await;
        }
        Some(Err(_)) => {
            return write_reply_until_shutdown(
                &shutdown,
                &mut stream,
                None,
                Some("request timed out".to_string()),
                Some(ClientErrorCode::InvalidInput),
            )
            .await;
        }
        None => return Ok(()),
    };
    let value = match serde_json::from_slice::<serde_json::Value>(line.trim_ascii()) {
        Ok(value) => value,
        Err(error) => {
            return invalid_request(&shutdown, &mut stream, error.to_string()).await;
        }
    };
    if value.get("protocol").is_some() {
        return super::server_search_api::handle_request(&shutdown, &mut stream, context, value)
            .await;
    }
    let request = match serde_json::from_value::<ClientRequest>(value) {
        Ok(request) => request,
        Err(error) => {
            return invalid_request(&shutdown, &mut stream, error.to_string()).await;
        }
    };
    let principal = match request.authentication {
        ClientAuthentication::InstanceToken { token } if token == context.token => {
            RequestPrincipal::Instance
        }
        ClientAuthentication::FederationGrant {
            consumer_realm,
            credential,
        } if matches!(
            &request.operation,
            super::ClientOperation::FederationSearch { .. }
                | super::ClientOperation::FederationEvidence { .. }
        ) =>
        {
            RequestPrincipal::Federation {
                consumer_realm,
                credential,
            }
        }
        _ => {
            return write_reply_until_shutdown(
                &shutdown,
                &mut stream,
                None,
                Some("unauthorized".to_string()),
                Some(ClientErrorCode::Unauthorized),
            )
            .await;
        }
    };
    let response =
        match run_until_shutdown(&shutdown, dispatch(&context, principal, request.operation)).await
        {
            Some(response) => response,
            None => return Ok(()),
        };
    match response {
        Ok(response) => {
            write_reply_until_shutdown(&shutdown, &mut stream, Some(response), None, None).await
        }
        Err(error) => {
            write_reply_until_shutdown(
                &shutdown,
                &mut stream,
                None,
                Some(error.to_string()),
                Some(classify_error(&error.to_string())),
            )
            .await
        }
    }
}
async fn invalid_request(
    shutdown: &CancellationToken,
    stream: &mut UnixStream,
    message: String,
) -> Result<()> {
    write_reply_until_shutdown(
        shutdown,
        stream,
        None,
        Some(format!("invalid request: {message}")),
        Some(ClientErrorCode::InvalidInput),
    )
    .await
}

async fn write_reply_until_shutdown(
    shutdown: &CancellationToken,
    stream: &mut UnixStream,
    response: Option<super::ClientResponse>,
    error: Option<String>,
    error_code: Option<ClientErrorCode>,
) -> Result<()> {
    match run_until_shutdown(shutdown, write_reply(stream, response, error, error_code)).await {
        Some(result) => result,
        None => Ok(()),
    }
}

pub(super) async fn run_until_shutdown<T, F>(shutdown: &CancellationToken, future: F) -> Option<T>
where
    F: Future<Output = T>,
{
    tokio::select! {
        _ = shutdown.cancelled() => None,
        output = future => Some(output),
    }
}

async fn read_request_line(stream: &mut UnixStream) -> Result<Vec<u8>> {
    read_capped_ndjson_line(stream).await
}

async fn write_reply(
    stream: &mut UnixStream,
    response: Option<super::ClientResponse>,
    error: Option<String>,
    error_code: Option<ClientErrorCode>,
) -> Result<()> {
    let code = error_code.or_else(|| error.as_deref().map(classify_error));
    let mut bytes = serde_json::to_vec(&ClientReplyOut {
        response,
        error,
        error_code: code,
    })
    .context("serialise daemon response")?;
    if bytes.len() + 1 > MAX_REQUEST_BYTES {
        bytes = serde_json::to_vec(&ClientReplyOut {
            response: None,
            error: Some("daemon response exceeds size limit".to_string()),
            error_code: Some(ClientErrorCode::Internal),
        })
        .context("serialise bounded daemon response")?;
    }
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .await
        .context("write daemon response")
}

pub(super) fn classify_error(message: &str) -> ClientErrorCode {
    let message = message.to_ascii_lowercase();
    if message.contains("executor is unavailable")
        || message.contains("service request timed out")
        || message.contains("requires a live daemon runtime")
    {
        ClientErrorCode::DaemonUnavailable
    } else if message.contains("unauthorized")
        || message.contains("access denied")
        || message.contains("grant expired")
    {
        ClientErrorCode::Unauthorized
    } else if message.contains("source_not_selected")
        || message.contains("source not selected")
        || message.contains("evidence not selected")
    {
        ClientErrorCode::SourceNotSelected
    } else if message.contains("source_unavailable")
        || message.contains("source unavailable")
        || message.contains("notebook source unavailable")
    {
        ClientErrorCode::SourceUnavailable
    } else if message.contains("revision") && message.contains("conflict") {
        ClientErrorCode::RevisionConflict
    } else if message.contains("not found")
        || message.contains("missing notebook")
        || message.contains("missing draft")
    {
        ClientErrorCode::NotFound
    } else if message.contains("limit")
        || message.contains("invalid")
        || message.contains("must not")
        || message.contains("must be between")
        || message.contains("must be absolute")
        || message.contains("empty")
    {
        ClientErrorCode::InvalidInput
    } else {
        ClientErrorCode::Internal
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod server_tests;
