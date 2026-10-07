use sillage_domain::{EvidenceCandidate, EvidenceCandidateDto, RetrievalScoreKind};
use sillage_ports::SearchQuery;

use crate::traits::RankFusion;
use crate::types::{FusedCandidate, RetrievalError, RetrievalResult};
#[path = "fusion_lexical_head.rs"]
mod lexical_head;
pub use lexical_head::HybridLexicalHead;
#[path = "fusion_rrf.rs"]
mod rrf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateIdentity {
    Cluster(sillage_domain::DuplicateClusterId),
    Exact(sillage_domain::EvidenceId),
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

fn candidate_identity(
    candidate: &EvidenceCandidate,
    normalized_cluster: Option<&sillage_domain::DuplicateClusterId>,
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
    let scores = sillage_domain::RetrievalScoreSet::new(lanes)
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
        let evidence_clusters = collect_evidence_clusters(batches, false);
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
        if !matches!(batch.status, sillage_domain::SearchLaneStatus::Succeeded) {
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
        sillage_domain::EvidenceId,
        sillage_domain::DuplicateClusterId,
    >,
) -> RetrievalResult<(
    std::collections::BTreeMap<CandidateIdentity, u64>,
    std::collections::BTreeMap<CandidateIdentity, EvidenceCandidate>,
)> {
    let mut scores = std::collections::BTreeMap::<CandidateIdentity, u64>::new();
    let mut best_candidates: std::collections::BTreeMap<CandidateIdentity, EvidenceCandidate> =
        std::collections::BTreeMap::new();
    for batch in batches {
        if !matches!(batch.status, sillage_domain::SearchLaneStatus::Succeeded) {
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
                .entry(identity)
                .and_modify(|score| *score = (*score).saturating_add(contribution))
                .or_insert(contribution);
            record_candidate(&mut best_candidates, identity, candidate, None)?;
        }
    }
    Ok((scores, best_candidates))
}

fn collect_evidence_clusters(
    batches: &[crate::types::CandidateBatch],
    lexical_only: bool,
) -> std::collections::BTreeMap<sillage_domain::EvidenceId, sillage_domain::DuplicateClusterId> {
    let mut evidence_clusters = std::collections::BTreeMap::new();
    for batch in batches {
        if (lexical_only && batch.descriptor.is_dense())
            || !matches!(batch.status, sillage_domain::SearchLaneStatus::Succeeded)
        {
            continue;
        }
        for candidate in &batch.candidates {
            if let Some(cluster) = candidate.duplicate_cluster() {
                evidence_clusters
                    .entry(candidate.evidence_id())
                    .and_modify(|existing: &mut sillage_domain::DuplicateClusterId| {
                        *existing = (*existing).min(cluster)
                    })
                    .or_insert(cluster);
            }
        }
    }
    evidence_clusters
}

pub(super) fn collect_hybrid_evidence_clusters(
    batches: &[crate::types::CandidateBatch],
) -> (
    std::collections::BTreeMap<sillage_domain::EvidenceId, sillage_domain::DuplicateClusterId>,
    std::collections::BTreeMap<sillage_domain::EvidenceId, sillage_domain::DuplicateClusterId>,
) {
    let mut evidence_clusters = std::collections::BTreeMap::new();
    let mut lexical_clusters = std::collections::BTreeMap::new();
    for batch in batches {
        if !matches!(batch.status, sillage_domain::SearchLaneStatus::Succeeded) {
            continue;
        }
        for candidate in &batch.candidates {
            let Some(cluster) = candidate.duplicate_cluster() else {
                continue;
            };
            let evidence_id = candidate.evidence_id();
            evidence_clusters
                .entry(evidence_id)
                .and_modify(|existing: &mut sillage_domain::DuplicateClusterId| {
                    *existing = (*existing).min(cluster)
                })
                .or_insert(cluster);
            if !batch.descriptor.is_dense() {
                lexical_clusters
                    .entry(evidence_id)
                    .and_modify(|existing: &mut sillage_domain::DuplicateClusterId| {
                        *existing = (*existing).min(cluster)
                    })
                    .or_insert(cluster);
            }
        }
    }
    (evidence_clusters, lexical_clusters)
}

fn record_candidate(
    best_candidates: &mut std::collections::BTreeMap<CandidateIdentity, EvidenceCandidate>,
    identity: CandidateIdentity,
    candidate: &EvidenceCandidate,
    protected_head: Option<&EvidenceCandidate>,
) -> RetrievalResult<()> {
    let is_protected_head = protected_head.is_some_and(|head| candidate == head);
    let canonical_candidate = if is_protected_head {
        candidate.clone()
    } else {
        match identity {
            CandidateIdentity::Cluster(cluster_id)
                if candidate.duplicate_cluster() != Some(cluster_id) =>
            {
                EvidenceCandidate::new(EvidenceCandidateDto {
                    evidence_id: candidate.evidence_id(),
                    artifact_version: candidate.artifact_version(),
                    source_span: candidate.source_span().clone(),
                    scores: candidate.scores().clone(),
                    trust: candidate.trust(),
                    freshness: candidate.freshness(),
                    duplicate_cluster: Some(cluster_id),
                    reasons: candidate.reasons().to_vec(),
                    coverage_keys: candidate.coverage_keys().to_vec(),
                })?
            }
            _ => candidate.clone(),
        }
    };
    match best_candidates.entry(identity) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(canonical_candidate);
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            let stored_is_protected_head = protected_head.is_some_and(|head| entry.get() == head);
            if is_protected_head {
                let merged = merge_lane_scores(&canonical_candidate, entry.get())?;
                *entry.get_mut() = merged;
            } else if stored_is_protected_head {
                let merged = merge_lane_scores(entry.get(), &canonical_candidate)?;
                *entry.get_mut() = merged;
            } else if candidate_order(&canonical_candidate) < candidate_order(entry.get()) {
                entry.insert(canonical_candidate);
            } else {
                let merged = merge_lane_scores(entry.get(), &canonical_candidate)?;
                *entry.get_mut() = merged;
            }
        }
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
