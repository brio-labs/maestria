use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::io::AsyncReadExt;

static NEXT_TEST_SOCKET: AtomicU64 = AtomicU64::new(0);

fn test_context(socket_path: PathBuf) -> Result<Arc<ApiContext>> {
    let realm_id = maestria_test_support::realm_id(10)?;
    let layout = InstanceLayout::for_root(std::env::temp_dir());
    let source_manifest = std::sync::Arc::new(parking_lot::RwLock::new(
        maestria_core::InstanceManifest::default_for_root(layout.root.clone(), realm_id.clone()),
    ));
    Ok(Arc::new(ApiContext {
        layout,
        token: "test-token".to_string(),
        socket_path,
        runtime: None,
        realm_id,
        source_manifest,
        interactive_searches: Arc::new(InteractiveSearchCoordinator::default()),
    }))
}

#[tokio::test]
async fn partial_request_disconnect_is_reported() -> Result<()> {
    let (mut writer, mut reader) = UnixStream::pair()?;
    writer.write_all(b"{\"token\":").await?;
    drop(writer);

    let result = read_request_line(&mut reader).await;

    assert!(
        matches!(result.as_ref(), Err(error) if error.to_string().contains("before end of message")),
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
        test_context(socket_path.clone())?,
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
#[tokio::test]
async fn search_api_protocol_mismatch_is_typed() -> Result<()> {
    let (mut client, mut server) = UnixStream::pair()?;
    let shutdown = CancellationToken::new();
    #[derive(serde::Serialize)]
    struct ProtocolProbe {
        protocol: &'static str,
        version: u64,
    }
    let request = serde_json::to_value(ProtocolProbe {
        protocol: super::super::protocol_search_api::SEARCH_API_PROTOCOL,
        version: u64::from(super::super::protocol_search_api::SEARCH_API_VERSION_2) + 1,
    })?;

    super::super::server_search_api::handle_request(
        &shutdown,
        &mut server,
        test_context(PathBuf::new())?,
        request,
    )
    .await?;

    let bytes = read_capped_ndjson_line(&mut client).await?;
    let reply: super::super::protocol_search_api::SearchApiReply =
        serde_json::from_slice(bytes.trim_ascii())?;
    assert_eq!(
        reply.protocol,
        super::super::protocol_search_api::SEARCH_API_PROTOCOL
    );
    assert_eq!(
        reply.version,
        super::super::protocol_search_api::SEARCH_API_VERSION_2
    );
    assert_eq!(
        reply.error_code,
        Some(ClientErrorCode::ProtocolVersionMismatch)
    );
    assert!(reply.response.is_none());
    Ok(())
}
#[test]
fn superseded_interactive_search_cancels_only_same_consumer_generation() -> Result<()> {
    let coordinator = Arc::new(InteractiveSearchCoordinator::default());
    let consumer = maestria_domain::RealmId::try_from("a".repeat(64))?;
    let other_consumer = maestria_domain::RealmId::try_from("b".repeat(64))?;
    let first_control = super::InteractiveSearchControl::default();
    let first = coordinator.begin(consumer.clone(), first_control.clone());
    let other_control = super::InteractiveSearchControl::default();
    let _other = coordinator.begin(other_consumer, other_control.clone());
    assert!(!first_control.signal.is_cancelled());
    assert!(!other_control.signal.is_cancelled());

    let second_control = super::InteractiveSearchControl::default();
    let second = coordinator.begin(consumer.clone(), second_control.clone());
    assert!(first_control.signal.is_cancelled());
    assert!(first_control.cancellation.is_cancelled());
    assert!(!other_control.signal.is_cancelled());

    drop(first);
    assert!(!second_control.signal.is_cancelled());
    let third_control = super::InteractiveSearchControl::default();
    let _third = coordinator.begin(consumer, third_control.clone());
    assert!(second_control.signal.is_cancelled());
    assert!(second_control.cancellation.is_cancelled());
    drop(second);
    assert!(!third_control.signal.is_cancelled());
    assert!(!other_control.signal.is_cancelled());
    Ok(())
}

#[test]
fn completed_interactive_search_does_not_mark_its_result_superseded() -> Result<()> {
    let coordinator = Arc::new(InteractiveSearchCoordinator::default());
    let consumer = maestria_domain::RealmId::try_from("a".repeat(64))?;
    let control = super::InteractiveSearchControl::default();
    let request = coordinator.begin(consumer.clone(), control.clone());
    drop(request);
    assert!(!control.signal.is_cancelled());
    assert!(!control.cancellation.is_cancelled());

    let next = super::InteractiveSearchControl::default();
    let _next_request = coordinator.begin(consumer, next.clone());
    assert!(!control.signal.is_cancelled());
    assert!(!next.signal.is_cancelled());
    Ok(())
}
