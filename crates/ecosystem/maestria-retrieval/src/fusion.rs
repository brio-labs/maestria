use maestria_domain::{EvidenceCandidate, EvidenceCandidateDto, RetrievalScoreKind};
use maestria_ports::SearchQuery;

use crate::traits::RankFusion;
use crate::types::{FusedCandidate, RetrievalError, RetrievalResult};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateIdentity {
    Cluster(maestria_domain::DuplicateClusterId),
    Exact(maestria_domain::EvidenceId),
}

const RRF_SCALE: u64 = 10_000_000;

/// Deterministic rank-only Reciprocal Rank Fusion.
pub struct FixedKRrf {
    pub k: usize,
}

impl FixedKRrf {
    pub fn new(k: usize) -> Self {
        Self { k }
    }
}

impl RankFusion for FixedKRrf {
    fn fuse(
        &self,
        _query: &SearchQuery,
        batches: &[crate::types::CandidateBatch],
    ) -> RetrievalResult<Vec<FusedCandidate>> {
        let k = u64::try_from(self.k).map_err(|_| {
            RetrievalError::Internal("RRF k does not fit the fixed-point denominator".to_string())
        })?;
        if k == 0 {
            return Err(RetrievalError::Internal(
                "RRF k must be greater than zero".to_string(),
            ));
        }
        let evidence_clusters = collect_evidence_clusters(batches);
        let mut scores = std::collections::BTreeMap::<CandidateIdentity, u64>::new();
        let mut best_candidates =
            std::collections::BTreeMap::<CandidateIdentity, EvidenceCandidate>::new();
        let mut seen = std::collections::BTreeSet::new();
        for batch in batches {
            if !matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded) {
                continue;
            }
            seen.clear();
            let mut compact_rank = 0usize;
            for candidate in &batch.candidates {
                let identity =
                    candidate_identity(candidate, evidence_clusters.get(&candidate.evidence_id()));
                if !seen.insert(identity.clone()) {
                    continue;
                }
                let rank = compact_rank.checked_add(1).ok_or_else(|| {
                    RetrievalError::Internal("RRF lane rank overflow".to_string())
                })?;
                let rank = u64::try_from(rank).map_err(|_| {
                    RetrievalError::Internal("RRF lane rank does not fit denominator".to_string())
                })?;
                let denominator = k.checked_add(rank).ok_or_else(|| {
                    RetrievalError::Internal("RRF denominator overflow".to_string())
                })?;
                let contribution = RRF_SCALE / denominator;
                compact_rank += 1;
                scores
                    .entry(identity.clone())
                    .and_modify(|score| *score = score.saturating_add(contribution))
                    .or_insert(contribution);
                record_candidate(&mut best_candidates, &identity, candidate)?;
            }
        }
        Ok(finalize_fusion(scores, best_candidates))
    }
}

fn candidate_identity(
    candidate: &EvidenceCandidate,
    normalized_cluster: Option<&maestria_domain::DuplicateClusterId>,
) -> CandidateIdentity {
    if let Some(cluster_id) = normalized_cluster.or(candidate.duplicate_cluster().as_ref()) {
        CandidateIdentity::Cluster(*cluster_id)
    } else {
        CandidateIdentity::Exact(candidate.evidence_id())
    }
}

fn candidate_order(candidate: &EvidenceCandidate) -> (u64, u64) {
    (
        candidate.evidence_id().value(),
        candidate.artifact_version().value(),
    )
}

/// Merges `extra`'s lane scores into `base`, keeping the base's identity and
/// metadata. A score kind the base already carries is kept as-is: the kind
/// identifies the model, and a same-model lane from another retriever is
/// redundant provenance, not a second measurement.
fn merge_lane_scores(
    base: &EvidenceCandidate,
    extra: &EvidenceCandidate,
) -> RetrievalResult<EvidenceCandidate> {
    let mut lanes = base.scores().lanes().to_vec();
    let mut added = false;
    for lane in extra.scores().lanes() {
        if !lanes
            .iter()
            .any(|existing| existing.score_kind == lane.score_kind)
        {
            lanes.push(lane.clone());
            added = true;
        }
    }
    if !added {
        return Ok(base.clone());
    }
    let scores = maestria_domain::RetrievalScoreSet::new(lanes)
        .map_err(|error| RetrievalError::Internal(format!("merge fused lane scores: {error}")))?;
    EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: base.evidence_id(),
        artifact_version: base.artifact_version(),
        source_span: base.source_span().clone(),
        scores,
        trust: base.trust(),
        freshness: base.freshness(),
        duplicate_cluster: base.duplicate_cluster(),
        reasons: base.reasons().to_vec(),
        coverage_keys: base.coverage_keys().to_vec(),
    })
    .map_err(|error| RetrievalError::Internal(format!("rebuild fused candidate: {error}")))
}

/// Score-level fusion with per-lane min-max normalization and a fixed
/// lexical weight.
///
/// The lexical lane's normalized scores carry `lexical_weight`; the blended
/// lanes (dense, learned-sparse, late-interaction) share the remainder
/// equally. Raw lane scores are min-max-normalized to [0, 1] before blending
/// so heterogeneous score scales cannot dominate the mix. A candidate
/// present in several lanes accumulates its weighted contributions, which
/// keeps the lexical first hits on top while blended lanes contribute
/// coverage below them.
pub struct NormalizedBlend {
    pub lexical_weight: f32,
    pub blended_kinds: Vec<RetrievalScoreKind>,
}

impl NormalizedBlend {
    pub fn new(lexical_weight: f32, blended_kinds: Vec<RetrievalScoreKind>) -> Self {
        Self {
            lexical_weight,
            blended_kinds,
        }
    }
}

const BLEND_SCALE: u64 = 1_000_000;

impl RankFusion for NormalizedBlend {
    fn fuse(
        &self,
        _query: &SearchQuery,
        batches: &[crate::types::CandidateBatch],
    ) -> RetrievalResult<Vec<FusedCandidate>> {
        if !(0.0..=1.0).contains(&self.lexical_weight) || self.blended_kinds.is_empty() {
            return Err(RetrievalError::Internal(
                "NormalizedBlend requires a lexical weight in [0, 1] and blended kinds".to_string(),
            ));
        }
        let evidence_clusters = collect_evidence_clusters(batches);
        let kinds = self
            .blended_kinds
            .iter()
            .cloned()
            .chain(std::iter::once(RetrievalScoreKind::LexicalBm25))
            .collect::<Vec<_>>();
        let bounds = blend_bounds(batches, &kinds);
        let (scores, best_candidates) =
            blend_scores(self, batches, &kinds, &bounds, &evidence_clusters)?;
        Ok(finalize_fusion(scores, best_candidates))
    }
}

fn blend_bounds(
    batches: &[crate::types::CandidateBatch],
    kinds: &[RetrievalScoreKind],
) -> std::collections::BTreeMap<RetrievalScoreKind, (f32, f32)> {
    let mut bounds = std::collections::BTreeMap::<RetrievalScoreKind, (f32, f32)>::new();
    for batch in batches {
        if !matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded) {
            continue;
        }
        for candidate in &batch.candidates {
            for kind in kinds {
                let Some(lane) = candidate.scores().lane(kind) else {
                    continue;
                };
                let raw = lane.raw_score as f32;
                let (min, max) = bounds.entry(kind.clone()).or_insert((raw, raw));
                *min = min.min(raw);
                *max = max.max(raw);
            }
        }
    }
    bounds
}

fn blend_scores(
    fusion: &NormalizedBlend,
    batches: &[crate::types::CandidateBatch],
    kinds: &[RetrievalScoreKind],
    bounds: &std::collections::BTreeMap<RetrievalScoreKind, (f32, f32)>,
    evidence_clusters: &std::collections::BTreeMap<
        maestria_domain::EvidenceId,
        maestria_domain::DuplicateClusterId,
    >,
) -> RetrievalResult<(
    std::collections::BTreeMap<CandidateIdentity, u64>,
    std::collections::BTreeMap<CandidateIdentity, EvidenceCandidate>,
)> {
    let mut scores = std::collections::BTreeMap::<CandidateIdentity, u64>::new();
    let mut best_candidates: std::collections::BTreeMap<CandidateIdentity, EvidenceCandidate> =
        std::collections::BTreeMap::new();
    for batch in batches {
        if !matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded) {
            continue;
        }
        for candidate in &batch.candidates {
            let identity =
                candidate_identity(candidate, evidence_clusters.get(&candidate.evidence_id()));
            let mut blended = 0.0_f32;
            for kind in kinds {
                let Some(lane) = candidate.scores().lane(kind) else {
                    continue;
                };
                let weight = if *kind == RetrievalScoreKind::LexicalBm25 {
                    fusion.lexical_weight
                } else {
                    (1.0 - fusion.lexical_weight) / fusion.blended_kinds.len() as f32
                };
                let default_bounds = (0.0, 1.0);
                let (min, max) = bounds.get(kind).map_or(default_bounds, |bounds| *bounds);
                let normalized = if max > min {
                    ((lane.raw_score as f32 - min) / (max - min)).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                blended += weight * normalized;
            }
            let contribution = (blended.clamp(0.0, 1.0) * BLEND_SCALE as f32) as u64;
            scores
                .entry(identity.clone())
                .and_modify(|score| *score = (*score).saturating_add(contribution))
                .or_insert(contribution);
            record_candidate(&mut best_candidates, &identity, candidate)?;
        }
    }
    Ok((scores, best_candidates))
}

fn collect_evidence_clusters(
    batches: &[crate::types::CandidateBatch],
) -> std::collections::BTreeMap<maestria_domain::EvidenceId, maestria_domain::DuplicateClusterId> {
    let mut evidence_clusters = std::collections::BTreeMap::new();
    for batch in batches {
        if !matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded) {
            continue;
        }
        for candidate in &batch.candidates {
            if let Some(cluster) = candidate.duplicate_cluster() {
                evidence_clusters
                    .entry(candidate.evidence_id())
                    .and_modify(|existing: &mut maestria_domain::DuplicateClusterId| {
                        *existing = (*existing).min(cluster)
                    })
                    .or_insert(cluster);
            }
        }
    }
    evidence_clusters
}

fn record_candidate(
    best_candidates: &mut std::collections::BTreeMap<CandidateIdentity, EvidenceCandidate>,
    identity: &CandidateIdentity,
    candidate: &EvidenceCandidate,
) -> RetrievalResult<()> {
    let canonical_candidate = match identity {
        CandidateIdentity::Cluster(cluster_id)
            if candidate.duplicate_cluster().as_ref() != Some(cluster_id) =>
        {
            EvidenceCandidate::new(EvidenceCandidateDto {
                evidence_id: candidate.evidence_id(),
                artifact_version: candidate.artifact_version(),
                source_span: candidate.source_span().clone(),
                scores: candidate.scores().clone(),
                trust: candidate.trust(),
                freshness: candidate.freshness(),
                duplicate_cluster: Some(*cluster_id),
                reasons: candidate.reasons().to_vec(),
                coverage_keys: candidate.coverage_keys().to_vec(),
            })?
        }
        _ => candidate.clone(),
    };
    let replace = best_candidates
        .get(identity)
        .is_none_or(|existing| candidate_order(&canonical_candidate) < candidate_order(existing));
    if replace {
        best_candidates.insert(identity.clone(), canonical_candidate);
    } else if let Some(existing) = best_candidates.get_mut(identity) {
        let merged = merge_lane_scores(existing, &canonical_candidate)?;
        *existing = merged;
    }
    Ok(())
}

fn finalize_fusion(
    scores: std::collections::BTreeMap<CandidateIdentity, u64>,
    mut best_candidates: std::collections::BTreeMap<CandidateIdentity, EvidenceCandidate>,
) -> Vec<FusedCandidate> {
    let mut sorted = scores.into_iter().collect::<Vec<_>>();
    sorted.sort_by(|(left_id, left_score), (right_id, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_id.cmp(right_id))
    });
    sorted
        .into_iter()
        .filter_map(|(identity, _)| {
            best_candidates
                .remove(&identity)
                .map(|candidate| FusedCandidate { candidate })
        })
        .collect()
}
