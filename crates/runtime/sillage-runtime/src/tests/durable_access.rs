use super::*;
use sillage_domain::{
    DomainEvent, FederatedAccessRecord, FederatedEvidenceBounds, FederatedReadAccess,
    GrantTokenDigest, IssueRealmReadGrantInput, QueryId, RealmId, RealmReadGrant,
    RealmReadGrantExpiry, RecordFederatedAccessInput, SearchTraceId, Sensitivity,
    StartFullTextIndex,
};
use sillage_ports::{EventFilter, EventLog};
use sillage_storage_sqlite::SqliteStore;

struct ReleaseIndexOnDrop(Arc<AtomicBool>);

impl Drop for ReleaseIndexOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn pending_full_text_state(artifact_id: ArtifactId) -> KernelState {
    let chunk_id = ChunkId::new(10);
    let mut state = KernelState::new();
    let mut artifact = artifact_fixture(artifact_id);
    artifact.chunk_ids.insert(chunk_id);
    Arc::make_mut(&mut state.artifacts).insert(artifact_id, artifact);
    Arc::make_mut(&mut state.chunks).insert(
        chunk_id,
        chunk_fixture(chunk_id, artifact_id, 0, "indexed fixture passage"),
    );
    Arc::make_mut(&mut state.pending_full_text).insert(chunk_id);
    state
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_search_audit_does_not_wait_for_saturated_indexing()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database_path = directory.path().join("audit.db");
    let store = Arc::new(SqliteStore::open(&database_path)?);
    let release = Arc::new(AtomicBool::new(false));
    let _release_on_drop = ReleaseIndexOnDrop(Arc::clone(&release));
    let entered = Arc::new(AtomicUsize::new(0));
    let artifact_id = ArtifactId::new(1);
    let adapters = Adapters {
        event_log: store.clone(),
        realm_read_grant_repo: store.clone(),
        search_index: Arc::new(BlockingFullTextIndex {
            inner: InMemoryFullTextIndex::new(),
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        }),
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, inputs) = SillageRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 1,
            max_retries: 0,
            ..RuntimeConfig::default()
        },
        pending_full_text_state(artifact_id),
        adapters,
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let shutdown = CancellationToken::new();
    let runner = tokio::spawn(runtime.run(inputs, shutdown.clone()));
    let provider_realm = RealmId::try_from("a".repeat(64))?;
    let consumer_realm = RealmId::try_from("b".repeat(64))?;
    let digest = GrantTokenDigest::derive(b"durable-audit-regression-fixture");
    let grant = RealmReadGrant::new(
        digest.clone(),
        provider_realm.clone(),
        consumer_realm.clone(),
        FederatedReadAccess::SearchAndOpenEvidence,
        Sensitivity::Internal,
        FederatedEvidenceBounds::try_new(1, 512)?,
        RealmReadGrantExpiry::new(4_000_000_000)?,
    )
    .with_allowed_roots(vec![directory.path().join("approved")]);
    handle
        .submit_durable(DomainInput::IssueRealmReadGrant(IssueRealmReadGrantInput {
            grant,
        }))
        .await?;
    handle
        .submit(DomainInput::StartFullTextIndex(StartFullTextIndex {
            artifact_id,
        }))
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while entered.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;

    let record = FederatedAccessRecord::Search {
        query_id: QueryId::new(41),
        trace_id: SearchTraceId::new(73),
    };
    let audit = tokio::time::timeout(
        Duration::from_millis(100),
        handle.submit_durable(DomainInput::RecordFederatedAccess(
            RecordFederatedAccessInput {
                token_digest: digest.clone(),
                provider_realm: provider_realm.clone(),
                consumer_realm: consumer_realm.clone(),
                record,
            },
        )),
    )
    .await;
    let persisted_before_release = SqliteStore::open_read_only(&database_path)?
        .scan(EventFilter { artifact_id: None })?
        .into_iter()
        .any(|envelope| {
            matches!(envelope.event, DomainEvent::FederatedReadAccessRecorded {
                token_digest, provider_realm: provider, consumer_realm: consumer, record: observed,
            } if token_digest == digest && provider == provider_realm
                && consumer == consumer_realm && observed == record)
        });
    release.store(true, Ordering::SeqCst);
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), runner).await???;

    audit.map_err(|_| "durable search audit waited for the indexing permit past 100 ms")??;
    assert!(
        persisted_before_release,
        "the correlated audit reply must follow durable persistence without releasing indexing"
    );
    Ok(())
}
