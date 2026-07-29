use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_SOCKET: AtomicU64 = AtomicU64::new(0);

fn test_context(socket_path: PathBuf) -> Arc<ApiContext> {
    Arc::new(ApiContext {
        layout: InstanceLayout::for_root(std::env::temp_dir()),
        token: "test-token".to_string(),
        socket_path,
        runtime: None,
    })
}

#[tokio::test]
async fn partial_request_disconnect_is_reported() -> Result<()> {
    let (mut writer, mut reader) = UnixStream::pair()?;
    writer.write_all(b"{\"token\":").await?;
    drop(writer);

    let result = read_request_line(&mut reader).await;

    assert!(
        matches!(result.as_ref(), Err(error) if error.to_string().contains("before end of request")),
        "expected truncated request error, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn shutdown_joins_blocked_connection_handler() -> Result<()> {
    let id = NEXT_TEST_SOCKET.fetch_add(1, Ordering::Relaxed);
    let socket_path = std::env::temp_dir().join(format!(
        "maestria-api-server-test-{}-{id}.sock",
        std::process::id()
    ));
    remove_stale_socket(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)?;
    let shutdown = CancellationToken::new();
    let connections = ConnectionTasks::default();
    let task = tokio::spawn(serve(
        listener,
        test_context(socket_path.clone()),
        shutdown.clone(),
        connections.clone(),
    ));
    let mut client = UnixStream::connect(&socket_path).await?;
    client.write_all(b"{\"token\":").await?;
    timeout(Duration::from_secs(1), async {
        loop {
            if connections.len().await == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;

    let remaining = connections.clone();
    let server = ApiServer {
        socket_path: socket_path.clone(),
        shutdown,
        task,
        connections,
    };
    timeout(Duration::from_secs(1), server.shutdown()).await??;

    let mut byte = [0u8; 1];
    let read = timeout(Duration::from_secs(1), client.read(&mut byte)).await?;
    match read {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        Ok(bytes) => {
            return Err(anyhow!(
                "blocked connection returned {bytes} bytes after shutdown"
            ));
        }
        Err(error) => return Err(error.into()),
    }
    assert_eq!(
        remaining.len().await,
        0,
        "connection task remained registered"
    );
    Ok(())
}
