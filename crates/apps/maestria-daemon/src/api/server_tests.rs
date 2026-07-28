use super::*;
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate};
use maestria_ports::{
    InMemoryApprovalRepository, InMemoryArtifactRepository, InMemoryBlobStore,
    InMemoryCardRepository, InMemoryChunkRepository, InMemoryEffectJournal, InMemoryEventLog,
    InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex, InMemoryHarnessAdapter,
    InMemoryIdAllocator, InMemoryParser, InMemoryVectorIndex, InMemoryWebFetcher,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_SOCKET: AtomicU64 = AtomicU64::new(0);

fn test_context(socket_path: PathBuf) -> Arc<ApiContext> {
    let (input_tx, _) = mpsc::channel(1);
    Arc::new(ApiContext {
        layout: InstanceLayout::for_root(std::env::temp_dir()),
        token: "test-token".to_string(),
        socket_path,
        input_tx,
        adapters: Arc::new(maestria_runtime::Adapters {
            event_log: Arc::new(InMemoryEventLog::new()),
            blob_store: Arc::new(InMemoryBlobStore::new()),
            search_index: Arc::new(InMemoryFullTextIndex::new()),
            harness: Arc::new(InMemoryHarnessAdapter::new()),
            parser: Arc::new(InMemoryParser::new()),
            artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
            chunk_repo: Arc::new(InMemoryChunkRepository::new()),
            card_repo: Arc::new(InMemoryCardRepository::new()),
            evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
            embedding_provider: None,
            search_executor: None,
            vector_index: Arc::new(InMemoryVectorIndex::new()),
            graph_index: Arc::new(InMemoryGraphIndex::new()),
            web_fetcher: Arc::new(InMemoryWebFetcher::new()),
            id_allocator: Arc::new(InMemoryIdAllocator::new()),
            effect_journal: Arc::new(InMemoryEffectJournal::default()),
            approval_repo: Arc::new(InMemoryApprovalRepository::new()),
        }),
        governance: Arc::new(maestria_runtime::Governance {
            classifier: Arc::new(DefaultRiskClassifier),
            approval_gate: Arc::new(DefaultApprovalGate),
            validation_gate: Arc::new(DefaultValidationGate::new(true)),
            memory_promotion_gate: Arc::new(maestria_governance::DefaultMemoryPromotionGate),
        }),
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
