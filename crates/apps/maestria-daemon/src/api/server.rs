use std::{future::Future, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::DomainInput;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, Semaphore, mpsc},
    task::{JoinHandle, JoinSet},
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
    pub(crate) input_tx: mpsc::Sender<DomainInput>,
    pub(crate) adapters: Arc<maestria_runtime::Adapters>,
    pub(crate) governance: Arc<maestria_runtime::Governance>,
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
            return match run_until_shutdown(
                &shutdown,
                write_reply(&mut stream, None, Some(error.to_string())),
            )
            .await
            {
                Some(result) => result,
                None => Ok(()),
            };
        }
        Some(Err(_)) => {
            return match run_until_shutdown(
                &shutdown,
                write_reply(&mut stream, None, Some("request timed out".to_string())),
            )
            .await
            {
                Some(result) => result,
                None => Ok(()),
            };
        }
        None => return Ok(()),
    };
    let request = match serde_json::from_slice::<ClientRequest>(line.trim_ascii()) {
        Ok(request) => request,
        Err(error) => {
            return match run_until_shutdown(
                &shutdown,
                write_reply(&mut stream, None, Some(format!("invalid request: {error}"))),
            )
            .await
            {
                Some(result) => result,
                None => Ok(()),
            };
        }
    };
    if request.token != context.token {
        return match run_until_shutdown(
            &shutdown,
            write_reply(&mut stream, None, Some("unauthorized".to_string())),
        )
        .await
        {
            Some(result) => result,
            None => Ok(()),
        };
    }
    let response = match run_until_shutdown(&shutdown, dispatch(&context, request.operation)).await
    {
        Some(response) => response,
        None => return Ok(()),
    };
    match response {
        Ok(response) => {
            match run_until_shutdown(&shutdown, write_reply(&mut stream, Some(response), None))
                .await
            {
                Some(result) => result,
                None => Ok(()),
            }
        }
        Err(error) => match run_until_shutdown(
            &shutdown,
            write_reply(&mut stream, None, Some(error.to_string())),
        )
        .await
        {
            Some(result) => result,
            None => Ok(()),
        },
    }
}

async fn run_until_shutdown<T, F>(shutdown: &CancellationToken, future: F) -> Option<T>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        _ = shutdown.cancelled() => None,
        output = future => Some(output),
    }
}

async fn read_request_line(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        match stream.read_exact(&mut buf).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                let message = if line.is_empty() {
                    "connection closed before any data"
                } else {
                    "connection closed before end of request"
                };
                return Err(anyhow!(message));
            }
            Err(error) => return Err(error).context("read request line"),
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

#[cfg(test)]
#[path = "server_tests.rs"]
mod server_tests;
