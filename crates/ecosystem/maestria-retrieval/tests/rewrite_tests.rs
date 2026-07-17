use maestria_retrieval::rewrite::{
    DeterministicRewriter, QueryRewriteRecord, QueryRewriteSession, RewriteAccounting,
    RewriteOrigin, StageRole,
};

#[test]
fn test_original_identity() {
    let session = QueryRewriteSession::new("test query PR");
    assert_eq!(session.original_query(), "test query PR");

    let records = session.records();
    assert_eq!(records.len(), 1);

    let original_record = &records[0];
    assert_eq!(original_record.query, "test query PR");
    assert_eq!(original_record.origin, RewriteOrigin::Original);
    assert_eq!(original_record.stage, StageRole::InitialRetrieval);
}

#[test]
fn test_deterministic_before_proposal_ordering() {
    let mut session = QueryRewriteSession::new("query");
    session.expand_deterministic();

    // Add model proposal first
    session.add_rewrite(QueryRewriteRecord {
        query: "proposal".to_string(),
        origin: RewriteOrigin::ModelProposal,
        stage: StageRole::Reranking,
        accounting: RewriteAccounting {
            token_estimate: 1,
            latency_budget_units: 1,
            is_proposal: true,
        },
        missing_slot: None,
    });

    // Add deterministic second
    session.add_rewrite(QueryRewriteRecord {
        query: "deterministic".to_string(),
        origin: RewriteOrigin::Deterministic,
        stage: StageRole::InitialRetrieval,
        accounting: RewriteAccounting {
            token_estimate: 1,
            latency_budget_units: 1,
            is_proposal: false,
        },
        missing_slot: None,
    });

    let records = session.records();
    // Order should be Original, Deterministic, ModelProposal
    assert_eq!(records[0].origin, RewriteOrigin::Original);
    assert_eq!(records[1].origin, RewriteOrigin::Deterministic);
    assert_eq!(records[2].origin, RewriteOrigin::ModelProposal);
}

#[test]
fn test_policy_stage_restrictions() {
    let proposal_initial = QueryRewriteRecord {
        query: "proposal".to_string(),
        origin: RewriteOrigin::ModelProposal,
        stage: StageRole::InitialRetrieval,
        accounting: RewriteAccounting {
            token_estimate: 1,
            latency_budget_units: 1,
            is_proposal: true,
        },
        missing_slot: None,
    };

    let proposal_reranking = QueryRewriteRecord {
        query: "proposal".to_string(),
        origin: RewriteOrigin::ModelProposal,
        stage: StageRole::Reranking,
        accounting: RewriteAccounting {
            token_estimate: 1,
            latency_budget_units: 1,
            is_proposal: true,
        },
        missing_slot: None,
    };

    assert!(!QueryRewriteSession::policy_accepts(
        &proposal_initial,
        None
    ));
    assert!(QueryRewriteSession::policy_accepts(
        &proposal_reranking,
        None
    ));
}

#[test]
fn test_policy_missing_slot_gating() {
    let missing_slot_record = QueryRewriteRecord {
        query: "fill slot".to_string(),
        origin: RewriteOrigin::MissingSlot,
        stage: StageRole::IterativeRetrieval,
        accounting: RewriteAccounting {
            token_estimate: 1,
            latency_budget_units: 1,
            is_proposal: false,
        },
        missing_slot: Some("missing claim".to_string()),
    };

    // Reject if no context
    assert!(!QueryRewriteSession::policy_accepts(
        &missing_slot_record,
        None
    ));

    // Reject if empty context
    assert!(!QueryRewriteSession::policy_accepts(
        &missing_slot_record,
        Some("   ")
    ));

    // Accept if non-empty context
    assert!(QueryRewriteSession::policy_accepts(
        &missing_slot_record,
        Some("context data")
    ));
}

#[test]
fn test_budget_accounting() {
    let record = QueryRewriteRecord {
        query: "test accounting budget".to_string(),
        origin: RewriteOrigin::ModelProposal,
        stage: StageRole::Reranking,
        accounting: RewriteAccounting {
            token_estimate: 3,
            latency_budget_units: 10,
            is_proposal: true,
        },
        missing_slot: None,
    };

    assert_eq!(record.accounting.token_estimate, 3);
    assert_eq!(record.accounting.latency_budget_units, 10);
    assert!(record.accounting.is_proposal);
}

#[test]
fn test_deterministic_expansions_and_deduplication() {
    let mut session = QueryRewriteSession::new("PR #123 in src/module");
    session.expand_deterministic();

    // Should expand to:
    // "Pull Request #123 in src/module"
    // "PR Issue 123 in src/module"
    // "PR #123 in crates/module"
    // "PR #123 in src/mod"
    let queries: Vec<_> = session.records().iter().map(|r| r.query.clone()).collect();

    assert!(queries.contains(&"Pull Request #123 in src/module".to_string()));
    assert!(queries.contains(&"PR Issue 123 in src/module".to_string()));
    assert!(queries.contains(&"PR #123 in crates/module".to_string()));
    assert!(queries.contains(&"PR #123 in src/mod".to_string()));

    let original_len = session.records().len();

    // Trying to add exact duplicate should not increase length
    session.add_rewrite(QueryRewriteRecord {
        query: "Pull Request #123 in src/module".to_string(),
        origin: RewriteOrigin::Deterministic,
        stage: StageRole::InitialRetrieval,
        accounting: RewriteAccounting {
            token_estimate: 6,
            latency_budget_units: 1,
            is_proposal: false,
        },
        missing_slot: None,
    });

    assert_eq!(session.records().len(), original_len);
}

#[test]
fn test_date_normalization_is_deterministic() {
    let expansions = DeterministicRewriter::expand("decision 2026-07-16");
    assert!(
        expansions
            .iter()
            .any(|query| query == "decision date 2026-07-16")
    );
}
#[test]
fn test_model_proposal_requires_deterministic_phase() {
    let mut session = QueryRewriteSession::new("query");
    let proposal = QueryRewriteRecord {
        query: "proposal".to_string(),
        origin: RewriteOrigin::ModelProposal,
        stage: StageRole::Reranking,
        accounting: RewriteAccounting {
            token_estimate: 1,
            latency_budget_units: 1,
            is_proposal: true,
        },
        missing_slot: None,
    };
    assert!(!session.add_rewrite(proposal.clone()));
    session.expand_deterministic();
    assert!(session.add_rewrite(proposal));
}

#[test]
fn test_rewrite_budgets_are_enforced() {
    let mut session = QueryRewriteSession::with_budget("query", 1, 1);
    session.expand_deterministic();
    assert!(!session.add_rewrite(QueryRewriteRecord {
        query: "proposal".to_string(),
        origin: RewriteOrigin::ModelProposal,
        stage: StageRole::Reranking,
        accounting: RewriteAccounting {
            token_estimate: 1,
            latency_budget_units: 1,
            is_proposal: true,
        },
        missing_slot: None,
    }));
    assert_eq!(session.records().len(), 1);
}

#[test]
fn test_missing_slot_rewrite_requires_named_slot() {
    let mut session = QueryRewriteSession::new("query").with_missing_slots([String::from("claim")]);
    session.expand_deterministic();
    assert!(session.add_missing_slot_rewrite(
        "fill claim",
        "claim",
        RewriteAccounting {
            token_estimate: 2,
            latency_budget_units: 1,
            is_proposal: false,
        },
    ));
    let record = session
        .records()
        .iter()
        .find(|record| record.origin == RewriteOrigin::MissingSlot)
        .expect("missing-slot rewrite should be recorded");
    assert_eq!(record.missing_slot.as_deref(), Some("claim"));
    assert_eq!(record.stage, StageRole::IterativeRetrieval);
}
#[test]
fn test_missing_slot_rewrite_rejects_unidentified_slot() {
    let mut session = QueryRewriteSession::new("query").with_missing_slots([String::from("claim")]);
    assert!(!session.add_missing_slot_rewrite(
        "fill unrelated",
        "other",
        RewriteAccounting {
            token_estimate: 2,
            latency_budget_units: 1,
            is_proposal: false,
        },
    ));
}
#[test]
fn test_query_budget_limits_rewrite_count() {
    let mut session = QueryRewriteSession::with_limits("PR query", 20, 10, 1);
    session.expand_deterministic();
    assert_eq!(session.records().len(), 1);
}
#[test]
fn test_original_query_is_traced_when_budget_is_smaller() {
    let session = QueryRewriteSession::with_budget("multi word query", 1, 1);
    assert_eq!(session.records().len(), 1);
    assert_eq!(session.records()[0].origin, RewriteOrigin::Original);
}
