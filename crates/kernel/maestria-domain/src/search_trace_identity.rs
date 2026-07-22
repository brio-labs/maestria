use crate::{
    SearchTrace, SearchTraceCandidate, SearchTraceDiversity, SearchTraceId, SearchTraceLane,
    SearchTraceRerank, SearchTraceRewrite,
};

impl SearchTrace {
    pub fn deterministic_id(&self) -> SearchTraceId {
        let mut hash = 0xcbf29ce484222325u64;
        mix_trace_header(&mut hash, self);
        mix_trace_budgets(&mut hash, self);
        mix_trace_stop_conditions(&mut hash, self);
        mix_trace_candidates(&mut hash, &self.raw_candidates, self.identity_version >= 6);
        mix_trace_post_candidates(&mut hash, self);
        mix_trace_lanes(
            &mut hash,
            &self.lanes,
            self.identity_version >= 3,
            self.identity_version >= 6,
        );
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
    let identity_v4 = trace.identity_version >= 4;
    let identity_v5 = trace.identity_version >= 5;
    if trace.identity_version != 0 {
        mix_hash(
            hash,
            format!("maestria-search-trace:v{}", trace.identity_version).as_bytes(),
        );
    }
    mix_hash(hash, &trace.query_id.value().to_le_bytes());
    mix_hash(hash, trace.original_query.as_bytes());
    if identity_v5 {
        mix_debug(hash, &trace.original_intent);
        mix_debug(hash, &trace.unavailable_capability);
        mix_debug(hash, &trace.route_decision);
    }
    mix_debug(hash, &trace.intent);
    mix_debug(hash, &trace.scope);
    mix_debug(hash, &trace.freshness);
    mix_debug(hash, &trace.modalities);
    if identity_v4 {
        mix_debug(hash, &trace.degradation);
    }
    mix_debug(hash, &trace.stages);
    mix_debug(hash, &trace.evidence_requirements);
    mix_hash(hash, &trace.corpus_snapshot.value().to_le_bytes());
    mix_hash(hash, &trace.index_generation.value().to_le_bytes());
    mix_hash(hash, trace.fingerprint.as_str().as_bytes());
    for retriever in &trace.retrievers {
        mix_hash(hash, retriever.as_bytes());
    }
    mix_debug(hash, &trace.policy_fingerprint);
}

fn mix_trace_budgets(hash: &mut u64, trace: &SearchTrace) {
    let identity_v2 = trace.identity_version >= 2;
    mix_hash(hash, &u64::from(trace.budgets.max_tokens()).to_le_bytes());
    mix_hash(
        hash,
        &u64::from(trace.budgets.max_latency_ms()).to_le_bytes(),
    );
    if identity_v2 {
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
    }
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
    let identity_v2 = trace.identity_version >= 2;
    mix_debug(hash, &trace.fusion);
    if identity_v2 {
        mix_trace_rewrites(hash, &trace.rewrites);
    }
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
        mix_hash(hash, &candidate.evidence_id.value().to_le_bytes());
        mix_hash(hash, &candidate.artifact_version.value().to_le_bytes());
        mix_debug(hash, &candidate.source_span);
        mix_hash(hash, &u64::from(candidate.rank).to_le_bytes());
        mix_scores(hash, &candidate.scores, complete_score_provenance);
        mix_debug(hash, &candidate.trust);
        mix_debug(hash, &candidate.freshness);
        mix_debug(hash, &candidate.duplicate_cluster);
        mix_debug(hash, &candidate.reasons);
        mix_debug(hash, &candidate.coverage_keys);
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
        mix_hash(hash, &[u8::from(rewrite.accounting.is_proposal)]);
        mix_debug(hash, &rewrite.missing_slot);
    }
}

fn mix_trace_lanes(
    hash: &mut u64,
    lanes: &[SearchTraceLane],
    include_query: bool,
    complete_score_provenance: bool,
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
        for candidate in &lane.candidates {
            mix_hash(hash, &candidate.evidence_id.value().to_le_bytes());
            mix_hash(hash, &candidate.artifact_version.value().to_le_bytes());
            mix_debug(hash, &candidate.source_span);
            mix_hash(hash, &u64::from(candidate.lane_rank).to_le_bytes());
            mix_debug(hash, &candidate.duplicate_cluster);
            mix_scores(hash, &candidate.scores, complete_score_provenance);
            mix_debug(hash, &candidate.reasons);
        }
    }
}

fn mix_scores(hash: &mut u64, scores: &crate::RetrievalScoreSet, complete_score_provenance: bool) {
    if complete_score_provenance {
        mix_hash(hash, &u64::from(scores.schema_version).to_le_bytes());
        for score in &scores.lanes {
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
    for score in &scores.lanes {
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
        mix_debug(hash, &candidate.new_rank);
        mix_debug(hash, &candidate.status);
        mix_debug(hash, &candidate.relevance_score);
        mix_debug(hash, &candidate.constraint_score);
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
        mix_debug(hash, &candidate.selected_rank);
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
    use crate::{CorpusScope, FreshnessRequirement, SearchIntent, SearchStage, SearchStopReason};

    fn old_mix_debug<T: std::fmt::Debug>(hash: &mut u64, value: &T) {
        mix_hash(hash, format!("{:?}", value).as_bytes());
    }

    #[test]
    fn mix_debug_matches_format_for_enum() {
        let value = SearchIntent::FactualLocal;
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        old_mix_debug(&mut h1, &value);
        mix_debug(&mut h2, &value);
        assert_eq!(h1, h2, "mix_debug diverged for SearchIntent");
    }

    #[test]
    fn mix_debug_matches_format_for_option_some() {
        let value = Some(SearchStage::Reranking);
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        old_mix_debug(&mut h1, &value);
        mix_debug(&mut h2, &value);
        assert_eq!(h1, h2, "mix_debug diverged for Option<SearchStage>::Some");
    }

    #[test]
    fn mix_debug_matches_format_for_option_none() {
        let value: Option<SearchStage> = None;
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        old_mix_debug(&mut h1, &value);
        mix_debug(&mut h2, &value);
        assert_eq!(h1, h2, "mix_debug diverged for Option<SearchStage>::None");
    }

    #[test]
    fn mix_debug_matches_format_for_string() {
        let value = String::from("hello world");
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        old_mix_debug(&mut h1, &value);
        mix_debug(&mut h2, &value);
        assert_eq!(h1, h2, "mix_debug diverged for String");
    }

    #[test]
    fn mix_debug_matches_format_for_enum_with_data() {
        let value = FreshnessRequirement::MaximumAgeDays(7);
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        old_mix_debug(&mut h1, &value);
        mix_debug(&mut h2, &value);
        assert_eq!(
            h1, h2,
            "mix_debug diverged for FreshnessRequirement::MaximumAgeDays"
        );
    }

    #[test]
    fn mix_debug_matches_format_for_complex_enum() {
        let value = SearchStopReason::EvidenceComplete;
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        old_mix_debug(&mut h1, &value);
        mix_debug(&mut h2, &value);
        assert_eq!(h1, h2, "mix_debug diverged for SearchStopReason");
    }

    #[test]
    fn mix_debug_matches_format_for_corpus_scope() {
        let value = CorpusScope::Global;
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        old_mix_debug(&mut h1, &value);
        mix_debug(&mut h2, &value);
        assert_eq!(h1, h2, "mix_debug diverged for CorpusScope");
    }

    #[test]
    fn mix_debug_matches_format_for_vec_of_enum() {
        let value = vec![SearchIntent::ExactLookup, SearchIntent::SemanticDiscovery];
        let mut h1 = 0u64;
        let mut h2 = 0u64;
        old_mix_debug(&mut h1, &value);
        mix_debug(&mut h2, &value);
        assert_eq!(h1, h2, "mix_debug diverged for Vec<SearchIntent>");
    }
}
