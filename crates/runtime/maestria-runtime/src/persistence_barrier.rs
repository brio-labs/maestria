//! Durable event-log persistence barriers.
//!
//! Runtime orchestration must observe that an event emitted by the domain has
//! actually been durably recorded before proceeding (e.g. resuming parsing,
//! resolving approvals, or continuing a command after a proposal commit).
//! Every barrier shares one shape — scan the event log, test a predicate,
//! poll on a small fixed interval, and stop on shutdown or timeout — so the
//! polling machinery lives here once instead of being duplicated at each
//! call site (R17: composition beats accumulation).
//!
//! A barrier returns `true` when the predicate observed the event within the
//! timeout, and `false` on timeout, scan error, or shutdown. Callers decide
//! what `false` means for their orchestration step.

use std::time::Duration;

use maestria_domain::{
    ApprovalId, ArtifactId, BlobId, DomainEvent, DomainEventEnvelope, EventId, ValidationReportId,
};
use maestria_ports::{ApprovalRepository, EventFilter, EventLog};
use tokio_util::sync::CancellationToken;

/// Wait until `matches` observes the target condition in the event log, or
/// `timeout` elapses, or `shutdown_token` is cancelled.
///
/// The predicate is invoked for every envelope of each scan; the scan is
/// repeated on a fixed 5ms poll interval. Scan errors are logged and treated
/// as a failed barrier (the caller must not continue on an unobservable
/// durability check).
pub(crate) async fn wait_for_event(
    event_log: &dyn EventLog,
    timeout: Duration,
    shutdown_token: &CancellationToken,
    context: &str,
    matches: impl Fn(&DomainEventEnvelope) -> bool,
) -> bool {
    let check = async {
        loop {
            if shutdown_token.is_cancelled() {
                return false;
            }
            match event_log.scan(EventFilter { artifact_id: None }) {
                Ok(events) => {
                    if events.iter().any(&matches) {
                        return true;
                    }
                }
                Err(error) => {
                    tracing::error!(
                        %error,
                        %context,
                        "failed to scan event log during persistence barrier"
                    );
                    return false;
                }
            }
            tokio::select! {
                () = shutdown_token.cancelled() => return false,
                () = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }
    };
    matches!(tokio::time::timeout(timeout, check).await, Ok(true))
}

/// Predicate observing that a `ValidationReportCreated` event for
/// `report_id` is durably recorded.
pub(crate) fn validation_report_created(
    report_id: ValidationReportId,
) -> impl Fn(&DomainEventEnvelope) -> bool {
    move |env| {
        matches!(
            &env.event,
            DomainEvent::ValidationReportCreated {
                report_id: id,
                ..
            } if *id == report_id
        )
    }
}

/// Predicate observing that the event with `event_id` is durably recorded.
pub(crate) fn event_persisted(event_id: EventId) -> impl Fn(&DomainEventEnvelope) -> bool {
    move |env| env.id == event_id
}

/// Predicate observing that the `ApprovalRecorded` event for `approval_id`
/// with the requested outcome is durably recorded *and* the approval
/// projection reflects that outcome.
///
/// The projection is the read model consumed by the caller, so the barrier
/// confirms both the event log (source of truth) and the projection (the
/// observable outcome) before returning.
pub(crate) fn approval_resolved(
    event_id: EventId,
    approval_id: ApprovalId,
    approved: bool,
    approval_repo: &dyn ApprovalRepository,
) -> impl Fn(&DomainEventEnvelope) -> bool + '_ {
    move |env| {
        if env.id != event_id {
            return false;
        }
        let event_matches = matches!(
            &env.event,
            DomainEvent::ApprovalRecorded {
                approval_id: id,
                outcome,
            } if *id == approval_id && outcome.approved() == approved
        );
        if !event_matches {
            return false;
        }
        match approval_repo.find_by_id(approval_id) {
            Ok(Some(record)) => {
                record.status
                    == if approved {
                        maestria_ports::ApprovalStatus::Approved
                    } else {
                        maestria_ports::ApprovalStatus::Denied
                    }
            }
            Ok(None) => false,
            Err(error) => {
                tracing::error!(
                    %error,
                    "failed to read approval projection during persistence barrier"
                );
                false
            }
        }
    }
}

/// Predicate observing that a `ParserStarted` event for the given artifact,
/// blob, and content hash is durably recorded.
pub(crate) fn parser_started(
    artifact_id: ArtifactId,
    blob_id: BlobId,
    content_hash: String,
) -> impl Fn(&DomainEventEnvelope) -> bool {
    move |env| {
        matches!(
            &env.event,
            DomainEvent::ParserStarted {
                artifact_id: id,
                blob_id: bid,
                content_hash: hash,
                ..
            } if *id == artifact_id && *bid == blob_id && *hash == content_hash
        )
    }
}
