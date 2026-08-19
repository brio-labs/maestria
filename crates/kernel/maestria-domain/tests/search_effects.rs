use maestria_domain::*;

// ── Search, knowledge, and web effects ────────────────────────────

fn search_plan() -> Result<SearchPlan, DomainError> {
    SearchPlan::builder()
        .query_id(QueryId::new(1))
        .original_query("find notes".to_string())
        .intent(SearchIntent::ExactLookup)
        .scope(CorpusScope::Global)
        .corpus_snapshot(CorpusSnapshotId::new(1))
        .index_generation(IndexGenerationId::new(1))
        .freshness(FreshnessRequirement::Any)
        .modalities(ModalitySet::new(vec![Modality::Text]))
        .stages(vec![SearchStage::InitialRetrieval])
        .budgets(SearchBudget::new(100, 1_000).map_err(|_| {
            DomainError::InternalInvariantViolation {
                detail: "search budget fixture must be valid",
            }
        })?)
        .stop_conditions(StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        })
        .evidence_requirements(EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: vec![],
            required_subquestions: vec![],
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        })
        .fingerprint(
            RetrievalModelFingerprint::new("test-model".to_string()).map_err(|_| {
                DomainError::InternalInvariantViolation {
                    detail: "search fingerprint fixture must be valid",
                }
            })?,
        )
        .authorization(maestria_domain::RetrievalPolicySnapshot::global_default())
        .build()
        .map_err(|error| DomainError::SearchIncompatible { error })
}

fn no_evidence_outcome(plan: &SearchPlan) -> Result<SearchOutcome, DomainError> {
    let policy_fingerprint = plan.authorization().canonical_fingerprint();
    let trace = SearchTrace::from_plan(
        plan,
        vec![],
        &[],
        vec![],
        None,
        vec![],
        SearchStopReason::NoEvidence,
    )
    .map_err(|error| DomainError::SearchIncompatible { error })?
    .with_policy_fingerprint(policy_fingerprint);
    Ok(SearchOutcome::from_trace(
        trace,
        plan,
        SearchStatus::NoEvidenceFound,
        vec![],
        EvidenceCoverage::new(EvidenceCoverageDto {
            percent_covered: 0,
            gaps_identified: vec![],
            required_claims: vec![],
            required_subquestions: vec![],
            distinct_sources: 0,
            distinct_documents: 0,
            distinct_sections: 0,
            candidate_coverage_keys: vec![],
        })
        .map_err(|error| DomainError::SearchIncompatible { error })?,
        vec![],
    ))
}

#[test]
fn search_executed_emits_audit_event_with_evidence_ids() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::SearchExecuted(SearchExecutedInput {
        query: "hello world".to_string(),
        limit: 10,
        evidence_ids: vec![EvidenceId::new(1), EvidenceId::new(2)],
        pack_metadata: None,
        at: LogicalTick::new(42),
    }))?;

    assert_eq!(output.events.len(), 1);
    assert_eq!(output.effects.len(), 1);
    let envelope = &output.events[0];
    match &envelope.event {
        DomainEvent::SearchExecuted {
            query,
            limit,
            evidence_ids,
            pack_metadata,
            at,
        } => {
            assert_eq!(query, "hello world");
            assert_eq!(*limit, 10);
            assert_eq!(evidence_ids, &vec![EvidenceId::new(1), EvidenceId::new(2)]);
            assert!(pack_metadata.is_none());
            assert_eq!(*at, LogicalTick::new(42));
        }
        _ => {
            return Err(DomainError::InternalInvariantViolation {
                detail: "expected SearchExecuted event",
            });
        }
    }
    // Audit events must not mutate any entity collections.
    assert!(state.artifacts.is_empty());
    assert!(state.cards.is_empty());
    assert!(state.evidences.is_empty());
    assert_eq!(state.event_log.len(), 1);
    Ok(())
}

#[test]
fn search_executed_rejects_empty_query() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let err = match state.apply_input(DomainInput::SearchExecuted(SearchExecutedInput {
        query: "   ".to_string(),
        limit: 5,
        evidence_ids: vec![],
        pack_metadata: None,
        at: LogicalTick::new(1),
    })) {
        Ok(_) => return Err(std::io::Error::other("empty query must be rejected").into()),
        Err(error) => error,
    };
    assert!(matches!(err, DomainError::EmptyIntent));
    Ok(())
}

#[test]
fn search_executed_is_deterministic_on_replay() -> Result<(), DomainError> {
    let mut state_a = KernelState::new();
    state_a.apply_input(DomainInput::SearchExecuted(SearchExecutedInput {
        query: "deterministic".to_string(),
        limit: 3,
        evidence_ids: vec![EvidenceId::new(10)],
        pack_metadata: None,
        at: LogicalTick::new(7),
    }))?;

    let replayed = replay_events(&state_a.event_log)?;
    assert_eq!(state_a, replayed);
    Ok(())
}

#[test]
fn search_executed_persist_effect_matches_event_envelope() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::SearchExecuted(SearchExecutedInput {
        query: "audit".to_string(),
        limit: 1,
        evidence_ids: vec![],
        pack_metadata: None,
        at: LogicalTick::new(1),
    }))?;

    let envelope = match output.effects.as_slice() {
        [MaestriaEffect::PersistEvent { envelope }] => envelope,
        _ => return Err(DomainError::EmptyIntent),
    };
    assert_eq!(envelope.as_ref(), &output.events[0]);
    Ok(())
}

#[test]
fn search_knowledge_completed_emits_only_compatible_event() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let plan = search_plan()?;
    let outcome = no_evidence_outcome(&plan)?;

    let output = state.apply_input(DomainInput::SearchKnowledgeCompleted(
        maestria_domain::SearchKnowledgeCompleted {
            task_id: None,
            plan: Box::new(plan),
            outcome: outcome.clone(),
        },
    ))?;

    assert_eq!(output.events.len(), 1);
    let envelope = &output.events[0];
    match &envelope.event {
        DomainEvent::SearchKnowledgeCompleted {
            plan: Some(recorded_plan),
            outcome: recorded_outcome,
            ..
        } => {
            assert_eq!(recorded_outcome.trace, outcome.trace);
            assert_eq!(recorded_plan.fingerprint(), &outcome.fingerprint);
        }
        _ => {
            return Err(DomainError::InternalInvariantViolation {
                detail: "expected traced SearchKnowledgeCompleted event",
            });
        }
    }
    Ok(())
}

#[test]
fn search_knowledge_completed_rejects_missing_trace_atomically() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let plan = search_plan()?;
    let mut outcome = no_evidence_outcome(&plan)?;
    outcome.trace_data = None;

    let result = state.apply_input(DomainInput::SearchKnowledgeCompleted(
        maestria_domain::SearchKnowledgeCompleted {
            task_id: None,
            plan: Box::new(plan),
            outcome,
        },
    ));

    assert!(matches!(
        result,
        Err(DomainError::SearchIncompatible {
            error: SearchCompatibilityError::TracePlanMismatch("trace data is missing")
        })
    ));
    assert!(state.event_log.is_empty());
    Ok(())
}

#[test]
fn search_knowledge_request_emits_effect() -> Result<(), DomainError> {
    let plan = search_plan()?;
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::SearchKnowledgeRequested(
        SearchKnowledgeRequested {
            task_id: None,
            plan: plan.clone(),
        },
    ))?;
    match output.effects.as_slice() {
        [MaestriaEffect::SearchKnowledge(request)] => {
            assert_eq!(request.plan, plan);
        }
        _ => {
            return Err(DomainError::InternalInvariantViolation {
                detail: "expected SearchKnowledge effect",
            });
        }
    }
    assert!(output.events.is_empty());
    Ok(())
}

#[test]
fn search_plan_construction_rejects_whitespace_query() -> Result<(), DomainError> {
    let plan = search_plan()?;
    assert!(matches!(
        plan.with_original_query("   ".to_string()),
        Err(SearchCompatibilityError::InvalidPlan(
            "original_query must not be empty"
        ))
    ));
    Ok(())
}

#[test]
fn fetch_web_request_emits_fetch_effect() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::FetchWebRequested(FetchWebRequested {
        request: FetchWebRequest {
            url: "https://example.com/research".to_string(),
            max_bytes: 4096,
            max_requests: 1,
            max_latency_ms: 15_000,
            allowed_domains: Vec::new(),
            allowed_content_types: Vec::new(),
        },
    }))?;

    assert!(output.events.is_empty());
    assert_eq!(
        output.effects,
        vec![MaestriaEffect::FetchWeb(FetchWebRequest {
            url: "https://example.com/research".to_string(),
            max_bytes: 4096,
            max_requests: 1,
            max_latency_ms: 15_000,
            allowed_domains: Vec::new(),
            allowed_content_types: Vec::new(),
        })]
    );
    Ok(())
}
