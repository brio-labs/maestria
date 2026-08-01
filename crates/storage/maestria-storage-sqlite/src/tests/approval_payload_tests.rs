use crate::{SqliteStore, sqlite_store::to_port_error};
use maestria_domain::*;
use maestria_ports::*;
use rusqlite::params;

#[test]
fn approval_recorded_v3_payload_round_trips_transition_and_acknowledgement() -> Result<(), PortError>
{
    let store = SqliteStore::in_memory()?;
    let transition = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ApprovalRecorded {
            approval_id: ApprovalId::new(11),
            outcome: ApprovalOutcome::TaskTransition {
                task_id: TaskId::new(3),
                approved: true,
                from_status: TaskStatus::Draft,
                to_status: TaskStatus::Active,
            },
        },
    };
    let acknowledged = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ApprovalRecorded {
            approval_id: ApprovalId::new(12),
            outcome: ApprovalOutcome::Acknowledged {
                task_id: None,
                approved: false,
            },
        },
    };
    store.append(transition.clone())?;
    store.append(acknowledged.clone())?;
    assert_eq!(
        store.scan(EventFilter { artifact_id: None })?,
        vec![transition, acknowledged]
    );
    Ok(())
}

#[test]
fn approval_recorded_v2_flat_payload_upcasts_transition_and_acknowledgement()
-> Result<(), PortError> {
    let store = SqliteStore::in_memory()?;
    {
        let connection = store.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events \
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'approval_recorded', NULL, ?1, 2)",
                params![r#"{"event_kind":"approval_recorded","approval_id":11,"task_id":3,"approved":true,"from_status":"draft","to_status":"active"}"#],
            )
            .map_err(to_port_error)?;
        connection
            .execute(
                "INSERT INTO domain_events \
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (2, 2, 'approval_recorded', NULL, ?1, 2)",
                params![r#"{"event_kind":"approval_recorded","approval_id":12,"task_id":null,"approved":false,"from_status":null,"to_status":null}"#],
            )
            .map_err(to_port_error)?;
    }
    let events = store.scan(EventFilter { artifact_id: None })?;
    assert!(matches!(
        events.first(),
        Some(DomainEventEnvelope {
            event: DomainEvent::ApprovalRecorded {
                approval_id,
                outcome: ApprovalOutcome::TaskTransition {
                    task_id,
                    approved: true,
                    from_status: TaskStatus::Draft,
                    to_status: TaskStatus::Active,
                },
                ..
            },
            ..
        }) if approval_id.value() == 11 && task_id.value() == 3
    ));
    assert!(matches!(
        events.get(1),
        Some(DomainEventEnvelope {
            event: DomainEvent::ApprovalRecorded {
                approval_id,
                outcome: ApprovalOutcome::Acknowledged {
                    task_id: None,
                    approved: false,
                },
                ..
            },
            ..
        }) if approval_id.value() == 12
    ));
    Ok(())
}

#[test]
fn approval_recorded_v2_invalid_combo_is_rejected_fail_closed() -> Result<(), PortError> {
    for (payload, label) in [
        (
            r#"{"event_kind":"approval_recorded","approval_id":11,"task_id":null,"approved":true,"from_status":"draft","to_status":null}"#,
            "mixed status fields",
        ),
        (
            r#"{"event_kind":"approval_recorded","approval_id":12,"task_id":null,"approved":true,"from_status":"draft","to_status":"active"}"#,
            "taskless transition",
        ),
    ] {
        let store = SqliteStore::in_memory()?;
        {
            let connection = store.lock()?;
            connection
                .execute(
                    "INSERT INTO domain_events \
                         (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'approval_recorded', NULL, ?1, 2)",
                    params![payload],
                )
                .map_err(to_port_error)?;
        }
        assert!(
            store
                .scan(EventFilter { artifact_id: None })
                .is_err_and(|e| e.is_internal()),
            "{label} must fail closed"
        );
    }
    Ok(())
}

#[test]
fn model_agent_proposal_completed_v2_result_upcasts_to_v3_enum() -> Result<(), PortError> {
    let store = SqliteStore::in_memory()?;
    {
        let connection = store.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events \
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'model_agent_proposal_completed', NULL, ?1, 2)",
                params![r#"{"event_kind":"model_agent_proposal_completed","result":{"run_id":7,"correlation_id":42,"status":"Succeeded","search":null,"harness":null,"validation":null,"memory_candidate":null,"error":null}}"#],
            )
            .map_err(to_port_error)?;
        connection
            .execute(
                "INSERT INTO domain_events \
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (2, 2, 'model_agent_proposal_completed', NULL, ?1, 2)",
                params![r#"{"event_kind":"model_agent_proposal_completed","result":{"run_id":8,"correlation_id":43,"status":"Failed","search":null,"harness":null,"validation":null,"memory_candidate":null,"error":"harness crashed"}}"#],
            )
            .map_err(to_port_error)?;
    }
    let events = store.scan(EventFilter { artifact_id: None })?;
    assert!(matches!(
        events.first(),
        Some(DomainEventEnvelope {
            event: DomainEvent::ModelAgentProposalCompleted {
                result: ModelAgentProposalResult::Succeeded {
                    run_id,
                    correlation_id,
                    search: None,
                    harness: None,
                    validation: None,
                    memory_candidate: None,
                },
            },
            ..
        }) if run_id.value() == 7 && *correlation_id == 42
    ));
    assert!(matches!(
        events.get(1),
        Some(DomainEventEnvelope {
            event: DomainEvent::ModelAgentProposalCompleted {
                result: ModelAgentProposalResult::Failed {
                    run_id,
                    correlation_id,
                    error,
                },
            },
            ..
        }) if run_id.value() == 8 && *correlation_id == 43 && error == "harness crashed"
    ));
    Ok(())
}

#[test]
fn model_agent_proposal_completed_v2_invalid_combo_is_rejected_fail_closed() -> Result<(), PortError>
{
    for (payload, label) in [
        (
            r#"{"event_kind":"model_agent_proposal_completed","result":{"run_id":7,"correlation_id":42,"status":"Succeeded","search":null,"harness":null,"validation":null,"memory_candidate":null,"error":"unexpected"}}"#,
            "success with error",
        ),
        (
            r#"{"event_kind":"model_agent_proposal_completed","result":{"run_id":8,"correlation_id":43,"status":"Failed","search":null,"harness":null,"validation":null,"memory_candidate":null,"error":null}}"#,
            "failure without error",
        ),
    ] {
        let store = SqliteStore::in_memory()?;
        {
            let connection = store.lock()?;
            connection
                .execute(
                    "INSERT INTO domain_events \
                         (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'model_agent_proposal_completed', NULL, ?1, 2)",
                    params![payload],
                )
                .map_err(to_port_error)?;
        }
        assert!(
            store
                .scan(EventFilter { artifact_id: None })
                .is_err_and(|e| e.is_internal()),
            "{label} must fail closed"
        );
    }
    Ok(())
}
