    use super::normalize_batch;
    use crate::types::{CandidateBatch, RetrieverDescriptor};
    use maestria_domain::{
        CorpusScope, CorpusSnapshotId, EvidenceRequirements, FreshnessRequirement,
        IndexGenerationId, Modality, ModalitySet, ProjectionNamespace, QueryId,
        RepresentationName, RetrievalModelFingerprint, SearchBudget, SearchIntent, SearchLaneStatus,
        SearchPlan, SearchStage, StopConditions, TrustZone,
    };
    use maestria_ports::SearchQuery;

    fn namespace(instance: &str) -> Result<ProjectionNamespace, Box<dyn std::error::Error>> {
        Ok(ProjectionNamespace::new(
            instance,
            TrustZone::Verified,
            "documents",
        )?)
    }

    fn plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
        Ok(SearchPlan {
            query_id: QueryId::new(1),
            original_query: "namespace test".to_string(),
            intent: SearchIntent::SemanticDiscovery,
            original_intent: None,
            route_decision: None,
            scope: CorpusScope::Global,
            corpus_snapshot: CorpusSnapshotId::new(1),
            index_generation: IndexGenerationId::new(1),
            freshness: FreshnessRequirement::Any,
            modalities: ModalitySet::new(vec![Modality::Text]),
            stages: vec![SearchStage::InitialRetrieval],
            budgets: SearchBudget::with_resource_limits(32, 100, 1, 1, 0, 1_024, 1)?,
            stop_conditions: StopConditions {
                max_results: 5,
                min_score_threshold: 0,
            },
            evidence_requirements: EvidenceRequirements {
                require_primary_sources: false,
                minimum_corroboration: 1,
                required_claims: Vec::new(),
                required_subquestions: Vec::new(),
                minimum_sources: 1,
                minimum_documents: 1,
                minimum_sections: 1,
            },
            fingerprint: RetrievalModelFingerprint::new("namespace-test-v1".to_string())?,
        })
    }

    #[test]
    fn returned_batch_from_foreign_namespace_fails_closed(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let expected = namespace("instance-a")?;
        let descriptor = RetrieverDescriptor {
            id: "learned_sparse".to_string(),
            modality: "sparse".to_string(),
            representation: RepresentationName::new("sparse_text_v1"),
            generation: IndexGenerationId::new(1),
            namespace: Some(expected),
        };
        let batch = CandidateBatch {
            descriptor: RetrieverDescriptor {
                namespace: Some(namespace("instance-b")?),
                ..descriptor.clone()
            },
            query: "namespace test".to_string(),
            candidates: Vec::new(),
            status: SearchLaneStatus::Empty,
            generation: Some(IndexGenerationId::new(1)),
            bytes_read: 0,
        };
        let query = SearchQuery {
            q: "namespace test".to_string(),
            limit: 5,
            offset: 0,
        };
        let mut web_bytes = 0;
        let normalized = normalize_batch(batch, descriptor, &query, &plan()?, &mut web_bytes);
        assert!(matches!(normalized.status, SearchLaneStatus::Failed { .. }));
        assert!(normalized.candidates.is_empty());
        Ok(())
    }
