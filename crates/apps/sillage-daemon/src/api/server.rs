use std::{future::Future, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow};
use parking_lot::{Mutex as ParkingMutex, RwLock};
use sillage_core::{InstanceLayout, InstanceManifest};
use tokio::{
    net::UnixListener,
    sync::{Mutex, Semaphore},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use tracing::error;

use super::protocol::{ClientErrorCode, FederationCredential};
use super::{
    load_or_create_token, remove_stale_socket, set_private_permissions, socket_path, token_path,
};
#[path = "server_connection.rs"]
mod connection;

pub struct ApiServer {
    socket_path: std::path::PathBuf,
    socket_identity: (u64, u64),
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
        runtime: sillage_runtime::RuntimeHandle,
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
        let socket_identity = socket_identity(&socket)?;
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
            socket_identity,
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
        remove_bound_socket(&self.socket_path, self.socket_identity)
    }
}

fn socket_identity(path: &Path) -> std::io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

fn remove_bound_socket(path: &Path, expected: (u64, u64)) -> Result<()> {
    match socket_identity(path) {
        Ok(actual) if actual == expected => {
            std::fs::remove_file(path).context("remove owned daemon socket")
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("inspect owned daemon socket before cleanup"),
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
    pub(crate) runtime: Option<sillage_runtime::RuntimeHandle>,
    pub(crate) realm_id: sillage_domain::RealmId,
    pub(crate) source_manifest: Arc<RwLock<InstanceManifest>>,
    pub(crate) interactive_searches: Arc<InteractiveSearchCoordinator>,
}
#[derive(Default)]
struct InteractiveSearchState {
    generation: u64,
    active: std::collections::BTreeMap<sillage_domain::RealmId, (u64, InteractiveSearchControl)>,
}

#[derive(Default)]
pub(crate) struct InteractiveSearchCoordinator {
    state: ParkingMutex<InteractiveSearchState>,
}

#[derive(Clone, Default)]
pub(crate) struct InteractiveSearchControl {
    pub(crate) signal: CancellationToken,
    pub(crate) cancellation: sillage_retrieval::SearchCancellation,
}

impl InteractiveSearchControl {
    pub(crate) fn cancel(&self) {
        self.cancellation.cancel();
        self.signal.cancel();
    }
}

pub(crate) struct InteractiveSearchRequest {
    coordinator: Arc<InteractiveSearchCoordinator>,
    consumer_realm: sillage_domain::RealmId,
    generation: u64,
}

impl InteractiveSearchCoordinator {
    pub(crate) fn begin(
        self: &Arc<Self>,
        consumer_realm: sillage_domain::RealmId,
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
        consumer_realm: sillage_domain::RealmId,
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
                    if let Err(error) =
                        connection::handle_connection(stream, context, shutdown).await
                    {
                        error!(%error, "api connection handler failed");
                    }
                }).await;
            }
        }
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
