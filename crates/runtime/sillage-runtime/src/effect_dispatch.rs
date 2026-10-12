use crate::SillageRuntime;
use crate::effect_execution_dispatch::PreparedEffect;
use sillage_domain::SillageEffect;
use std::fmt;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) enum EffectWork {
    Pending(SillageEffect),
    Prepared(PreparedEffect),
    FullTextBatch(Vec<sillage_domain::IndexChunkRequest>),
}

pub(crate) type EffectBatch = Vec<EffectWork>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectAdmissionError {
    Cancelled,
    ChannelClosed,
}

impl fmt::Display for EffectAdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("effect batch admission cancelled"),
            Self::ChannelClosed => f.write_str("effect channel closed during batch admission"),
        }
    }
}

impl std::error::Error for EffectAdmissionError {}

impl SillageRuntime {
    /// Reserve one channel item for the complete effect batch before the
    /// domain transition is committed. A successful reservation is the
    /// all-or-none admission cutoff.
    pub(crate) async fn reserve_effect_batch<'a>(
        &self,
        effect_tx: &'a mpsc::Sender<EffectBatch>,
        shutdown_token: &CancellationToken,
    ) -> Result<mpsc::Permit<'a, EffectBatch>, EffectAdmissionError> {
        tokio::select! {
            biased;
            // Cancellation closes admission only outside the shutdown
            // drain: the drain itself must keep admitting completion
            // deliveries so their batches reach the live executor.
            () = shutdown_token.cancelled(), if !self.admission_open.load(Ordering::Relaxed) => {
                Err(EffectAdmissionError::Cancelled)
            }
            result = effect_tx.reserve() => {
                result.map_err(|_| EffectAdmissionError::ChannelClosed)
            }
        }
    }

    /// Send an already-reserved batch. The vector is one bounded channel
    /// item, so no prefix can be admitted independently of the transition.
    pub(crate) fn send_reserved_effects(
        &self,
        permit: mpsc::Permit<'_, EffectBatch>,
        effects: EffectBatch,
    ) -> Result<(), EffectAdmissionError> {
        self.pending_effect_batches.fetch_add(1, Ordering::AcqRel);
        self.executor_quiescence_notify.notify_one();
        permit.send(effects);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sillage_domain::UpdateGraphRequest;

    #[tokio::test]
    async fn admission_carries_a_large_transition_batch_as_one_item()
    -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) = mpsc::channel::<EffectBatch>(1);
        let batch = (0..8)
            .map(|index| {
                EffectWork::Pending(SillageEffect::UpdateGraph(UpdateGraphRequest {
                    relation_id: sillage_domain::RelationId::new(index as u64),
                }))
            })
            .collect::<Vec<_>>();
        let expected_len = batch.len();
        let permit = sender.reserve().await?;
        permit.send(batch);

        let admitted = receiver.recv().await.ok_or("batch should be admitted")?;
        assert_eq!(admitted.len(), expected_len);
        Ok(())
    }
}
#[cfg(test)]
mod settlement_tests {
    use super::*;
    use crate::SillageRuntime;
    use crate::config::{Adapters, RuntimeConfig};
    use crate::test_helpers::{test_adapters, test_governance};
    use sillage_domain::{
        DomainEvent, DomainEventEnvelope, EventId, FetchWebRequest, LogicalTick, SillageEffect,
    };
    use sillage_ports::{EventFilter, EventLog, InMemoryEventLog, InMemoryWebFetcher, PortError};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    struct FirstAppendGateEventLog {
        inner: InMemoryEventLog,
        entered: std::sync::mpsc::SyncSender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl EventLog for FirstAppendGateEventLog {
        fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
            self.entered.send(()).map_err(|_| {
                PortError::internal("effect settlement test", "entry observer closed")
            })?;
            let release = self.release.lock().map_err(|_| {
                PortError::internal("effect settlement test", "release mutex poisoned")
            })?;
            let _ = release.recv();
            self.inner.append(event)
        }

        fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
            self.inner.scan(filter)
        }

        fn contains_id(&self, event_id: EventId) -> Result<bool, PortError> {
            self.inner.contains_id(event_id)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merged_batches_stay_counted_through_inline_persistence_before_child_admission()
    -> Result<(), Box<dyn std::error::Error>> {
        let event_log = InMemoryEventLog::new();
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let gated_event_log = Arc::new(FirstAppendGateEventLog {
            inner: event_log.clone(),
            entered: entered_tx,
            release: Mutex::new(release_rx),
        });
        let fetcher = Arc::new(InMemoryWebFetcher::new());
        let url = "https://example.com/settlement";
        let html = "<html><body>accepted effect</body></html>";
        fetcher.seed(url, html)?;
        let adapters = Adapters {
            event_log: gated_event_log,
            web_fetcher: fetcher,
            ..test_adapters()
        };
        let config = RuntimeConfig {
            scope: sillage_governance::Scope::new(vec![], vec![], vec![], vec![], true),
            drain_effects_on_shutdown: true,
            ..RuntimeConfig::default()
        };
        let (mut runtime, _input_rx) = SillageRuntime::new(
            config,
            sillage_domain::KernelState::new(),
            adapters,
            test_governance(),
        );
        let mut feedback_rx = runtime
            .feedback_rx
            .take()
            .ok_or("runtime feedback receiver missing")?;
        let (effect_tx, effect_rx) = mpsc::channel(2);
        let admission_shutdown = CancellationToken::new();
        let persist_permit = runtime
            .reserve_effect_batch(&effect_tx, &admission_shutdown)
            .await?;
        runtime.send_reserved_effects(
            persist_permit,
            vec![EffectWork::Pending(SillageEffect::PersistEvent {
                envelope: Box::new(DomainEventEnvelope {
                    id: EventId::new(1),
                    event: DomainEvent::TickObserved {
                        at: LogicalTick::new(1),
                    },
                }),
            })],
        )?;
        let child_permit = runtime
            .reserve_effect_batch(&effect_tx, &admission_shutdown)
            .await?;
        runtime.send_reserved_effects(
            child_permit,
            vec![EffectWork::Pending(SillageEffect::FetchWeb(
                FetchWebRequest {
                    url: url.to_string(),
                    max_bytes: html.len() + 1,
                    max_requests: 1,
                    max_latency_ms: 15_000,
                    allowed_domains: Vec::new(),
                    allowed_content_types: Vec::new(),
                },
            ))],
        )?;
        drop(effect_tx);

        let effect_shutdown = CancellationToken::new();
        let runtime_shutdown = CancellationToken::new();
        let executor = runtime.spawn_effect_executor(effect_rx, effect_shutdown, runtime_shutdown);
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::task::spawn_blocking(move || entered_rx.recv()),
        )
        .await???;
        assert_eq!(
            runtime
                .pending_effect_batches
                .load(std::sync::atomic::Ordering::Acquire),
            2,
            "the merged batch must remain counted while inline persistence is blocked"
        );
        assert_eq!(
            runtime
                .in_flight_effects
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        release_tx.send(())?;

        let registration = tokio::time::timeout(Duration::from_secs(2), feedback_rx.recv())
            .await?
            .ok_or("fetch child did not produce feedback")?;
        assert!(matches!(
            registration,
            sillage_domain::DomainInput::RegisterArtifact(_)
        ));
        let executor_result = tokio::time::timeout(Duration::from_secs(2), executor).await??;
        assert!(
            executor_result.is_ok(),
            "executor failed: {executor_result:?}"
        );
        assert_eq!(
            runtime
                .pending_effect_batches
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        assert_eq!(
            runtime
                .in_flight_effects
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        let events = event_log.scan(EventFilter { artifact_id: None })?;
        assert!(
            events
                .iter()
                .any(|event| matches!(&event.event, DomainEvent::TickObserved { .. }))
        );
        Ok(())
    }
}

#[cfg(test)]
mod final_feedback_settlement_tests {
    use crate::SillageRuntime;
    use crate::config::{Adapters, RuntimeConfig};
    use crate::test_helpers::{test_adapters, test_governance};
    use sillage_domain::{DomainEvent, DomainInput, FetchWebRequest, FetchWebRequested};
    use sillage_ports::{
        EventFilter, EventLog, InMemoryEventLog, InMemoryWebFetcher, PortError, WebFetcher,
        WebSnapshotData,
    };
    use std::error::Error;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    struct ShutdownTokenGuard(CancellationToken);

    impl Drop for ShutdownTokenGuard {
        fn drop(&mut self) {
            self.0.cancel();
        }
    }

    struct GatedWebFetcher {
        inner: InMemoryWebFetcher,
        entered: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl WebFetcher for GatedWebFetcher {
        fn fetch(&self, url: &str, max_bytes: usize) -> Result<WebSnapshotData, PortError> {
            self.entered.send(()).map_err(|_| {
                PortError::internal("final feedback settlement test", "entry observer closed")
            })?;
            let release = self.release.lock().map_err(|_| {
                PortError::internal("final feedback settlement test", "release mutex poisoned")
            })?;
            let _ = release.recv();
            self.inner.fetch(url, max_bytes)
        }
    }

    fn install_settlement_hook(
        runtime: &mut SillageRuntime,
    ) -> (mpsc::Receiver<()>, mpsc::SyncSender<()>) {
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (resume_tx, resume_rx) = mpsc::sync_channel(1);
        let resume_rx = Arc::new(Mutex::new(resume_rx));
        let first_check = Arc::new(AtomicBool::new(true));
        runtime.test_settlement_check_hook = Some(Arc::new(move || {
            if first_check.swap(false, Ordering::AcqRel) {
                let _ = entered_tx.send(());
                if let Ok(resume_rx) = resume_rx.lock() {
                    let _ = resume_rx.recv();
                }
            }
        }));
        (entered_rx, resume_tx)
    }

    async fn wait_for_executor_idle(
        pending_effect_batches: &AtomicUsize,
        in_flight_effects: &AtomicUsize,
        quiescence_notify: &Notify,
    ) {
        loop {
            let quiescence_changed = quiescence_notify.notified();
            tokio::pin!(quiescence_changed);
            quiescence_changed.as_mut().enable();
            if pending_effect_batches.load(Ordering::Acquire) == 0
                && in_flight_effects.load(Ordering::Acquire) == 0
            {
                return;
            }
            quiescence_changed.await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn final_feedback_is_rechecked_after_child_reap() -> Result<(), Box<dyn Error>> {
        let event_log: Arc<dyn EventLog> = Arc::new(InMemoryEventLog::new());
        let fetcher = InMemoryWebFetcher::new();
        let url = "https://example.com/final-settlement-feedback";
        let html = "<html><body>final feedback</body></html>";
        fetcher.seed(url, html)?;
        let (fetch_entered_tx, fetch_entered_rx) = mpsc::sync_channel(1);
        let (fetch_release_tx, fetch_release_rx) = mpsc::sync_channel(1);
        let adapters = Adapters {
            event_log: event_log.clone(),
            web_fetcher: Arc::new(GatedWebFetcher {
                inner: fetcher,
                entered: fetch_entered_tx,
                release: Mutex::new(fetch_release_rx),
            }),
            ..test_adapters()
        };
        let config = RuntimeConfig {
            scope: sillage_governance::Scope::new(vec![], vec![], vec![], vec![], true),
            ..RuntimeConfig::default()
        };
        let (mut runtime, input_rx) = SillageRuntime::new(
            config,
            sillage_domain::KernelState::new(),
            adapters,
            test_governance(),
        );
        let (settlement_entered_rx, settlement_resume_tx) = install_settlement_hook(&mut runtime);
        let pending_effect_batches = Arc::clone(&runtime.pending_effect_batches);
        let in_flight_effects = Arc::clone(&runtime.in_flight_effects);
        let quiescence_notify = Arc::clone(&runtime.executor_quiescence_notify);
        let handle = runtime.handle();
        let shutdown = CancellationToken::new();
        let _shutdown_guard = ShutdownTokenGuard(shutdown.clone());
        let run = tokio::spawn(
            runtime
                .with_graceful_shutdown()
                .run(input_rx, shutdown.clone()),
        );

        handle
            .submit(DomainInput::FetchWebRequested(FetchWebRequested {
                request: FetchWebRequest {
                    url: url.to_string(),
                    max_bytes: html.len() + 1,
                    max_requests: 1,
                    max_latency_ms: 15_000,
                    allowed_domains: Vec::new(),
                    allowed_content_types: Vec::new(),
                },
            }))
            .await?;
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::task::spawn_blocking(move || fetch_entered_rx.recv()),
        )
        .await???;
        shutdown.cancel();
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::task::spawn_blocking(move || settlement_entered_rx.recv()),
        )
        .await???;

        fetch_release_tx.send(())?;
        tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_executor_idle(
                &pending_effect_batches,
                &in_flight_effects,
                &quiescence_notify,
            ),
        )
        .await?;
        settlement_resume_tx.send(())?;

        let run_result = tokio::time::timeout(Duration::from_secs(7), run).await??;
        assert!(
            run_result.is_ok(),
            "runtime failed during graceful settlement: {run_result:?}"
        );
        let events = event_log.scan(EventFilter { artifact_id: None })?;
        assert!(
            events
                .iter()
                .any(|event| matches!(&event.event, DomainEvent::EvidenceRecorded { .. }))
        );
        Ok(())
    }
}
