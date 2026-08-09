//! The sparse lane's per-hit authorization and candidate assembly.
//!
//! Every visited chunk runs the full record check — owner lookup, artifact
//! authorization, chunk content secret scan, evidence ownership, per-artifact
//! snapshot verification — while the record and verification caches stay
//! request-scoped.

use super::common::{candidate_from_records, port_error};
use super::learned_sparse::{LearnedSparseChunkRetriever, SparseRecords};
use super::prescore_cache::PrescoreCache;
use super::sparse_record_cache::RecordCache;
use crate::types::{CandidateRequest, CandidateSourceFilter, RetrievalError};
use maestria_domain::EvidenceCandidate;
use maestria_ports::SparseSearchHit;

impl LearnedSparseChunkRetriever {
    pub(super) fn checked_records(
        &self,
        chunk_id: maestria_domain::ChunkId,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
        records: &RecordCache,
    ) -> Result<Option<SparseRecords>, RetrievalError> {
        if !super::chunk_access::source_filter_allows_chunk(
            self.chunks.as_ref(),
            chunk_id,
            source_filter,
        )
        .map_err(port_error)?
        {
            return Ok(None);
        }
        let Some(owner_id) = self.chunks.find_artifact_id(chunk_id).map_err(port_error)? else {
            return Ok(None);
        };
        let Some(artifact) = records
            .artifact(self.artifacts.as_ref(), self.evidence.as_ref(), owner_id)
            .map_err(port_error)?
        else {
            return Ok(None);
        };
        if artifact.index_status != maestria_domain::IndexStatus::Indexed
            || authorization.evaluate(&artifact.security)
                != maestria_governance::RetrievalDecision::Allowed
        {
            return Ok(None);
        }
        let Some(chunk) = self.chunks.get(chunk_id).map_err(port_error)? else {
            return Ok(None);
        };
        if chunk.artifact_id != owner_id {
            return Err(port_error(maestria_ports::PortError::Conflict {
                message: format!(
                    "chunk {chunk_id} owner mismatch: metadata points to artifact {owner_id}, full row points to {}",
                    chunk.artifact_id
                ),
            }));
        }
        if !maestria_governance::scan_secrets(&chunk.text).is_clean() {
            return Ok(None);
        }
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = records
            .evidence(self.evidence.as_ref(), evidence_id)
            .map_err(port_error)?
        else {
            return Ok(None);
        };
        if evidence.artifact_id != artifact.id {
            return Err(port_error(maestria_ports::PortError::Conflict {
                message: format!(
                    "evidence {} owner mismatch: expected artifact {}, got {}",
                    evidence.id, artifact.id, evidence.artifact_id
                ),
            }));
        }
        if authorization.evaluate(&evidence.security)
            != maestria_governance::RetrievalDecision::Allowed
            || !maestria_governance::scan_secrets(&evidence.excerpt).is_clean()
        {
            return Ok(None);
        }
        // Content-addressed snapshot: one verification per artifact per search.
        if !records.is_verified(evidence.artifact_id) {
            self.verifier.verify(&evidence, &artifact)?;
            records.mark_verified(evidence.artifact_id);
        }
        Ok(Some((artifact, chunk, evidence)))
    }

    pub(super) fn candidate_from_hit(
        &self,
        hit: SparseSearchHit,
        raw_rank: u32,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
        prescore_cache: &PrescoreCache<SparseRecords>,
        record_cache: &RecordCache,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        let records = match prescore_cache.take(hit.chunk_id) {
            Some(records) => Some(records),
            None => {
                self.checked_records(hit.chunk_id, authorization, source_filter, record_cache)?
            }
        };
        let Some((artifact, chunk, evidence)) = records else {
            return Ok(None);
        };
        if source_filter.is_some_and(|filter| !filter.allows(artifact.id)) {
            return Ok(None);
        }
        let contributions = hit
            .contributions
            .into_iter()
            .map(|contribution| maestria_domain::LearnedSparseContribution {
                term_id: contribution.term_id,
                contribution_micros: contribution.contribution_micros,
            })
            .collect();
        candidate_from_records(
            artifact.id,
            artifact.content_hash.as_ref(),
            &chunk.source_span,
            &evidence,
            chunk.node_id,
            super::score_provenance::learned_sparse_score(
                &self.identity,
                self.fingerprint.clone(),
                hit.score_micros,
                raw_rank,
            )?,
            vec![maestria_domain::RetrievalReason::LearnedSparse(Box::new(
                maestria_domain::LearnedSparseReason::new(contributions),
            ))],
        )
        .map(Some)
    }

    pub(super) fn preflight_chunk(
        &self,
        chunk_id: maestria_domain::ChunkId,
        request: &CandidateRequest,
        prescore_cache: &PrescoreCache<SparseRecords>,
        record_cache: &RecordCache,
    ) -> Result<bool, RetrievalError> {
        let Some(records) = self.checked_records(
            chunk_id,
            &request.authorization,
            request.source_filter.as_ref(),
            record_cache,
        )?
        else {
            return Ok(false);
        };
        if request
            .source_filter
            .as_ref()
            .is_some_and(|filter| !filter.allows(records.0.id))
        {
            return Ok(false);
        }
        prescore_cache.insert(chunk_id, records);
        Ok(true)
    }
}
