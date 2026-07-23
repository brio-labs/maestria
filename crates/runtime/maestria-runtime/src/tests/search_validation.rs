use crate::test_helpers;
use crate::tests::run_complete_task_test;
use crate::{EffectExecutionContext, MaestriaRuntime};
use maestria_domain::{
    DomainEvent, DomainInput, KernelState, Task, TaskId, TaskStatus, ValidationReportId,
};
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::test]
async fn search_validation_failure_records_a_failed_report()
-> Result<(), Box<dyn std::error::Error>> {
    let task_id = TaskId::new(7);
    let report_id = ValidationReportId::new(8);
    let mut state = KernelState::new();
    state.tasks.insert(
        task_id,
        Task {
            id: task_id,
            title: "search validation".to_string(),
            priority: maestria_domain::TaskPriority::Normal,
            status: TaskStatus::Validating,
            validation_report_id: None,
            artifact_ids: Default::default(),
            evidence_ids: Default::default(),
        },
    );
    let outcome = maestria_domain::SearchOutcome {
        trace: maestria_domain::SearchTraceId::new(1),
        trace_data: None,
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("fixture".to_string())?,
        index_generation: maestria_domain::IndexGenerationId::new(1),
        status: maestria_domain::SearchStatus::Answerable,
        evidence: Vec::new(),
        coverage: maestria_domain::EvidenceCoverage {
            percent_covered: 0,
            gaps_identified: vec!["required evidence".to_string()],
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            distinct_sources: 0,
            distinct_documents: 0,
            distinct_sections: 0,
            candidate_coverage_keys: Vec::new(),
        },
        conflicts: Vec::new(),
    };
    state.event_log.push(maestria_domain::DomainEventEnvelope {
        id: maestria_domain::EventId::new(1),
        sequence: maestria_domain::SequenceNumber::new(1),
        event: DomainEvent::SearchKnowledgeCompleted {
            task_id: Some(task_id),
            plan: None,
            outcome,
        },
    });

    let (input_tx, mut input_rx) = tokio::sync::mpsc::channel(8);
    let ctx = EffectExecutionContext::test_default(
        Arc::new(test_helpers::test_adapters()),
        Arc::new(test_helpers::test_governance()),
        Arc::new(RwLock::new(state)),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        maestria_domain::MaestriaEffect::RunValidation(maestria_domain::RunValidationRequest {
            task_id: Some(task_id),
            claim_id: None,
            validation_report_id: report_id,
        }),
        ctx,
        None,
    )
    .await;
    assert!(result);

    match input_rx
        .recv()
        .await
        .ok_or("validation report input missing")?
    {
        DomainInput::RecordValidationReport(input) => {
            assert_eq!(input.report_id, report_id);
            assert_eq!(input.task_id, Some(task_id));
            assert!(!input.passed);
            assert!(input.warnings.is_empty());
        }
        other => return Err(format!("unexpected validation input: {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn completion_rejects_a_forged_passing_search_report()
-> Result<(), Box<dyn std::error::Error>> {
    let task_id = TaskId::new(9);
    let report_id = ValidationReportId::new(10);
    let task = Task {
        id: task_id,
        title: "search completion".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    let outcome = maestria_domain::SearchOutcome {
        trace: maestria_domain::SearchTraceId::new(1),
        trace_data: None,
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("fixture".to_string())?,
        index_generation: maestria_domain::IndexGenerationId::new(1),
        status: maestria_domain::SearchStatus::Answerable,
        evidence: Vec::new(),
        coverage: maestria_domain::EvidenceCoverage {
            percent_covered: 0,
            gaps_identified: vec!["required evidence".to_string()],
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            distinct_sources: 0,
            distinct_documents: 0,
            distinct_sections: 0,
            candidate_coverage_keys: Vec::new(),
        },
        conflicts: Vec::new(),
    };
    let events = vec![
        DomainEvent::SearchKnowledgeCompleted {
            task_id: Some(task_id),
            plan: None,
            outcome,
        },
        DomainEvent::ValidationReportCreated {
            report_id,
            task_id: Some(task_id),
            passed: true,
            warnings: Vec::new(),
        },
    ];
    let mut state = KernelState::new();
    state.tasks.insert(task_id, task);
    state.validation_reports.insert(
        report_id,
        maestria_domain::ValidationReportRecord {
            task_id: Some(task_id),
            passed: true,
            warnings: Vec::new(),
        },
    );

    let events = run_complete_task_test(
        state,
        test_helpers::test_governance(),
        task_id,
        report_id,
        events,
    )
    .await?;
    assert!(events.is_empty(), "forged search report was accepted");
    Ok(())
}

#[tokio::test]
async fn associated_search_coverage_and_conflicts_block_verified_completion()
-> Result<(), Box<dyn std::error::Error>> {
    let task_id = TaskId::new(21);
    let mut state = KernelState::new();
    state.tasks.insert(
        task_id,
        Task {
            id: task_id,
            title: "associated search".to_string(),
            priority: maestria_domain::TaskPriority::Normal,
            status: TaskStatus::Validating,
            validation_report_id: None,
            artifact_ids: Default::default(),
            evidence_ids: Default::default(),
        },
    );
    state.event_log.push(maestria_domain::DomainEventEnvelope {
        id: maestria_domain::EventId::new(1),
        sequence: maestria_domain::SequenceNumber::new(1),
        event: DomainEvent::SearchKnowledgeCompleted {
            task_id: Some(task_id),
            plan: None,
            outcome: maestria_domain::SearchOutcome {
                trace: maestria_domain::SearchTraceId::new(21),
                trace_data: None,
                fingerprint: maestria_domain::RetrievalModelFingerprint::new(
                    "fixture:associated-search".to_string(),
                )?,
                index_generation: maestria_domain::IndexGenerationId::new(1),
                status: maestria_domain::SearchStatus::SourcesConflict,
                evidence: Vec::new(),
                coverage: maestria_domain::EvidenceCoverage {
                    percent_covered: 50,
                    gaps_identified: vec!["unresolved claim".to_string()],
                    required_claims: vec!["claim".to_string()],
                    required_subquestions: Vec::new(),
                    distinct_sources: 0,
                    distinct_documents: 0,
                    distinct_sections: 0,
                    candidate_coverage_keys: Vec::new(),
                },
                conflicts: vec![maestria_domain::ConflictSet {
                    id: maestria_domain::ConflictSetId::new(1),
                    candidates: Vec::new(),
                }],
            },
        },
    });

    let report = crate::validation::build_validation_report_from_state(
        &state,
        &maestria_domain::RunValidationRequest {
            task_id: Some(task_id),
            claim_id: None,
            validation_report_id: ValidationReportId::new(22),
        },
    );
    assert!(!report.passed);
    assert!(
        report
            .checks
            .iter()
            .any(|check| { check.name == "coverage" && !check.passed })
    );
    assert!(
        report
            .checks
            .iter()
            .any(|check| { check.name == "conflict" && !check.passed })
    );
    Ok(())
}
