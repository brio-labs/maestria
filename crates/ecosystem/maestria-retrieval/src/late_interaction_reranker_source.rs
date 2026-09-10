use maestria_domain::{EvidenceCandidate, EvidenceKind, IndexStatus, SourceLocation};
use maestria_governance::{RetrievalAuthorizationContext, RetrievalDecision, scan_secrets};
use maestria_ports::MultiVectorSourceIdentity;

use crate::adapters::SourceSnapshotVerifier;
use crate::cancellation::SearchCallControl;
use crate::types::{CandidateSourceFilter, RankedCandidate};

use super::{LateInteractionReranker, MAX_DOCUMENT_BYTES, MAX_SNAPSHOT_BYTES};

impl LateInteractionReranker {
    fn candidate_span_matches_evidence(candidate: &EvidenceCandidate, kind: &EvidenceKind) -> bool {
        match kind {
            EvidenceKind::FileSpan { path, range, .. } => {
                matches!(
                    candidate.source_span().location(),
                    SourceLocation::File {
                        path: candidate_path,
                        start_line,
                        end_line,
                    } if candidate_path == path
                        && u32::try_from(range.start()).ok() == Some(*start_line)
                        && u32::try_from(range.end()).ok() == Some(*end_line)
                )
            }
            EvidenceKind::WebSnapshot { .. } => true,
            _ => false,
        }
    }

    pub(crate) fn authorized_source(
        &self,
        candidate: &RankedCandidate,
        authorization: &RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
        control: &SearchCallControl,
    ) -> Result<(String, MultiVectorSourceIdentity), &'static str> {
        control
            .check()
            .map_err(|_| "late_interaction:budget_exhausted")?;
        let evidence = self
            .parts
            .evidence
            .get(candidate.candidate.evidence_id())
            .map_err(|_| "late_interaction:source_invalid")?
            .ok_or("late_interaction:source_invalid")?;
        let artifact = self
            .parts
            .artifacts
            .get(evidence.artifact_id)
            .map_err(|_| "late_interaction:source_invalid")?
            .ok_or("late_interaction:source_invalid")?;
        if source_filter.is_some_and(|filter| !filter.allows(artifact.id))
            || artifact.index_status != IndexStatus::Indexed
            || artifact.id != evidence.artifact_id
            || authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || authorization.evaluate(&evidence.security) != RetrievalDecision::Allowed
        {
            return Err("late_interaction:privacy_rejected");
        }
        if !Self::candidate_span_matches_evidence(&candidate.candidate, &evidence.kind) {
            return Err("late_interaction:source_invalid");
        }
        let snapshot = match &evidence.kind {
            EvidenceKind::FileSpan { snapshot, .. }
            | EvidenceKind::WebSnapshot { snapshot, .. } => snapshot,
            _ => return Err("late_interaction:source_invalid"),
        };
        let _verified_snapshot = SourceSnapshotVerifier::new(self.parts.blobs.clone())
            .verify_bounded(&evidence, &artifact, MAX_SNAPSHOT_BYTES)
            .map_err(|_| "late_interaction:source_invalid")?;
        if evidence.excerpt.len() > MAX_DOCUMENT_BYTES
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Err("late_interaction:privacy_rejected");
        }
        let source_hash = maestria_ports::MultiVectorSourceIdentity::representation_hash(
            &evidence.excerpt,
            evidence.id,
            artifact.id,
            candidate.candidate.artifact_version(),
            snapshot.content_hash(),
            candidate.candidate.source_span(),
        )
        .map_err(|_| "late_interaction:source_invalid")?;
        let source = MultiVectorSourceIdentity {
            evidence_id: evidence.id,
            artifact_id: artifact.id,
            version_id: candidate.candidate.artifact_version(),
            source_snapshot_hash: snapshot.content_hash().clone(),
            span: candidate.candidate.source_span().clone(),
            source_representation_hash: source_hash,
        };
        Ok((evidence.excerpt, source))
    }
}
