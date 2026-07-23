use maestria_domain::*;

// ── Search, knowledge, and web effects ────────────────────────────

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
fn search_knowledge_completed_emits_event() -> Result<(), DomainError> {
    let mut state = KernelState::new();

    let outcome: maestria_domain::SearchOutcome = serde_json::from_str(
        r#"{
        "trace": 1,
        "fingerprint": "fp1",
        "index_generation": 1,
        "status": "Answerable",
        "evidence": [],
        "coverage": {
            "percent_covered": 100,
            "gaps_identified": []
        },
        "conflicts": []
    }"#,
    )
    .map_err(|_| DomainError::InternalInvariantViolation {
        detail: "search outcome fixture must deserialize",
    })?;

    let output = state.apply_input(DomainInput::SearchKnowledgeCompleted(
        maestria_domain::SearchKnowledgeCompleted {
            task_id: None,
            plan: None,
            outcome: outcome.clone(),
        },
    ))?;

    assert_eq!(output.events.len(), 1);
    let envelope = &output.events[0];
    match &envelope.event {
        DomainEvent::SearchKnowledgeCompleted { outcome: out, .. } => {
            assert_eq!(out.trace, outcome.trace);
        }
        _ => {
            return Err(DomainError::InternalInvariantViolation {
                detail: "expected SearchKnowledgeCompleted event",
            });
        }
    }
    Ok(())
}

#[test]
fn search_knowledge_request_emits_effect() -> Result<(), DomainError> {
    let plan = SearchPlan {
        query_id: QueryId::new(1),
        original_query: "find notes".to_string(),
        intent: SearchIntent::ExactLookup,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(100, 1_000).map_err(|_| {
            DomainError::InternalInvariantViolation {
                detail: "search budget fixture must be valid",
            }
        })?,
        stop_conditions: StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: vec![],
            required_subquestions: vec![],
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        },
        fingerprint: RetrievalModelFingerprint::new("test-model".to_string()).map_err(|_| {
            DomainError::InternalInvariantViolation {
                detail: "search fingerprint fixture must be valid",
            }
        })?,
        original_intent: None,
        route_decision: None,
    };
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
