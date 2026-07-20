use crate::{
    SearchTrace, SearchTraceCandidate, SearchTraceDiversity, SearchTraceId, SearchTraceLane,
    SearchTraceRerank, SearchTraceRewrite,
};

impl SearchTrace {
    pub fn deterministic_id(&self) -> SearchTraceId {
        let mut hash = 0xcbf29ce484222325u64;
        let identity_v2 = self.identity_version >= 2;
        let identity_v3 = self.identity_version >= 3;
        let identity_v4 = self.identity_version >= 4;
        let identity_v5 = self.identity_version >= 5;
        if self.identity_version != 0 {
            mix_hash(
                &mut hash,
                format!("maestria-search-trace:v{}", self.identity_version).as_bytes(),
            );
        }
        mix_hash(&mut hash, &self.query_id.value().to_le_bytes());
        mix_hash(&mut hash, self.original_query.as_bytes());
        if identity_v5 {
            mix_hash(&mut hash, format!("{:?}", self.original_intent).as_bytes());
            mix_hash(
                &mut hash,
                format!("{:?}", self.unavailable_capability).as_bytes(),
            );
            mix_hash(&mut hash, format!("{:?}", self.route_decision).as_bytes());
        }
        mix_hash(&mut hash, format!("{:?}", self.intent).as_bytes());
        mix_hash(&mut hash, format!("{:?}", self.scope).as_bytes());
        mix_hash(&mut hash, format!("{:?}", self.freshness).as_bytes());
        mix_hash(&mut hash, format!("{:?}", self.modalities).as_bytes());
        if identity_v4 {
            mix_hash(&mut hash, format!("{:?}", self.degradation).as_bytes());
        }
        mix_hash(&mut hash, format!("{:?}", self.stages).as_bytes());
        mix_hash(
            &mut hash,
            format!("{:?}", self.evidence_requirements).as_bytes(),
        );
        mix_hash(&mut hash, &self.corpus_snapshot.value().to_le_bytes());
        mix_hash(&mut hash, &self.index_generation.value().to_le_bytes());
        mix_hash(&mut hash, self.fingerprint.as_str().as_bytes());
        for retriever in &self.retrievers {
            mix_hash(&mut hash, retriever.as_bytes());
        }
        mix_hash(
            &mut hash,
            format!("{:?}", self.policy_fingerprint).as_bytes(),
        );
        mix_hash(
            &mut hash,
            &u64::from(self.budgets.max_tokens()).to_le_bytes(),
        );
        mix_hash(
            &mut hash,
            &u64::from(self.budgets.max_latency_ms()).to_le_bytes(),
        );
        if identity_v2 {
            mix_hash(
                &mut hash,
                &u64::from(self.budgets.max_queries()).to_le_bytes(),
            );
            mix_hash(
                &mut hash,
                &u64::from(self.budgets.max_stages()).to_le_bytes(),
            );
            mix_hash(
                &mut hash,
                &u64::from(self.budgets.max_web_requests()).to_le_bytes(),
            );
            mix_hash(&mut hash, &self.budgets.max_bytes_read().to_le_bytes());
            mix_hash(
                &mut hash,
                &u64::from(self.budgets.max_concurrency()).to_le_bytes(),
            );
        }
        mix_hash(
            &mut hash,
            &u64::from(self.stop_conditions.max_results).to_le_bytes(),
        );
        mix_hash(
            &mut hash,
            &u64::from(self.stop_conditions.min_score_threshold).to_le_bytes(),
        );
        mix_trace_candidates(&mut hash, &self.raw_candidates);
        mix_hash(&mut hash, format!("{:?}", self.fusion).as_bytes());
        if identity_v2 {
            mix_trace_rewrites(&mut hash, &self.rewrites);
        }
        mix_hash(&mut hash, format!("{:?}", self.filters).as_bytes());
        mix_hash(&mut hash, format!("{:?}", self.expansions).as_bytes());
        mix_hash(&mut hash, format!("{:?}", self.missing_evidence).as_bytes());
        for conflict in &self.conflicts {
            mix_hash(&mut hash, &conflict.value().to_le_bytes());
        }
        mix_hash(&mut hash, format!("{:?}", self.stop_reason).as_bytes());
        mix_trace_lanes(&mut hash, &self.lanes, identity_v3);
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

fn mix_trace_candidates(hash: &mut u64, candidates: &[SearchTraceCandidate]) {
    for candidate in candidates {
        mix_hash(hash, &candidate.evidence_id.value().to_le_bytes());
        mix_hash(hash, &candidate.artifact_version.value().to_le_bytes());
        mix_hash(hash, format!("{:?}", candidate.source_span).as_bytes());
        mix_hash(hash, &u64::from(candidate.rank).to_le_bytes());
        mix_hash(hash, &u64::from(candidate.scores.bm25).to_le_bytes());
        mix_hash(
            hash,
            &u64::from(candidate.scores.semantic_similarity).to_le_bytes(),
        );
        mix_hash(hash, format!("{:?}", candidate.trust).as_bytes());
        mix_hash(hash, format!("{:?}", candidate.freshness).as_bytes());
        mix_hash(
            hash,
            format!("{:?}", candidate.duplicate_cluster).as_bytes(),
        );
        mix_hash(hash, format!("{:?}", candidate.reasons).as_bytes());
        mix_hash(hash, format!("{:?}", candidate.coverage_keys).as_bytes());
    }
}

fn mix_trace_rewrites(hash: &mut u64, rewrites: &[SearchTraceRewrite]) {
    for rewrite in rewrites {
        mix_hash(hash, rewrite.query.as_bytes());
        mix_hash(hash, format!("{:?}", rewrite.origin).as_bytes());
        mix_hash(hash, format!("{:?}", rewrite.stage).as_bytes());
        mix_hash(
            hash,
            &u64::from(rewrite.accounting.token_estimate).to_le_bytes(),
        );
        mix_hash(
            hash,
            &u64::from(rewrite.accounting.latency_budget_units).to_le_bytes(),
        );
        mix_hash(hash, &[u8::from(rewrite.accounting.is_proposal)]);
        mix_hash(hash, format!("{:?}", rewrite.missing_slot).as_bytes());
    }
}

fn mix_trace_lanes(hash: &mut u64, lanes: &[SearchTraceLane], include_query: bool) {
    for lane in lanes {
        mix_hash(hash, lane.retriever_id.as_bytes());
        if include_query {
            mix_hash(hash, lane.query.as_bytes());
        }
        mix_hash(hash, format!("{:?}", lane.status).as_bytes());
        for candidate in &lane.candidates {
            mix_hash(hash, &candidate.evidence_id.value().to_le_bytes());
            mix_hash(hash, &candidate.artifact_version.value().to_le_bytes());
            mix_hash(hash, format!("{:?}", candidate.source_span).as_bytes());
            mix_hash(hash, &u64::from(candidate.lane_rank).to_le_bytes());
            mix_hash(
                hash,
                format!("{:?}", candidate.duplicate_cluster).as_bytes(),
            );
            mix_hash(hash, &u64::from(candidate.scores.bm25).to_le_bytes());
            mix_hash(
                hash,
                &u64::from(candidate.scores.semantic_similarity).to_le_bytes(),
            );
            mix_hash(hash, format!("{:?}", candidate.reasons).as_bytes());
        }
    }
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
        mix_hash(hash, format!("{:?}", candidate.new_rank).as_bytes());
        mix_hash(hash, format!("{:?}", candidate.status).as_bytes());
        mix_hash(hash, format!("{:?}", candidate.relevance_score).as_bytes());
        mix_hash(hash, format!("{:?}", candidate.constraint_score).as_bytes());
        for constraint in &candidate.constraint_scores {
            mix_hash(hash, constraint.name.as_bytes());
            mix_hash(hash, &u64::from(constraint.score).to_le_bytes());
        }
    }
}

fn mix_diversity(hash: &mut u64, diversity: &SearchTraceDiversity) {
    let mut mix = |bytes: &[u8]| {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    mix(&(diversity.distinct_sources as u64).to_le_bytes());
    mix(&(diversity.distinct_documents as u64).to_le_bytes());
    mix(&(diversity.distinct_sections as u64).to_le_bytes());
    for claim in &diversity.required_claims {
        mix(claim.as_bytes());
    }
    for subquestion in &diversity.required_subquestions {
        mix(subquestion.as_bytes());
    }
    for key in &diversity.covered_keys {
        mix(key.as_bytes());
    }
    mix(format!("{:?}", diversity.stop_reason).as_bytes());
    for candidate in &diversity.candidates {
        mix(&candidate.candidate_id.value().to_le_bytes());
        mix(&(candidate.original_rank as u64).to_le_bytes());
        mix(format!("{:?}", candidate.selected_rank).as_bytes());
        mix(format!("{:?}", candidate.duplicate_cluster).as_bytes());
        mix(&u64::from(candidate.marginal_coverage).to_le_bytes());
        for key in &candidate.coverage_keys {
            mix(key.as_bytes());
        }
    }
}
