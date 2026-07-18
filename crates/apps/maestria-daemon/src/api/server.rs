use std::{path::Path, sync::Arc};

use anyhow::{Result, anyhow};
use maestria_core::InstanceLayout;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    task::JoinHandle,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;

use super::protocol::ClientRequest;
use super::{
    ClientReplyOut, MAX_REQUEST_BYTES, dispatch, load_or_create_token, remove_stale_socket,
    set_private_permissions, socket_path, token_path,
};

/// Running local API server for one instance.
pub struct ApiServer {
    socket_path: std::path::PathBuf,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

impl ApiServer {
    /// Binds and starts the instance-scoped Unix socket server.
    pub async fn start(layout: InstanceLayout) -> Result<Self> {
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
        });
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(serve(listener, context.clone(), shutdown.clone()));
        Ok(Self {
            socket_path: context.socket_path.clone(),
            shutdown,
            task,
        })
    }

    /// Returns the socket path advertised to clients.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Stops the server and removes its socket.
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
                    let _ = handle_connection(stream, context).await;
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
    let mut buffer = [0u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let newline = buffer[..read].iter().position(|byte| *byte == b'\n');
        let length = match newline {
            Some(index) => index,
            None => read,
        };
        line.extend_from_slice(&buffer[..length]);
        if line.len() > MAX_REQUEST_BYTES {
            return Err(anyhow!("request exceeds size limit"));
        }
        if newline.is_some() {
            break;
        }
    }
    Ok(line)
}

async fn write_reply(
    stream: &mut UnixStream,
    response: Option<super::ClientResponse>,
    error: Option<String>,
) -> Result<()> {
    let mut line = serde_json::to_vec(&ClientReplyOut { response, error })?;
    line.push(b'\n');
    stream.write_all(&line).await?;
    Ok(())
}
