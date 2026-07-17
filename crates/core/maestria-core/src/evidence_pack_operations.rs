use maestria_domain::{EvidenceId, EvidenceKind, SearchTrace};
use sha2::{Digest, Sha256};

use super::super::evidence_pack::{
    ClaimCoverageStatus, EvidencePackCompression, EvidencePackError, EvidencePackReplayKey,
    EvidencePackReproducibility,
};
use super::EvidencePack;
use crate::evidence_pack_provenance::candidate_provenance_matches_hit;

impl EvidencePack {
    pub fn freeze(
        &mut self,
        trace: SearchTrace,
        policy_fingerprint: String,
    ) -> Result<(), EvidencePackError> {
        if self.is_frozen() {
            return Err(EvidencePackError::InvalidFreeze(
                "evidence pack is already frozen".to_string(),
            ));
        }
        if policy_fingerprint.trim().is_empty() {
            return Err(EvidencePackError::InvalidFreeze(
                "policy fingerprint must not be empty".to_string(),
            ));
        }
        validate_freeze_identity(self, &trace, &policy_fingerprint)?;
        validate_trace_outcome(self, &trace)?;
        validate_trace_evidence(self, &trace)?;
        if matches!(
            self.metadata.compression,
            EvidencePackCompression::Compressed { .. }
        ) && !self.compression_verified
        {
            return Err(EvidencePackError::InvalidFreeze(
                "compressed evidence lineage was not validated".to_string(),
            ));
        }
        validate_pack_materialization(self)?;
        let trace_id = trace.deterministic_id();
        let key = EvidencePackReplayKey {
            trace: trace_id,
            corpus_snapshot: self.metadata.corpus_snapshot,
            index_generation: self.metadata.index_generation,
            fingerprint: self.metadata.fingerprint.clone(),
            policy_fingerprint: policy_fingerprint.clone(),
        };
        self.metadata.search_trace = Some(trace_id);
        self.metadata.policy_fingerprint = Some(policy_fingerprint);
        self.metadata.reproducibility = EvidencePackReproducibility::Frozen(key);
        self.frozen_digest = Some(pack_digest(self));
        Ok(())
    }

    pub fn reproduce(&self, key: &EvidencePackReplayKey) -> Result<Self, EvidencePackError> {
        let digest = pack_digest(self);
        match &self.metadata.reproducibility {
            EvidencePackReproducibility::Frozen(existing)
                if existing == key && self.frozen_digest.as_deref() == Some(digest.as_str()) =>
            {
                Ok(self.clone())
            }
            EvidencePackReproducibility::Frozen(_) => {
                Err(EvidencePackError::ReplayIdentityMismatch)
            }
            EvidencePackReproducibility::LiveNonReproducible { .. } => {
                Err(EvidencePackError::NotReproducible)
            }
        }
    }

    pub fn compress(
        &mut self,
        retained_evidence_ids: Vec<EvidenceId>,
        selector: String,
    ) -> Result<(), EvidencePackError> {
        let (source_evidence_ids, retained_ids) =
            validate_compression_request(self, &retained_evidence_ids, &selector)?;
        self.chunks
            .retain(|hit| retained_ids.contains(&hit.evidence.id));
        self.evidence_ids.retain(|id| retained_ids.contains(id));
        self.metadata
            .freshness
            .retain(|entry| retained_ids.contains(&entry.evidence_id));
        self.metadata
            .source_independence
            .iter_mut()
            .for_each(|source| {
                source.evidence_ids.retain(|id| retained_ids.contains(id));
            });
        self.metadata
            .source_independence
            .retain(|source| !source.evidence_ids.is_empty());
        self.metadata.distinct_sources = self.metadata.source_independence.len();
        self.metadata.distinct_documents = self
            .chunks
            .iter()
            .map(|hit| hit.artifact.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        self.metadata.distinct_sections = self
            .chunks
            .iter()
            .map(|hit| (hit.artifact.id, hit.chunk.node_id))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        self.metadata
            .claim_coverage
            .iter_mut()
            .for_each(|coverage| {
                coverage.evidence_ids.retain(|id| retained_ids.contains(id));
                if coverage.evidence_ids.is_empty() {
                    coverage.status = ClaimCoverageStatus::Missing;
                }
            });
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
        self.metadata.compression = EvidencePackCompression::Compressed {
            source_evidence_ids,
            retained_evidence_ids,
            selector,
        };
        self.compression_verified = true;
        self.metadata.refresh_stop_reason();
        Ok(())
    }

    pub(super) fn is_frozen(&self) -> bool {
        self.frozen_digest.is_some()
            || matches!(
                self.metadata.reproducibility,
                EvidencePackReproducibility::Frozen(_)
            )
    }
}

fn validate_compression_request(
    pack: &EvidencePack,
    retained_evidence_ids: &[EvidenceId],
    selector: &str,
) -> Result<(Vec<EvidenceId>, std::collections::BTreeSet<EvidenceId>), EvidencePackError> {
    if pack.is_frozen() {
        return Err(EvidencePackError::InvalidCompression(
            "frozen evidence pack cannot be compressed".to_string(),
        ));
    }
    if matches!(
        pack.metadata.compression,
        EvidencePackCompression::Compressed { .. }
    ) {
        return Err(EvidencePackError::InvalidCompression(
            "compressed evidence cannot be compressed again".to_string(),
        ));
    }
    if selector.trim().is_empty() {
        return Err(EvidencePackError::InvalidCompression(
            "compressed evidence requires a selector".to_string(),
        ));
    }
    let source_evidence_ids = pack.evidence_ids.clone();
    let source_ids = source_evidence_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let source_chunk_ids = pack
        .chunks
        .iter()
        .map(|hit| hit.evidence.id)
        .collect::<std::collections::BTreeSet<_>>();
    if source_evidence_ids
        .iter()
        .any(|evidence_id| !source_chunk_ids.contains(evidence_id))
        || pack
            .chunks
            .iter()
            .any(|hit| !immutable_evidence(&hit.evidence.kind))
    {
        return Err(EvidencePackError::InvalidCompression(
            "compressed evidence requires immutable materialized source data".to_string(),
        ));
    }
    if retained_evidence_ids
        .iter()
        .any(|evidence_id| !source_ids.contains(evidence_id))
    {
        return Err(EvidencePackError::InvalidCompression(
            "compressed evidence must retain source evidence IDs".to_string(),
        ));
    }
    Ok((
        source_evidence_ids,
        retained_evidence_ids.iter().copied().collect(),
    ))
}

fn validate_freeze_identity(
    pack: &EvidencePack,
    trace: &SearchTrace,
    policy_fingerprint: &str,
) -> Result<(), EvidencePackError> {
    if trace.query_id != pack.metadata.query_id
        || trace.original_query != pack.query
        || trace.corpus_snapshot != pack.metadata.corpus_snapshot
        || trace.index_generation != pack.metadata.index_generation
        || trace.fingerprint != pack.metadata.fingerprint
        || trace.policy_fingerprint.as_deref() != Some(policy_fingerprint)
    {
        return Err(EvidencePackError::InvalidFreeze(
            "trace identity does not match evidence pack metadata".to_string(),
        ));
    }
    Ok(())
}

fn validate_trace_outcome(
    pack: &EvidencePack,
    trace: &SearchTrace,
) -> Result<(), EvidencePackError> {
    let conflict_ids = pack
        .metadata
        .conflicts
        .iter()
        .map(|conflict| conflict.id)
        .collect::<Vec<_>>();
    if trace.stop_reason != pack.metadata.stop_reason
        || trace.missing_evidence != pack.metadata.missing_evidence
        || trace.conflicts != conflict_ids
    {
        return Err(EvidencePackError::InvalidFreeze(
            "trace outcome metadata does not match evidence pack".to_string(),
        ));
    }
    Ok(())
}

fn validate_trace_evidence(
    pack: &EvidencePack,
    trace: &SearchTrace,
) -> Result<(), EvidencePackError> {
    let trace_evidence_ids = trace
        .raw_candidates
        .iter()
        .map(|candidate| candidate.evidence_id)
        .collect::<Vec<_>>();
    if trace_evidence_ids != pack.evidence_ids {
        return Err(EvidencePackError::InvalidFreeze(
            "trace evidence does not match evidence pack evidence".to_string(),
        ));
    }
    if trace.raw_candidates.iter().any(|candidate| {
        !pack.chunks.iter().any(|hit| {
            candidate_provenance_matches_hit(
                candidate.evidence_id,
                candidate.artifact_version,
                &candidate.source_span,
                hit,
            )
        })
    }) {
        return Err(EvidencePackError::InvalidFreeze(
            "trace candidate provenance does not match evidence pack".to_string(),
        ));
    }
    Ok(())
}

fn validate_pack_materialization(pack: &EvidencePack) -> Result<(), EvidencePackError> {
    let chunk_ids = pack
        .chunks
        .iter()
        .map(|hit| hit.evidence.id)
        .collect::<std::collections::BTreeSet<_>>();
    let compression_source_ids = match &pack.metadata.compression {
        EvidencePackCompression::Compressed {
            source_evidence_ids,
            ..
        } => source_evidence_ids.iter().copied().collect(),
        EvidencePackCompression::Verbatim { .. } => std::collections::BTreeSet::new(),
    };
    let mut metadata_references = pack.metadata.counterevidence.iter().copied().chain(
        pack.metadata
            .conflicts
            .iter()
            .flat_map(|conflict| conflict.candidates.iter())
            .map(|candidate| candidate.evidence_id),
    );
    if metadata_references.any(|evidence_id| {
        !chunk_ids.contains(&evidence_id) && !compression_source_ids.contains(&evidence_id)
    }) {
        return Err(EvidencePackError::InvalidFreeze(
            "conflict metadata references unverified evidence".to_string(),
        ));
    }
    if pack
        .evidence_ids
        .iter()
        .any(|evidence_id| !chunk_ids.contains(evidence_id))
        || pack
            .chunks
            .iter()
            .any(|hit| !immutable_evidence(&hit.evidence.kind))
    {
        return Err(EvidencePackError::InvalidFreeze(
            "all evidence must have immutable source data".to_string(),
        ));
    }
    Ok(())
}

fn pack_digest(pack: &EvidencePack) -> String {
    let mut digest = Sha256::new();
    digest.update(
        format!(
            "{:?}",
            (
                &pack.query,
                &pack.cards,
                &pack.chunks,
                &pack.evidence_ids,
                &pack.metadata,
                pack.compression_verified,
            )
        )
        .as_bytes(),
    );
    format!("{:x}", digest.finalize())
}

fn immutable_evidence(kind: &EvidenceKind) -> bool {
    match kind {
        EvidenceKind::FileSpan { snapshot, .. } => snapshot.is_some(),
        EvidenceKind::PdfSpan { .. }
        | EvidenceKind::WebSnapshot { .. }
        | EvidenceKind::CommandOutput { .. }
        | EvidenceKind::TestResult { .. }
        | EvidenceKind::Diff { .. }
        | EvidenceKind::Validation { .. } => true,
    }
}
