use crate::test_support::*;
use maestria_domain::{
    BlobId, ContentHash, DomainEvent, DomainEventEnvelope, DomainInput, KernelState,
    MaestriaEffect, OcrDisclosure, OcrEffect, OcrIntent, OcrProviderIdentity, OcrRetentionPolicy,
};
use maestria_ports::{
    EventFilter, EventLog, InMemoryEventLog, OcrIdentity, OcrPage, OcrProvider, OcrRequest,
    OcrResponse, PortError, ProviderDisclosure, RetentionPolicy,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn identity() -> OcrIdentity {
    OcrIdentity {
        provider: "fixture".into(),
        model: "ocr".into(),
        revision: "v1".into(),
        artifact_hash: "sha256:provider".into(),
        preprocessing_version: "prep-v1".into(),
    }
}
fn intent(
    blob: BlobId,
    bytes: &[u8],
    remote: bool,
) -> Result<OcrIntent, Box<dyn std::error::Error>> {
    Ok(OcrIntent::new(
        maestria_domain::ArtifactId::new(9),
        blob,
        ContentHash::new(maestria_domain::content_hash(bytes))?,
        [1, 2],
        OcrProviderIdentity::new("fixture", "ocr", "v1", "sha256:provider", "prep-v1")?,
        OcrDisclosure::new(remote, OcrRetentionPolicy::NoRetention),
    )?)
}

struct FixtureProvider {
    calls: Arc<AtomicUsize>,
    malformed: bool,
}
impl OcrProvider for FixtureProvider {
    fn recognize(&self, request: OcrRequest) -> Result<OcrResponse, PortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let pages = if self.malformed {
            vec![
                OcrPage {
                    page: 1,
                    text: "one".into(),
                },
                OcrPage {
                    page: 1,
                    text: "duplicate".into(),
                },
            ]
        } else {
            request
                .pages
                .into_iter()
                .map(|page| OcrPage {
                    page,
                    text: format!("page-{page}"),
                })
                .collect()
        };
        Ok(OcrResponse {
            pages,
            identity: identity(),
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        })
    }
    fn identity(&self) -> OcrIdentity {
        identity()
    }

    fn disclosure(&self) -> ProviderDisclosure {
        ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        }
    }
}

#[derive(Clone)]
struct BlockingOcrRequestedLog {
    inner: InMemoryEventLog,
    append_started: Arc<AtomicBool>,
    release_append: Arc<AtomicBool>,
}

impl EventLog for BlockingOcrRequestedLog {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        if matches!(event.event, DomainEvent::OcrRequested { .. }) {
            self.append_started.store(true, Ordering::SeqCst);
            while !self.release_append.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
        }
        self.inner.append(event)
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        self.inner.scan(filter)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn ocr_transport_waits_for_requested_event_persistence() -> TestResult {
    let bytes = b"pdf";
    let calls = Arc::new(AtomicUsize::new(0));
    let append_started = Arc::new(AtomicBool::new(false));
    let release_append = Arc::new(AtomicBool::new(false));
    let event_log = Arc::new(BlockingOcrRequestedLog {
        inner: InMemoryEventLog::new(),
        append_started: append_started.clone(),
        release_append: release_append.clone(),
    });
    let mut adapters = crate::test_helpers::test_adapters();
    let blob = adapters.blob_store.put(bytes.to_vec())?;
    adapters.event_log = event_log.clone();
    adapters.ocr_provider = Some(Arc::new(FixtureProvider {
        calls: calls.clone(),
        malformed: false,
    }));
    let request = intent(blob, bytes, false)?;
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig::default(),
        KernelState::new(),
        adapters,
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));
    let submission = tokio::spawn(async move {
        handle
            .submit(DomainInput::OcrRequested(maestria_domain::OcrRequested {
                intent: request,
            }))
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        while !append_started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "provider ran before OcrRequested was durable"
    );

    release_append.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(1), submission).await???;
    tokio::time::timeout(Duration::from_secs(1), async {
        while calls.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert!(
        event_log
            .scan(EventFilter { artifact_id: None })?
            .iter()
            .any(|envelope| matches!(envelope.event, DomainEvent::OcrRequested { .. }))
    );

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), run).await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ocr_transport_is_not_called_when_requested_event_persistence_fails() -> TestResult {
    let bytes = b"pdf";
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = crate::test_helpers::test_adapters();
    let blob = adapters.blob_store.put(bytes.to_vec())?;
    adapters.event_log = Arc::new(super::FailingEventLog);
    adapters.ocr_provider = Some(Arc::new(FixtureProvider {
        calls: calls.clone(),
        malformed: false,
    }));
    let request = intent(blob, bytes, false)?;
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig::default(),
        KernelState::new(),
        adapters,
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let run = tokio::spawn(runtime.run(input_rx, CancellationToken::new()));

    let _ = handle
        .submit(DomainInput::OcrRequested(maestria_domain::OcrRequested {
            intent: request,
        }))
        .await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "provider ran despite OcrRequested persistence failure"
    );
    run.abort();
    let _ = run.await;
    Ok(())
}

#[tokio::test]
async fn governed_policy_rejection_sends_zero_ocr_bytes() -> TestResult {
    let bytes = b"pdf";
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = crate::test_helpers::test_adapters();
    let blob = adapters.blob_store.put(bytes.to_vec())?;
    adapters.ocr_provider = Some(Arc::new(FixtureProvider {
        calls: calls.clone(),
        malformed: false,
    }));
    let request = intent(blob, bytes, true)?;
    let (input_tx, _input_rx) = mpsc::channel(8);
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::Ocr(OcrEffect::new(request)),
        EffectExecutionContext::test_default(
            Arc::new(adapters),
            Arc::new(crate::test_helpers::test_governance()),
            Arc::new(tokio::sync::RwLock::new(KernelState::new())),
            input_tx,
        ),
        None,
    )
    .await;
    assert!(!result);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn malformed_page_set_is_rejected_after_transport_and_not_completed() -> TestResult {
    let bytes = b"pdf";
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = crate::test_helpers::test_adapters();
    let blob = adapters.blob_store.put(bytes.to_vec())?;
    adapters.ocr_provider = Some(Arc::new(FixtureProvider {
        calls: calls.clone(),
        malformed: true,
    }));
    let request = intent(blob, bytes, false)?;
    let request_id = request.request_id().clone();
    let mut state = KernelState::new();
    state.pending_ocr.insert(request_id, request.clone());
    state
        .ocr_intents
        .insert(request.request_id().clone(), request.clone());
    let (input_tx, mut input_rx) = mpsc::channel(8);
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::Ocr(OcrEffect::new(request)),
        EffectExecutionContext::test_default(
            Arc::new(adapters),
            Arc::new(crate::test_helpers::test_governance()),
            Arc::new(tokio::sync::RwLock::new(state)),
            input_tx,
        ),
        None,
    )
    .await;
    assert!(!result);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        input_rx.recv().await,
        Some(DomainInput::OcrFailed(_))
    ));
    Ok(())
}

#[tokio::test]
async fn durable_ocr_completion_restart_resumes_parse_without_provider_retransmission() -> TestResult
{
    let bytes = b"recovered text";
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters = crate::test_helpers::test_adapters();
    let blob = adapters.blob_store.put(bytes.to_vec())?;
    adapters.ocr_provider = Some(Arc::new(FixtureProvider {
        calls: calls.clone(),
        malformed: false,
    }));
    let request = intent(blob, bytes, false)?;
    let completion = maestria_domain::OcrCompletion::new(
        &request,
        [
            maestria_domain::OcrPageText::new(1, "one")?,
            maestria_domain::OcrPageText::new(2, "two")?,
        ],
    )?;
    let mut state = KernelState::new();
    state.pending_parsers.insert(
        maestria_domain::ArtifactId::new(9),
        maestria_domain::ParserStarted {
            artifact_id: maestria_domain::ArtifactId::new(9),
            title: "recovered".into(),
            source_path: "recovered.rs".into(),
            content_hash: maestria_domain::content_hash(bytes),
            blob_id: blob,
        },
    );
    state
        .ocr_intents
        .insert(request.request_id().clone(), request.clone());
    state
        .ocr_results
        .insert(request.request_id().clone(), completion);
    let (input_tx, _input_rx) = mpsc::channel(8);
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(maestria_domain::ParseArtifactRequest {
            artifact_id: maestria_domain::ArtifactId::new(9),
            source_path: "recovered.rs".into(),
            source_bytes: Vec::new(),
            source_blob: Some(blob),
        }),
        EffectExecutionContext::test_default(
            Arc::new(adapters),
            Arc::new(crate::test_helpers::test_governance()),
            Arc::new(tokio::sync::RwLock::new(state)),
            input_tx,
        ),
        None,
    )
    .await;
    assert!(result);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}
