use sillage_domain::{EvidenceCandidate, SearchLaneStatus};
use sillage_ports::SearchQuery;

use crate::traits::RankFusion;
use crate::types::{
    CandidateBatch, FusedCandidate, FusionOutput, HYBRID_LEXICAL_HEAD_POLICY_ID, RetrievalResult,
};

/// Keeps the first eligible lexical result ahead of the fused tail for Hybrid
/// searches. The wrapped fusion still determines every other candidate's rank.
pub struct HybridLexicalHead<F> {
    inner: F,
}

impl<F> HybridLexicalHead<F> {
    pub fn new(inner: F) -> Self {
        Self { inner }
    }
}

impl<F: RankFusion> RankFusion for HybridLexicalHead<F> {
    fn fuse(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
    ) -> RetrievalResult<Vec<FusedCandidate>> {
        Ok(self
            .fuse_with_protected_head(query, batches, None)?
            .candidates)
    }

    fn append_trace_identity(&self, identity: &mut String) {
        identity.push_str(HYBRID_LEXICAL_HEAD_POLICY_ID);
        identity.push('+');
        self.inner.append_trace_identity(identity);
    }

    fn fuse_with_protected_head(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
        protected_head: Option<&EvidenceCandidate>,
    ) -> RetrievalResult<FusionOutput> {
        if !batches.iter().any(|batch| batch.descriptor.is_dense()) {
            return Ok(FusionOutput {
                candidates: self.inner.fuse(query, batches)?,
                protected_lexical_head: None,
            });
        }
        let (mut output, head) =
            self.inner
                .fuse_with_lexical_baseline_head(query, batches, protected_head)?;
        let Some(head) = head else {
            return Ok(output);
        };
        if output.protected_lexical_head == Some(head.evidence_id())
            && output.candidates.first().is_some_and(|fused| {
                same_candidate_metadata(&fused.candidate, &head)
                    && has_head_score_provenance(&fused.candidate, &head)
            })
        {
            return Ok(output);
        }
        let identity = normalized_identity(&head, batches);
        let position = output
            .candidates
            .iter()
            .position(|fused| matches_identity(&fused.candidate, &head, identity));
        if let Some(position) = position {
            let fused_head = &mut output.candidates[position].candidate;
            if !same_candidate_metadata(fused_head, &head)
                || !has_head_score_provenance(fused_head, &head)
            {
                *fused_head = super::merge_lane_scores(&head, fused_head)?;
            }
            output.candidates[..=position].rotate_right(1);
        } else {
            output.candidates.push(FusedCandidate {
                candidate: head.clone(),
            });
            output.candidates.rotate_right(1);
        }
        output.protected_lexical_head = Some(head.evidence_id());
        Ok(output)
    }
}

fn normalized_identity(
    head: &EvidenceCandidate,
    batches: &[CandidateBatch],
) -> super::CandidateIdentity {
    let mut cluster = head.duplicate_cluster();
    for candidate in batches
        .iter()
        .filter(|batch| matches!(batch.status, SearchLaneStatus::Succeeded))
        .flat_map(|batch| &batch.candidates)
        .filter(|candidate| candidate.evidence_id() == head.evidence_id())
    {
        if let Some(candidate_cluster) = candidate.duplicate_cluster() {
            cluster = Some(cluster.map_or(candidate_cluster, |existing| {
                existing.min(candidate_cluster)
            }));
        }
    }
    super::candidate_identity(head, cluster.as_ref())
}

fn matches_identity(
    candidate: &EvidenceCandidate,
    head: &EvidenceCandidate,
    identity: super::CandidateIdentity,
) -> bool {
    match identity {
        super::CandidateIdentity::Exact(evidence_id) => candidate.evidence_id() == evidence_id,
        super::CandidateIdentity::Cluster(cluster) => {
            candidate.evidence_id() == head.evidence_id()
                || candidate.duplicate_cluster() == Some(cluster)
        }
    }
}

fn same_candidate_metadata(left: &EvidenceCandidate, right: &EvidenceCandidate) -> bool {
    left.evidence_id() == right.evidence_id()
        && left.artifact_version() == right.artifact_version()
        && left.source_span() == right.source_span()
        && left.trust() == right.trust()
        && left.freshness() == right.freshness()
        && left.duplicate_cluster() == right.duplicate_cluster()
        && left.reasons() == right.reasons()
        && left.coverage_keys() == right.coverage_keys()
}

fn has_head_score_provenance(candidate: &EvidenceCandidate, head: &EvidenceCandidate) -> bool {
    head.scores()
        .lanes()
        .iter()
        .all(|lane| candidate.scores().lane(&lane.score_kind) == Some(lane))
}
