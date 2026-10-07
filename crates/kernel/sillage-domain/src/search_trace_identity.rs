use crate::{
    SearchRewriteOrigin, SearchTrace, SearchTraceCandidate, SearchTraceDiversity, SearchTraceId,
    SearchTraceLane, SearchTraceRerank, SearchTraceRewrite,
};

impl SearchTrace {
    /// Deterministic content identity for the trace.
    ///
    /// The mix is version-gate-free: every component participates
    /// unconditionally, so live, degraded, and storage-canonicalized traces
    /// (which previously diverged through per-version gated branches and a
    /// version salt) hash identically. Degraded-search trace lookups against
    /// the durable id therefore succeed.
    pub fn deterministic_id(&self) -> SearchTraceId {
        let mut hash = 0xcbf29ce484222325u64;
        mix_trace_header(&mut hash, self);
        mix_trace_budgets(&mut hash, self);
        mix_trace_stop_conditions(&mut hash, self);
        mix_trace_candidates(&mut hash, &self.raw_candidates, true);
        mix_trace_post_candidates(&mut hash, self);
        mix_trace_lanes(&mut hash, &self.lanes, true, true, true);
        if let Some(rerank) = &self.rerank {
            mix_trace_rerank(&mut hash, rerank);
        }
        if let Some(diversity) = &self.diversity {
            mix_diversity(&mut hash, diversity);
        }
        SearchTraceId::new(hash)
    }
}

fn mix_hash(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

/// Writes `Debug` representation into the hash incrementally,
/// avoiding any heap allocation.
fn mix_debug(hash: &mut u64, value: &impl std::fmt::Debug) {
    struct HashWriter<'a> {
        hash: &'a mut u64,
    }
    impl<'a> std::fmt::Write for HashWriter<'a> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            mix_hash(self.hash, s.as_bytes());
            Ok(())
        }
    }
    use std::fmt::Write;
    let _ = write!(HashWriter { hash }, "{:?}", value);
}

fn mix_trace_header(hash: &mut u64, trace: &SearchTrace) {
    mix_hash(hash, &trace.query_id.value().to_le_bytes());
    mix_hash(hash, trace.original_query.as_bytes());
    mix_debug(hash, &trace.original_intent);
    mix_debug(hash, &trace.route_decision);
    mix_debug(hash, &trace.intent);
    mix_debug(hash, &trace.scope);
    mix_debug(hash, &trace.freshness);
    mix_debug(hash, &trace.modalities);
    mix_debug(hash, &trace.degradation);
    mix_debug(hash, &trace.stages);
    mix_debug(hash, &trace.evidence_requirements);
    mix_hash(hash, &trace.corpus_snapshot.value().to_le_bytes());
    mix_hash(hash, &trace.index_generation.value().to_le_bytes());
    mix_hash(hash, trace.fingerprint.as_str().as_bytes());
    for retriever in &trace.retrievers {
        mix_hash(hash, retriever.as_bytes());
    }
    mix_debug(hash, &trace.policy_fingerprint);
    mix_debug(hash, &trace.source_selection_digest);
}

fn mix_trace_budgets(hash: &mut u64, trace: &SearchTrace) {
    mix_hash(hash, &u64::from(trace.budgets.max_tokens()).to_le_bytes());
    mix_hash(
        hash,
        &u64::from(trace.budgets.max_latency_ms()).to_le_bytes(),
    );
    mix_hash(hash, &u64::from(trace.budgets.max_queries()).to_le_bytes());
    mix_hash(hash, &u64::from(trace.budgets.max_stages()).to_le_bytes());
    mix_hash(
        hash,
        &u64::from(trace.budgets.max_web_requests()).to_le_bytes(),
    );
    mix_hash(hash, &trace.budgets.max_bytes_read().to_le_bytes());
    mix_hash(
        hash,
        &u64::from(trace.budgets.max_concurrency()).to_le_bytes(),
    );
    mix_hash(
        hash,
        &u64::from(trace.budgets.max_candidates()).to_le_bytes(),
    );
    mix_hash(hash, &trace.budgets.max_work_units().to_le_bytes());
}

fn mix_trace_stop_conditions(hash: &mut u64, trace: &SearchTrace) {
    mix_hash(
        hash,
        &u64::from(trace.stop_conditions.max_results).to_le_bytes(),
    );
    mix_hash(
        hash,
        &u64::from(trace.stop_conditions.min_score_threshold).to_le_bytes(),
    );
}

fn mix_trace_post_candidates(hash: &mut u64, trace: &SearchTrace) {
    mix_debug(hash, &trace.fusion);
    mix_trace_rewrites(hash, &trace.rewrites);
    mix_debug(hash, &trace.filters);
    mix_debug(hash, &trace.expansions);
    mix_debug(hash, &trace.missing_evidence);
    for conflict in &trace.conflicts {
        mix_hash(hash, &conflict.value().to_le_bytes());
    }
    mix_debug(hash, &trace.stop_reason);
}

fn mix_trace_candidates(
    hash: &mut u64,
    candidates: &[SearchTraceCandidate],
    complete_score_provenance: bool,
) {
    for candidate in candidates {
        mix_hash(hash, &candidate.evidence_id().value().to_le_bytes());
        mix_hash(hash, &candidate.artifact_version().value().to_le_bytes());
        mix_debug(hash, candidate.source_span());
        mix_hash(hash, &u64::from(candidate.rank()).to_le_bytes());
        mix_scores(hash, candidate.scores(), complete_score_provenance);
        mix_debug(hash, &candidate.trust());
        mix_debug(hash, &candidate.freshness());
        mix_debug(hash, &candidate.duplicate_cluster());
        mix_debug(hash, &candidate.reasons());
        mix_debug(hash, &candidate.coverage_keys());
    }
}

fn mix_trace_rewrites(hash: &mut u64, rewrites: &[SearchTraceRewrite]) {
    for rewrite in rewrites {
        mix_hash(hash, rewrite.query.as_bytes());
        mix_debug(hash, &rewrite.origin);
        mix_debug(hash, &rewrite.stage);
        mix_hash(
            hash,
            &u64::from(rewrite.accounting.token_estimate).to_le_bytes(),
        );
        mix_hash(
            hash,
            &u64::from(rewrite.accounting.latency_budget_units).to_le_bytes(),
        );
        mix_hash(
            hash,
            &[u8::from(
                rewrite.origin == SearchRewriteOrigin::ModelProposal,
            )],
        );
    }
}

fn mix_trace_lanes(
    hash: &mut u64,
    lanes: &[SearchTraceLane],
    include_query: bool,
    complete_score_provenance: bool,
    include_execution: bool,
) {
    for lane in lanes {
        mix_hash(hash, lane.retriever_id.as_bytes());
        if include_query {
            mix_hash(hash, lane.query.as_bytes());
        }
        if complete_score_provenance {
            mix_debug(hash, &lane.generation);
        }
        mix_debug(hash, &lane.status);
        if include_execution {
            mix_debug(hash, &lane.execution);
        }
        for candidate in &lane.candidates {
            mix_hash(hash, &candidate.evidence_id().value().to_le_bytes());
            mix_hash(hash, &candidate.artifact_version().value().to_le_bytes());
            mix_debug(hash, candidate.source_span());
            mix_hash(hash, &u64::from(candidate.lane_rank()).to_le_bytes());
            mix_debug(hash, &candidate.duplicate_cluster());
            mix_scores(hash, candidate.scores(), complete_score_provenance);
            mix_debug(hash, &candidate.reasons());
        }
    }
}

fn mix_scores(hash: &mut u64, scores: &crate::RetrievalScoreSet, complete_score_provenance: bool) {
    if complete_score_provenance {
        mix_hash(hash, &u64::from(scores.schema_version()).to_le_bytes());
        for score in scores.lanes() {
            mix_debug(hash, &score.score_kind);
            mix_hash(hash, &score.raw_score.to_le_bytes());
            mix_debug(hash, &score.raw_rank);
            mix_debug(hash, &score.scale);
            mix_hash(hash, score.representation.0.as_bytes());
            mix_hash(hash, score.fingerprint.identity.as_str().as_bytes());
            for (key, value) in &score.fingerprint.components {
                mix_hash(hash, key.as_bytes());
                mix_hash(hash, value.as_bytes());
            }
        }
        return;
    }

    let mut bm25 = 0_i64;
    let mut semantic = 0_i64;
    for score in scores.lanes() {
        match &score.score_kind {
            crate::RetrievalScoreKind::LexicalBm25 => bm25 = score.raw_score,
            crate::RetrievalScoreKind::DenseSimilarity => semantic = score.raw_score,
            _ => {}
        }
    }
    mix_hash(hash, &bm25.to_le_bytes());
    mix_hash(hash, &semantic.to_le_bytes());
}

fn mix_trace_rerank(hash: &mut u64, rerank: &SearchTraceRerank) {
    mix_hash(hash, rerank.model.as_bytes());
    mix_hash(hash, rerank.fingerprint.as_str().as_bytes());
    mix_hash(hash, &(rerank.input_cap as u64).to_le_bytes());
    mix_hash(hash, &(rerank.score_cap as u64).to_le_bytes());
    mix_hash(hash, &(rerank.output_cap as u64).to_le_bytes());
    for candidate in &rerank.candidates {
        mix_hash(hash, &candidate.candidate_id.value().to_le_bytes());
        mix_hash(hash, &(candidate.original_rank as u64).to_le_bytes());
        mix_debug(hash, &candidate.position);
        mix_debug(hash, &candidate.relevance_score);
        for constraint in &candidate.constraint_scores {
            mix_hash(hash, constraint.name.as_bytes());
            mix_hash(hash, &u64::from(constraint.score).to_le_bytes());
        }
    }
}

fn mix_diversity(hash: &mut u64, diversity: &SearchTraceDiversity) {
    mix_hash(hash, &(diversity.distinct_sources as u64).to_le_bytes());
    mix_hash(hash, &(diversity.distinct_documents as u64).to_le_bytes());
    mix_hash(hash, &(diversity.distinct_sections as u64).to_le_bytes());
    for claim in &diversity.required_claims {
        mix_hash(hash, claim.as_bytes());
    }
    for subquestion in &diversity.required_subquestions {
        mix_hash(hash, subquestion.as_bytes());
    }
    for key in &diversity.covered_keys {
        mix_hash(hash, key.as_bytes());
    }
    mix_debug(hash, &diversity.stop_reason);
    for candidate in &diversity.candidates {
        mix_hash(hash, &candidate.candidate_id.value().to_le_bytes());
        mix_hash(hash, &(candidate.original_rank as u64).to_le_bytes());
        mix_debug(hash, &candidate.placement);
        mix_debug(hash, &candidate.duplicate_cluster);
        mix_hash(hash, &u64::from(candidate.marginal_coverage).to_le_bytes());
        for key in &candidate.coverage_keys {
            mix_hash(hash, key.as_bytes());
        }
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    use crate::{CorpusScope, FreshnessRequirement, SearchIntent, SearchStopReason};

    fn old_mix_debug<T: std::fmt::Debug>(hash: &mut u64, value: &T) {
        mix_hash(hash, format!("{:?}", value).as_bytes());
    }

    #[test]
    fn mix_debug_matches_format_for_enum() {
        let value = SearchIntent::FactualLocal;
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        mix_debug(&mut h1, &value);
        old_mix_debug(&mut h2, &value);
        assert_eq!(h1, h2);
    }

    #[test]
    fn mix_debug_matches_format_for_struct() {
        let value = CorpusScope::Global;
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        mix_debug(&mut h1, &value);
        old_mix_debug(&mut h2, &value);
        assert_eq!(h1, h2);
    }

    #[test]
    fn mix_debug_matches_format_for_option_and_empty_string() {
        let value: Option<FreshnessRequirement> = None;
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        mix_debug(&mut h1, &value);
        old_mix_debug(&mut h2, &value);
        assert_eq!(h1, h2);
    }

    fn fixture_plan(
        query: &str,
        intent: SearchIntent,
    ) -> Result<crate::SearchPlan, Box<dyn std::error::Error>> {
        Ok(crate::SearchPlan::builder()
            .query_id(crate::QueryId::new(7))
            .original_query(query.to_owned())
            .intent(intent)
            .scope(CorpusScope::Global)
            .corpus_snapshot(crate::CorpusSnapshotId::new(11))
            .index_generation(crate::IndexGenerationId::new(13))
            .freshness(FreshnessRequirement::Any)
            .modalities(crate::ModalitySet::new(vec![crate::Modality::Text]))
            .stages(vec![crate::SearchStage::InitialRetrieval])
            .budgets(crate::SearchBudget::with_limits(2_000, 5_000, 1, 2, 0)?)
            .stop_conditions(crate::StopConditions {
                max_results: 10,
                min_score_threshold: 70,
            })
            .evidence_requirements(crate::EvidenceRequirements {
                required_claims: vec![],
                required_subquestions: vec![],
                minimum_sources: 0,
                minimum_documents: 0,
                minimum_sections: 0,
                require_primary_sources: true,
                minimum_corroboration: 2,
            })
            .fingerprint(crate::RetrievalModelFingerprint::new(
                "model:v1".to_owned(),
            )?)
            .authorization(crate::RetrievalPolicySnapshot::global_default())
            .build()?)
    }

    #[test]
    fn deterministic_id_stable_across_versions() -> Result<(), Box<dyn std::error::Error>> {
        // Live, degraded, and storage-canonicalized traces must hash
        // identically: the identity mix is version-free, so a degraded
        // search's reported trace id resolves against the durable trace.
        let plan = fixture_plan("what is the latest guidance", SearchIntent::FactualLocal)?;
        let trace = SearchTrace::from_plan(
            &plan,
            vec!["web".to_string()],
            &[],
            vec![],
            None,
            vec![],
            SearchStopReason::NoEvidence,
        )?;
        // A degraded search reports the degraded trace's id; the durable
        // trace (storage-canonicalized) must hash to the same id.
        let degraded = trace.with_degradation(crate::SearchDegradation {
            capability: "vector lane".to_string(),
            reason: "text/layout retrieval".to_string(),
        });
        let degraded_id = degraded.deterministic_id();

        let mut durable = degraded;
        durable.canonicalize_score_provenance()?;
        assert_eq!(
            durable.deterministic_id(),
            degraded_id,
            "degraded trace id must survive canonicalization"
        );
        Ok(())
    }

    #[test]
    fn deterministic_id_changes_with_query() -> Result<(), Box<dyn std::error::Error>> {
        let plan = fixture_plan("query one", SearchIntent::FactualLocal)?;
        let trace = SearchTrace::from_plan(
            &plan,
            vec![],
            &[],
            vec![],
            None,
            vec![],
            SearchStopReason::NoEvidence,
        )?;
        let base_id = trace.deterministic_id();

        let mut changed = trace.clone();
        changed.original_query = "query two".to_string();
        assert_ne!(changed.deterministic_id(), base_id);
        Ok(())
    }
}
