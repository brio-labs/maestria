use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::DomainInput;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Semaphore, mpsc},
    task::JoinHandle,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::error;

use super::protocol::ClientRequest;
use super::{
    ClientReplyOut, MAX_REQUEST_BYTES, dispatch, load_or_create_token, remove_stale_socket,
    set_private_permissions, socket_path, token_path,
};

pub struct ApiServer {
    socket_path: std::path::PathBuf,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

impl ApiServer {
    /// Bind the Unix socket and start the request acceptor task.
    ///
    /// # Cancellation
    /// If the future is dropped after binding but before returning, the spawned acceptor task
    /// is aborted and the socket file may be left on disk.
    pub async fn start(
        layout: InstanceLayout,
        input_tx: mpsc::Sender<DomainInput>,
        adapters: Arc<maestria_runtime::Adapters>,
        governance: Arc<maestria_runtime::Governance>,
    ) -> Result<Self> {
        let socket = socket_path(&layout);
        super::set_private_directory_permissions(&layout.system_dir)?;
        let token = load_or_create_token(&token_path(&layout))?;
        remove_stale_socket(&socket)?;
        let listener = UnixListener::bind(&socket)
            .map_err(|error| anyhow!("bind daemon socket {}: {error}", socket.display()))?;
        set_private_permissions(&socket)?;
        let context = Arc::new(ApiContext {
            layout,
            token,
            socket_path: socket,
            input_tx,
            adapters,
            governance,
        });
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(serve(listener, context.clone(), shutdown.clone()));
        Ok(Self {
            socket_path: context.socket_path.clone(),
            shutdown,
            task,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Signal shutdown and await the acceptor task.
    ///
    /// # Cancellation
    /// Once called, the shutdown token is cancelled. If this future is dropped before the task
    /// joins, the acceptor continues until it observes the token but completion is not awaited.
    pub async fn shutdown(self) -> Result<()> {
        self.shutdown.cancel();
        self.task
            .await
            .map_err(|error| anyhow!("daemon API task failed: {error}"))?;
        remove_stale_socket(&self.socket_path)
    }
}

pub(crate) struct ApiContext {
    pub(crate) layout: InstanceLayout,
    pub(crate) token: String,
    pub(crate) socket_path: std::path::PathBuf,
    pub(crate) input_tx: mpsc::Sender<DomainInput>,
    pub(crate) adapters: Arc<maestria_runtime::Adapters>,
    pub(crate) governance: Arc<maestria_runtime::Governance>,
}

async fn serve(listener: UnixListener, context: Arc<ApiContext>, shutdown: CancellationToken) {
    let permits = Arc::new(Semaphore::new(32));
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { break };
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue };
                let context = context.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_connection(stream, context).await {
                        error!(%error, "api connection handler failed");
                    }
                });
            }
        }
    }
}

async fn handle_connection(mut stream: UnixStream, context: Arc<ApiContext>) -> Result<()> {
    let line = match timeout(Duration::from_secs(5), read_request_line(&mut stream)).await {
        Ok(Ok(line)) => line,
        Ok(Err(error)) => {
            return write_reply(&mut stream, None, Some(error.to_string())).await;
        }
        Err(_) => {
            return write_reply(&mut stream, None, Some("request timed out".to_string())).await;
        }
    };
    let request = match serde_json::from_slice::<ClientRequest>(line.trim_ascii()) {
        Ok(request) => request,
        Err(error) => {
            return write_reply(&mut stream, None, Some(format!("invalid request: {error}"))).await;
        }
    };
    if request.token != context.token {
        return write_reply(&mut stream, None, Some("unauthorized".to_string())).await;
    }
    match dispatch(&context, request.operation).await {
        Ok(response) => write_reply(&mut stream, Some(response), None).await,
        Err(error) => write_reply(&mut stream, None, Some(error.to_string())).await,
    }
}

async fn read_request_line(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        if stream.read_exact(&mut buf).await.is_err() {
            if line.is_empty() {
                return Err(anyhow!("connection closed before any data"));
            }
            break;
        }
        if buf[0] == b'\n' {
            break;
        }
        if line.len() >= MAX_REQUEST_BYTES {
            return Err(anyhow!("request line exceeds maximum length"));
        }
        line.push(buf[0]);
    }
    Ok(line)
}

async fn write_reply(
    stream: &mut UnixStream,
    response: Option<super::ClientResponse>,
    error: Option<String>,
) -> Result<()> {
    let reply = ClientReplyOut { response, error };
    let mut bytes = serde_json::to_vec(&reply).context("serialise daemon response")?;
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .await
        .context("write daemon response")
}
