use std::collections::BTreeMap;

use crate::{SqliteStore, payloads::StoredEventPayload};
use maestria_domain::*;

fn plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: "migration query".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(2),
        index_generation: IndexGenerationId::new(3),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(64, 1_000)?,
        stop_conditions: StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        },
        fingerprint: RetrievalModelFingerprint::new("migration-model-v1".to_string())?,
        original_intent: None,
        route_decision: None,
    })
}

fn lexical_scores() -> Result<RetrievalScoreSet, Box<dyn std::error::Error>> {
    let representation = RepresentationName::new("lexical_text_v1");
    Ok(RetrievalScoreSet::single(RetrievalLaneScore::new(
        RetrievalScoreKind::LexicalBm25,
        91,
        RetrievalRawRank::ranked(1),
        RetrievalScoreScale::unbounded("bm25"),
        representation.clone(),
        RetrievalScoreFingerprint::new(
            RetrievalModelFingerprint::new("migration-lexical-v1".to_string())?,
            BTreeMap::from([("representation".to_string(), representation.0)]),
        ),
    ))?)
}

fn candidate() -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    Ok(EvidenceCandidate {
        evidence_id: EvidenceId::new(10),
        artifact_version: ArtifactVersionId::new(11),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::File {
                path: "fixture.md".to_string(),
                start_line: 1,
                end_line: 1,
            },
            ContentRange { start: 1, end: 1 },
        )?,
        scores: lexical_scores()?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::LexicalMatch],
        coverage_keys: Vec::new(),
    })
}

fn legacy_payloads() -> Result<(String, String, SearchTraceId), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let candidate = candidate()?;
    let mut trace = SearchTrace::from_plan(
        &plan,
        vec!["lexical".to_string()],
        std::slice::from_ref(&candidate),
        Vec::new(),
        None,
        Vec::new(),
        SearchStopReason::EvidenceComplete,
    );
    trace.identity_version = 5;
    let old_trace = trace.deterministic_id();
    let outcome = SearchOutcome {
        trace: old_trace,
        trace_data: Some(Box::new(trace)),
        fingerprint: plan.fingerprint.clone(),
        index_generation: plan.index_generation,
        status: SearchStatus::Answerable,
        evidence: vec![candidate],
        coverage: EvidenceCoverage {
            percent_covered: 100,
            gaps_identified: Vec::new(),
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            distinct_sources: 1,
            distinct_documents: 1,
            distinct_sections: 1,
            candidate_coverage_keys: Vec::new(),
        },
        conflicts: Vec::new(),
    };
    let knowledge = StoredEventPayload::SearchKnowledgeCompleted {
        task_id: None,
        plan: Some(Box::new(plan.clone())),
        outcome,
    };
    let metadata = EvidencePackMetadataRecord {
        query_id: plan.query_id,
        search_trace: Some(old_trace),
        corpus_snapshot: plan.corpus_snapshot,
        index_generation: plan.index_generation,
        fingerprint: plan.fingerprint,
        policy_fingerprint: Some("policy-v1".to_string()),
        claims_required: Vec::new(),
        requirements: plan.evidence_requirements,
        claim_coverage: Vec::new(),
        source_independence: Vec::new(),
        card_count: 0,
        distinct_sources: 1,
        distinct_documents: 1,
        distinct_sections: 1,
        primary_sources_verified: true,
        freshness: Vec::new(),
        conflicts: Vec::new(),
        counterevidence: Vec::new(),
        missing_evidence: Vec::new(),
        compression: EvidencePackCompressionRecord::Verbatim {
            evidence_ids: vec![EvidenceId::new(10)],
        },
        stop_reason: SearchStopReason::EvidenceComplete,
        reproducibility: EvidencePackReproducibilityRecord::Frozen(EvidencePackReplayKeyRecord {
            trace: old_trace,
            corpus_snapshot: CorpusSnapshotId::new(2),
            index_generation: IndexGenerationId::new(3),
            fingerprint: RetrievalModelFingerprint::new("migration-model-v1".to_string())?,
            policy_fingerprint: "policy-v1".to_string(),
        }),
    };
    let search = StoredEventPayload::SearchExecuted {
        query: "migration query".to_string(),
        limit: 5,
        evidence_ids: vec![10],
        pack_metadata: Some(Box::new(metadata)),
        at: 1,
    };

    let mut knowledge_value = serde_json::to_value(knowledge)?;
    knowledge_value["outcome"]["evidence"][0]["scores"] =
        serde_json::json!({"bm25": 91, "semantic_similarity": 0});
    knowledge_value["outcome"]["trace_data"]["raw_candidates"][0]["scores"] =
        serde_json::json!({"bm25": 91, "semantic_similarity": 0});
    Ok((
        serde_json::to_string(&knowledge_value)?,
        serde_json::to_string(&search)?,
        old_trace,
    ))
}

#[test]
fn v8_migration_rewrites_scores_and_all_trace_references() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("score-migration.db");
    let store = SqliteStore::open(&path)?;
    let (knowledge, search, old_trace) = legacy_payloads()?;
    {
        let connection = store.lock()?;
        connection.execute("DELETE FROM schema_version", [])?;
        connection.execute("INSERT INTO schema_version (version) VALUES (8)", [])?;
        connection.execute(
            "INSERT INTO domain_events
             (id, sequence, event_kind, artifact_id, payload_json, payload_version)
             VALUES (1, 1, 'search_knowledge_completed', NULL, ?1, 2)",
            [knowledge],
        )?;
        connection.execute(
            "INSERT INTO domain_events
             (id, sequence, event_kind, artifact_id, payload_json, payload_version)
             VALUES (2, 2, 'search_executed', NULL, ?1, 2)",
            [search],
        )?;
    }
    drop(store);

    let migrated = SqliteStore::open(&path)?;
    let connection = migrated.lock()?;
    let version: i64 =
        connection.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })?;
    assert_eq!(version, 9);
    let knowledge_json: String = connection.query_row(
        "SELECT payload_json FROM domain_events WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    assert!(!knowledge_json.contains("\"bm25\""));
    assert!(!knowledge_json.contains("semantic_similarity"));
    let knowledge: StoredEventPayload = serde_json::from_str(&knowledge_json)?;
    let StoredEventPayload::SearchKnowledgeCompleted { outcome, .. } = knowledge else {
        return Err("migrated knowledge payload has the wrong kind".into());
    };
    assert_ne!(outcome.trace, old_trace);
    assert_eq!(
        outcome
            .trace_data
            .as_deref()
            .map(SearchTrace::deterministic_id),
        Some(outcome.trace)
    );
    assert_eq!(outcome.evidence[0].scores.schema_version, 2);

    let search_json: String = connection.query_row(
        "SELECT payload_json FROM domain_events WHERE id = 2",
        [],
        |row| row.get(0),
    )?;
    let search: StoredEventPayload = serde_json::from_str(&search_json)?;
    let StoredEventPayload::SearchExecuted {
        pack_metadata: Some(metadata),
        ..
    } = search
    else {
        return Err("migrated search payload is missing evidence-pack metadata".into());
    };
    assert_eq!(metadata.search_trace, Some(outcome.trace));
    let EvidencePackReproducibilityRecord::Frozen(key) = metadata.reproducibility else {
        return Err("migrated evidence pack lost its frozen replay key".into());
    };
    assert_eq!(key.trace, outcome.trace);
    let before = (knowledge_json, search_json);
    drop(connection);
    drop(migrated);

    let reopened = SqliteStore::open(&path)?;
    let connection = reopened.lock()?;
    let after = (
        connection.query_row(
            "SELECT payload_json FROM domain_events WHERE id = 1",
            [],
            |row| row.get::<_, String>(0),
        )?,
        connection.query_row(
            "SELECT payload_json FROM domain_events WHERE id = 2",
            [],
            |row| row.get::<_, String>(0),
        )?,
    );
    assert_eq!(before, after);
    Ok(())
}
