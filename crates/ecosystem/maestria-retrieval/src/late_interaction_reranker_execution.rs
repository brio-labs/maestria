use std::collections::BTreeMap;

use maestria_domain::{
    ContentHash, LateInteractionProvenance, LateInteractionTokenContribution, RepresentationName,
    RerankPosition, RetrievalLaneScore, RetrievalRawRank, RetrievalScoreFingerprint,
    RetrievalScoreKind, RetrievalScoreScale, SearchTraceRerankCandidate,
};
use maestria_governance::scan_secrets;
use maestria_ports::{
    LATE_INTERACTION_SCORE_DENOMINATOR, LATE_INTERACTION_SCORE_SCALE_V1,
    LateInteractionScoringResult, MULTIVECTOR_TEXT_V1, MultiVectorDocument,
    MultiVectorDocumentRequest, MultiVectorQuery, MultiVectorQueryRequest, ProviderCallControl,
};

use crate::cancellation::SearchCallControl;
use crate::traits::CandidateReranker;
use crate::types::{RankedCandidate, RerankRequest, RerankResult, RetrievalError};

use super::{
    LateInteractionReranker, MAX_QUERY_BYTES, MAX_RESPONSE_BYTES, ScoreCandidatesRequest,
    ScoredLateCandidate,
};

impl LateInteractionReranker {
    fn query_hash(query: &str) -> Result<ContentHash, &'static str> {
        ContentHash::new(maestria_domain::content_hash(query.as_bytes()))
            .map_err(|_| "late_interaction:source_invalid")
    }
    fn provenance(
        &self,
        query: &MultiVectorQuery,
        document: &MultiVectorDocument,
        score: &LateInteractionScoringResult,
        raw_rank: u32,
    ) -> Result<LateInteractionProvenance, &'static str> {
        if score.denominator != LATE_INTERACTION_SCORE_DENOMINATOR
            || score.scale != LATE_INTERACTION_SCORE_SCALE_V1
            || score.retained_contribution_count != score.contributions.len() as u32
            || score
                .retained_contribution_count
                .checked_add(score.omitted_contribution_count)
                != Some(score.total_query_tokens)
        {
            return Err("late_interaction:invalid_provenance");
        }
        let contributions = score
            .contributions
            .iter()
            .map(|contribution| LateInteractionTokenContribution {
                query_position: contribution.query_position,
                document_position: contribution.document_position,
                similarity_micros: contribution.similarity_micros,
            })
            .collect();
        let mut components = BTreeMap::new();
        components.insert(
            "aggregation".to_string(),
            format!("{:?}", self.parts.identity.fingerprint.aggregation),
        );
        components.insert(
            "normalization".to_string(),
            self.parts
                .identity
                .fingerprint
                .normalization_version
                .clone(),
        );
        components.insert(
            "provider".to_string(),
            self.parts.identity.fingerprint.base.provider.to_string(),
        );
        components.insert(
            "model".to_string(),
            self.parts.identity.fingerprint.base.model.to_string(),
        );
        components.insert(
            "representation".to_string(),
            self.parts.identity.representation.as_str().to_string(),
        );
        components.insert(
            "generation_id".to_string(),
            self.parts.identity.generation_id.value().to_string(),
        );
        components.insert(
            "corpus_snapshot".to_string(),
            self.parts.identity.corpus_snapshot.value().to_string(),
        );
        components.insert(
            "identity_digest".to_string(),
            self.parts
                .identity
                .digest()
                .map_err(|_| "late_interaction:invalid_provenance")?
                .as_str()
                .to_string(),
        );
        let lane_score = RetrievalLaneScore::new(
            RetrievalScoreKind::LateInteraction,
            score.aggregate_micros,
            RetrievalRawRank::ranked(raw_rank),
            RetrievalScoreScale::fixed_point(
                LATE_INTERACTION_SCORE_SCALE_V1,
                LATE_INTERACTION_SCORE_DENOMINATOR as u32,
            ),
            RepresentationName::new(MULTIVECTOR_TEXT_V1),
            RetrievalScoreFingerprint {
                identity: self.fingerprint.clone(),
                components,
            },
        );
        let provenance = LateInteractionProvenance {
            score: lane_score,
            generation_id: self.parts.identity.generation_id,
            corpus_snapshot: self.parts.identity.corpus_snapshot,
            namespace: self.parts.identity.representation.as_str().to_string(),
            source_snapshot_hash: document.source().source_snapshot_hash.clone(),
            source_representation_hash: document.source().source_representation_hash.clone(),
            query_hash: query.input_hash().clone(),
            aggregation: self.parts.identity.fingerprint.aggregation,
            query_token_count: score.total_query_tokens,
            document_token_count: document.original_token_count(),
            query_truncated: query.truncated(),
            document_truncated: document.truncated(),
            contributions,
            omitted_contribution_count: score.omitted_contribution_count,
            omitted_contribution_sum_micros: score.omitted_contribution_sum_micros,
        };
        provenance
            .validate()
            .map_err(|_| "late_interaction:invalid_provenance")?;
        Ok(provenance)
    }

    fn score_candidates(
        &self,
        request: ScoreCandidatesRequest<'_>,
    ) -> Result<Vec<ScoredLateCandidate>, &'static str> {
        let ScoreCandidatesRequest {
            query,
            candidates,
            eligible_positions,
            score_limit,
            authorization,
            source_filter,
            control,
        } = request;
        let mut scored = Vec::new();
        for &position in eligible_positions {
            if scored.len() >= score_limit {
                break;
            }
            let candidate = &candidates[position];
            let (text, source) =
                self.authorized_source(candidate, authorization, source_filter, control)?;
            let input_hash = Self::query_hash(&text)?;
            let document_request = MultiVectorDocumentRequest {
                identity: self.parts.identity.clone(),
                source,
                input_hash,
                max_vectors: self.parts.identity.fingerprint.document_policy.max_vectors,
            };
            let document = self
                .parts
                .provider
                .encode_document(&text, &document_request, control)
                .map_err(|error| fallback_code(&error, control))?;
            let score = self
                .parts
                .scorer
                .score(query, &document, control)
                .map_err(|error| fallback_code(&error, control))?;
            scored.push(ScoredLateCandidate {
                position,
                score,
                document,
            });
        }
        Ok(scored)
    }
    fn reorder_scored(
        &self,
        query: &MultiVectorQuery,
        candidates: &[RankedCandidate],
        mut scored: Vec<ScoredLateCandidate>,
    ) -> Result<(Vec<RankedCandidate>, Vec<SearchTraceRerankCandidate>), &'static str> {
        let target_positions: Vec<usize> = scored.iter().map(|item| item.position).collect();
        scored.sort_by(|left, right| {
            right
                .score
                .aggregate_micros
                .cmp(&left.score.aggregate_micros)
                .then_with(|| {
                    candidates[left.position]
                        .rank
                        .cmp(&candidates[right.position].rank)
                })
                .then_with(|| {
                    candidates[left.position]
                        .candidate
                        .evidence_id()
                        .cmp(&candidates[right.position].candidate.evidence_id())
                })
        });
        let slots = scored.len().min(self.limits.output_cap);
        let mut ordered_sources: Vec<usize> = scored
            .iter()
            .take(slots)
            .map(|item| item.position)
            .collect();
        let mut selected = vec![false; self.limits.input_cap.min(candidates.len())];
        for &position in &ordered_sources {
            selected[position] = true;
        }
        let remaining_sources: Vec<usize> = target_positions
            .iter()
            .copied()
            .filter(|position| !selected[*position])
            .collect();
        ordered_sources.extend(remaining_sources);
        let mut output = candidates.to_vec();
        let mut trace = self.all_trace(candidates, RerankPosition::SkippedNotApplicable);
        for (slot, source_position) in ordered_sources.iter().copied().enumerate() {
            let target_position = target_positions[slot];
            output[target_position] = candidates[source_position].clone();
            if slot < slots {
                let raw_rank = candidates[source_position]
                    .rank
                    .checked_add(1)
                    .and_then(|rank| u32::try_from(rank).ok())
                    .ok_or("late_interaction:invalid_provenance")?;
                let provenance =
                    self.provenance(query, &scored[slot].document, &scored[slot].score, raw_rank)?;
                trace[source_position].position = RerankPosition::Reranked(target_position);
                trace[source_position].late_interaction = Some(provenance);
            } else {
                trace[source_position].position = RerankPosition::SkippedCap;
            }
        }
        for (rank, candidate) in output.iter_mut().enumerate() {
            candidate.rank = rank;
        }
        Ok((output, trace))
    }
}
impl CandidateReranker for LateInteractionReranker {
    fn rerank(&self, request: RerankRequest) -> Result<RerankResult, RetrievalError> {
        let RerankRequest {
            plan,
            candidates,
            max_latency_ms,
            authorization,
            source_filter,
            cancellation,
        } = request;
        if candidates.is_empty() {
            return Ok(self.result(Vec::new(), Vec::new()));
        }
        if !Self::applicable_intent(plan.intent())
            || !self.intent_is_allowed(plan.intent())
            || max_latency_ms == 0
            || self.limits.input_cap == 0
            || self.limits.score_cap == 0
        {
            return Ok(self.result(
                candidates.clone(),
                self.all_trace(&candidates, RerankPosition::SkippedNotApplicable),
            ));
        }
        if !scan_secrets(plan.original_query()).is_clean()
            || plan.original_query().len() > MAX_QUERY_BYTES
        {
            return Ok(self.fallback(candidates, "late_interaction:privacy_rejected"));
        }
        let eligible_positions: Vec<usize> = candidates
            .iter()
            .enumerate()
            .take(self.limits.input_cap)
            .filter_map(|(position, candidate)| {
                (!Self::protected_candidate(&candidate.candidate)).then_some(position)
            })
            .collect();
        if eligible_positions.is_empty() {
            return Ok(self.result(
                candidates.clone(),
                self.all_trace(&candidates, RerankPosition::SkippedNotApplicable),
            ));
        }
        let control = cancellation.control(
            crate::MonotonicInstant::now(),
            max_latency_ms,
            MAX_RESPONSE_BYTES,
        );
        if control.check().is_err() {
            return Ok(self.fallback(candidates, "late_interaction:cancelled"));
        }
        let query_hash = Self::query_hash(plan.original_query())
            .map_err(|code| RetrievalError::Internal(code.into()))?;
        let query_request = MultiVectorQueryRequest {
            identity: self.parts.identity.clone(),
            input_hash: query_hash.clone(),
            max_vectors: self.parts.identity.fingerprint.query_policy.max_vectors,
        };
        let query =
            match self
                .parts
                .provider
                .encode_query(plan.original_query(), &query_request, &control)
            {
                Ok(query) => query,
                Err(error) => return Ok(self.fallback(candidates, fallback_code(&error, &control))),
            };
        let score_limit = self.limits.input_cap.min(self.limits.score_cap);
        let scored = match self.score_candidates(ScoreCandidatesRequest {
            query: &query,
            candidates: &candidates,
            eligible_positions: &eligible_positions,
            score_limit,
            authorization: &authorization,
            source_filter: source_filter.as_ref(),
            control: &control,
        }) {
            Ok(scored) => scored,
            Err(code) => return Ok(self.fallback(candidates, code)),
        };
        if scored.is_empty() {
            return Ok(self.result(
                candidates.clone(),
                self.all_trace(&candidates, RerankPosition::SkippedNotApplicable),
            ));
        }
        let (output, trace) = match self.reorder_scored(&query, &candidates, scored) {
            Ok(result) => result,
            Err(code) => return Ok(self.fallback(candidates, code)),
        };
        Ok(self.result(output, trace))
    }
}

fn fallback_code(error: &maestria_ports::PortError, control: &SearchCallControl) -> &'static str {
    if control.is_cancelled() {
        "late_interaction:cancelled"
    } else if control.remaining_ms() == 0 {
        "late_interaction:timeout"
    } else if error.is_invalid_input() {
        "late_interaction:invalid_representation"
    } else if error.is_identity() {
        "late_interaction:identity_mismatch"
    } else {
        "late_interaction:provider_unavailable"
    }
}
