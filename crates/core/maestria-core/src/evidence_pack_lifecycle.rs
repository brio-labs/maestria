use maestria_domain::{ConflictSet, EvidenceId, SearchPlan};

use super::evidence_pack::{
    ClaimCoverageStatus, ClaimEvidenceCoverage, EvidencePackError, EvidencePackMetadata,
};
use super::{SourceGroundedCardHit, SourceGroundedSearchHit};

#[path = "evidence_pack_operations.rs"]
mod operations;

#[derive(Debug, Clone, PartialEq)]
pub struct EvidencePack {
    pub(crate) query: String,
    pub(crate) cards: Vec<SourceGroundedCardHit>,
    pub(crate) chunks: Vec<SourceGroundedSearchHit>,
    pub(crate) evidence_ids: Vec<EvidenceId>,
    pub(crate) metadata: EvidencePackMetadata,
    frozen_digest: Option<String>,
    compression_verified: bool,
}

impl EvidencePack {
    pub fn from_plan(
        query: String,
        cards: Vec<SourceGroundedCardHit>,
        chunks: Vec<SourceGroundedSearchHit>,
        evidence_ids: Vec<EvidenceId>,
        plan: &SearchPlan,
    ) -> Result<Self, EvidencePackError> {
        let chunk_ids = chunks
            .iter()
            .map(|hit| hit.evidence.id)
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(unmaterialized_id) = evidence_ids
            .iter()
            .find(|evidence_id| !chunk_ids.contains(evidence_id))
        {
            return Err(EvidencePackError::UnmaterializedEvidence(format!(
                "{unmaterialized_id}"
            )));
        }
        let mut metadata = EvidencePackMetadata::from_plan(plan);
        metadata.populate_from_chunks(&chunks, &evidence_ids, cards.len());
        Ok(Self {
            query,
            cards,
            chunks,
            evidence_ids,
            metadata,
            frozen_digest: None,
            compression_verified: false,
        })
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn cards(&self) -> &[SourceGroundedCardHit] {
        &self.cards
    }
    pub fn chunks(&self) -> &[SourceGroundedSearchHit] {
        &self.chunks
    }
    pub fn evidence_ids(&self) -> &[EvidenceId] {
        &self.evidence_ids
    }
    pub fn metadata(&self) -> &EvidencePackMetadata {
        &self.metadata
    }
    pub fn mark_primary_sources_verified(
        &mut self,
        verified: bool,
    ) -> Result<(), EvidencePackError> {
        if self.is_frozen() {
            return Err(EvidencePackError::FrozenMutation(
                "frozen evidence pack cannot change primary-source verification".to_string(),
            ));
        }
        self.metadata.mark_primary_sources_verified(verified);
        Ok(())
    }
    pub fn set_claim_coverage(
        &mut self,
        coverage: Vec<ClaimEvidenceCoverage>,
    ) -> Result<(), EvidencePackError> {
        if self.is_frozen() {
            return Err(EvidencePackError::FrozenMutation(
                "frozen evidence pack cannot change claim coverage".to_string(),
            ));
        }
        let required = self
            .metadata
            .claims_required
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        let mut seen = std::collections::BTreeSet::new();
        let available = self
            .evidence_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if coverage.len() != required.len()
            || coverage.iter().any(|entry| {
                !required.contains(&entry.claim)
                    || !seen.insert(&entry.claim)
                    || match &entry.status {
                        ClaimCoverageStatus::Missing => !entry.evidence_ids.is_empty(),
                        ClaimCoverageStatus::Supported
                        | ClaimCoverageStatus::Partial
                        | ClaimCoverageStatus::Conflicted => entry.evidence_ids.is_empty(),
                    }
                    || entry
                        .evidence_ids
                        .iter()
                        .any(|evidence_id| !available.contains(evidence_id))
            })
        {
            return Err(EvidencePackError::InvalidCoverage(
                "claim coverage must match required claims and pack evidence".to_string(),
            ));
        }
        self.metadata.claim_coverage = coverage;
        self.metadata.missing_evidence = self
            .metadata
            .claim_coverage
            .iter()
            .filter(|entry| {
                matches!(
                    entry.status,
                    ClaimCoverageStatus::Missing
                        | ClaimCoverageStatus::Partial
                        | ClaimCoverageStatus::Conflicted
                )
            })
            .map(|entry| entry.claim.clone())
            .collect();
        self.metadata.refresh_stop_reason();
        Ok(())
    }

    pub fn set_conflicts(
        &mut self,
        conflicts: Vec<ConflictSet>,
        counterevidence: Vec<EvidenceId>,
    ) -> Result<(), EvidencePackError> {
        if self.is_frozen() {
            return Err(EvidencePackError::FrozenMutation(
                "frozen evidence pack cannot change conflicts".to_string(),
            ));
        }
        let available = self
            .evidence_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let invalid_reference = counterevidence.iter().any(|id| !available.contains(id))
            || conflicts
                .iter()
                .flat_map(|conflict| &conflict.candidates)
                .any(|candidate| {
                    !available.contains(&candidate.evidence_id)
                        || !self.chunks.iter().any(|hit| {
                            crate::evidence_pack_provenance::candidate_provenance_matches_hit(
                                candidate.evidence_id,
                                candidate.artifact_version,
                                &candidate.source_span,
                                hit,
                            )
                        })
                });
        if invalid_reference {
            return Err(EvidencePackError::InvalidCoverage(
                "conflicts and counterevidence must reference pack evidence".to_string(),
            ));
        }
        self.metadata.set_conflicts(conflicts, counterevidence);
        Ok(())
    }
}
