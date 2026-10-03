use sillage_domain::{EvidenceCandidate, FreshnessStatus, RetrievalScoreKind};
use sillage_ports::SearchQuery;

use crate::traits::RankFusion;
use crate::types::{FusedCandidate, FusionOutput, RetrievalError, RetrievalResult};

use super::{
    FixedKRrf, RRF_SCALE, candidate_identity, collect_evidence_clusters,
    collect_hybrid_evidence_clusters, finalize_fusion, record_candidate,
};

fn next_rrf_contribution(k: u64, rank: &mut usize) -> RetrievalResult<u64> {
    let next_rank = rank
        .checked_add(1)
        .ok_or_else(|| RetrievalError::Internal("RRF lane rank overflow".to_string()))?;
    let rank_u64 = u64::try_from(next_rank).map_err(|_| {
        RetrievalError::Internal("RRF lane rank does not fit denominator".to_string())
    })?;
    let denominator = k
        .checked_add(rank_u64)
        .ok_or_else(|| RetrievalError::Internal("RRF denominator overflow".to_string()))?;
    *rank = next_rank;
    Ok(RRF_SCALE / denominator)
}

impl FixedKRrf {
    fn fuse_candidates(
        &self,
        batches: &[crate::types::CandidateBatch],
        protected_head: Option<&EvidenceCandidate>,
        lexical_only: bool,
    ) -> RetrievalResult<Vec<FusedCandidate>> {
        let k = u64::try_from(self.k).map_err(|_| {
            RetrievalError::Internal("RRF k does not fit the fixed-point denominator".to_string())
        })?;
        if k == 0 {
            return Err(RetrievalError::Internal(
                "RRF k must be greater than zero".to_string(),
            ));
        }
        let evidence_clusters = collect_evidence_clusters(batches, lexical_only);
        let mut scores = std::collections::BTreeMap::<super::CandidateIdentity, u64>::new();
        let mut best_candidates =
            std::collections::BTreeMap::<super::CandidateIdentity, EvidenceCandidate>::new();
        let mut seen = Vec::new();
        for batch in batches {
            if (lexical_only && batch.descriptor.is_dense())
                || !matches!(batch.status, sillage_domain::SearchLaneStatus::Succeeded)
            {
                continue;
            }
            seen.clear();
            let mut compact_rank = 0usize;
            for candidate in &batch.candidates {
                let identity =
                    candidate_identity(candidate, evidence_clusters.get(&candidate.evidence_id()));
                if seen.contains(&identity) {
                    if protected_head.is_some_and(|head| head == candidate) {
                        record_candidate(
                            &mut best_candidates,
                            identity,
                            candidate,
                            protected_head,
                        )?;
                    }
                    continue;
                }
                seen.push(identity);
                let contribution = next_rrf_contribution(k, &mut compact_rank)?;
                scores
                    .entry(identity)
                    .and_modify(|score| *score = score.saturating_add(contribution))
                    .or_insert(contribution);
                record_candidate(&mut best_candidates, identity, candidate, protected_head)?;
            }
        }
        let mut candidates = finalize_fusion(scores, best_candidates);
        if let Some(head) = protected_head
            && let Some(position) = candidates
                .iter()
                .position(|candidate| candidate.candidate.evidence_id() == head.evidence_id())
        {
            candidates[..=position].rotate_right(1);
        }
        Ok(candidates)
    }
    pub(crate) fn lexical_baseline_head(
        &self,
        batches: &[crate::types::CandidateBatch],
    ) -> RetrievalResult<Option<EvidenceCandidate>> {
        Ok(self
            .fuse_candidates(batches, None, true)?
            .into_iter()
            .map(|fused| fused.candidate)
            .find(is_eligible_lexical_candidate))
    }

    fn fuse_candidates_with_lexical_baseline(
        &self,
        batches: &[crate::types::CandidateBatch],
    ) -> RetrievalResult<(Vec<FusedCandidate>, Option<EvidenceCandidate>)> {
        let k = u64::try_from(self.k).map_err(|_| {
            RetrievalError::Internal("RRF k does not fit the fixed-point denominator".to_string())
        })?;
        if k == 0 {
            return Err(RetrievalError::Internal(
                "RRF k must be greater than zero".to_string(),
            ));
        }
        let (evidence_clusters, lexical_clusters) = collect_hybrid_evidence_clusters(batches);
        let mut scores = std::collections::BTreeMap::<super::CandidateIdentity, u64>::new();
        let mut lexical_scores = std::collections::BTreeMap::<super::CandidateIdentity, u64>::new();
        let mut best_candidates =
            std::collections::BTreeMap::<super::CandidateIdentity, EvidenceCandidate>::new();
        let mut lexical_candidates =
            std::collections::BTreeMap::<super::CandidateIdentity, EvidenceCandidate>::new();
        let mut seen = Vec::new();
        let mut lexical_seen = Vec::new();
        for batch in batches {
            if !matches!(batch.status, sillage_domain::SearchLaneStatus::Succeeded) {
                continue;
            }
            seen.clear();
            let lexical_lane = !batch.descriptor.is_dense();
            if lexical_lane {
                lexical_seen.clear();
            }
            let mut compact_rank = 0usize;
            let mut lexical_rank = 0usize;
            for candidate in &batch.candidates {
                let identity =
                    candidate_identity(candidate, evidence_clusters.get(&candidate.evidence_id()));
                if !seen.contains(&identity) {
                    seen.push(identity);
                    let contribution = next_rrf_contribution(k, &mut compact_rank)?;
                    scores
                        .entry(identity)
                        .and_modify(|score| *score = score.saturating_add(contribution))
                        .or_insert(contribution);
                    record_candidate(&mut best_candidates, identity, candidate, None)?;
                }
                if lexical_lane {
                    let lexical_identity = candidate_identity(
                        candidate,
                        lexical_clusters.get(&candidate.evidence_id()),
                    );
                    if !lexical_seen.contains(&lexical_identity) {
                        lexical_seen.push(lexical_identity);
                        let contribution = next_rrf_contribution(k, &mut lexical_rank)?;
                        lexical_scores
                            .entry(lexical_identity)
                            .and_modify(|score| *score = score.saturating_add(contribution))
                            .or_insert(contribution);
                        record_candidate(
                            &mut lexical_candidates,
                            lexical_identity,
                            candidate,
                            None,
                        )?;
                    }
                }
            }
        }
        let head = finalize_fusion(lexical_scores, lexical_candidates)
            .into_iter()
            .map(|fused| fused.candidate)
            .find(is_eligible_lexical_candidate);
        if let Some(head) = &head {
            let identity = candidate_identity(head, evidence_clusters.get(&head.evidence_id()));
            record_candidate(&mut best_candidates, identity, head, Some(head))?;
        }
        let mut candidates = finalize_fusion(scores, best_candidates);
        if let Some(head) = &head
            && let Some(position) = candidates
                .iter()
                .position(|candidate| candidate.candidate.evidence_id() == head.evidence_id())
        {
            candidates[..=position].rotate_right(1);
        }
        Ok((candidates, head))
    }
}

fn is_eligible_lexical_candidate(candidate: &EvidenceCandidate) -> bool {
    !matches!(candidate.freshness(), FreshnessStatus::Stale)
        && candidate
            .scores()
            .lane(&RetrievalScoreKind::LexicalBm25)
            .is_some()
}

impl RankFusion for FixedKRrf {
    fn fuse(
        &self,
        _query: &SearchQuery,
        batches: &[crate::types::CandidateBatch],
    ) -> RetrievalResult<Vec<FusedCandidate>> {
        self.fuse_candidates(batches, None, false)
    }

    fn append_trace_identity(&self, identity: &mut String) {
        identity.push_str("fixed-k-rrf-v1:k=");
        use std::fmt::Write as _;
        let _ = write!(identity, "{}", self.k);
    }

    fn fuse_with_protected_head(
        &self,
        _query: &SearchQuery,
        batches: &[crate::types::CandidateBatch],
        protected_head: Option<&EvidenceCandidate>,
    ) -> RetrievalResult<FusionOutput> {
        Ok(FusionOutput {
            candidates: self.fuse_candidates(batches, protected_head, false)?,
            protected_lexical_head: protected_head.map(EvidenceCandidate::evidence_id),
        })
    }

    fn fuse_with_lexical_baseline_head(
        &self,
        query: &SearchQuery,
        batches: &[crate::types::CandidateBatch],
        protected_head: Option<&EvidenceCandidate>,
    ) -> RetrievalResult<(FusionOutput, Option<EvidenceCandidate>)> {
        if let Some(head) = protected_head {
            let output = self.fuse_with_protected_head(query, batches, Some(head))?;
            return Ok((output, Some(head.clone())));
        }
        let (candidates, head) = self.fuse_candidates_with_lexical_baseline(batches)?;
        let protected_lexical_head = head.as_ref().map(EvidenceCandidate::evidence_id);
        Ok((
            FusionOutput {
                candidates,
                protected_lexical_head,
            },
            head,
        ))
    }
}
