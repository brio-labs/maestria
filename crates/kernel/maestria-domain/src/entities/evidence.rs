use crate::evidence_source::EvidenceKind;
use crate::ids::{ArtifactId, ClaimId, EvidenceId, LogicalTick};
use crate::security::SecurityMetadata;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub claim_id: Option<ClaimId>,
    pub kind: EvidenceKind,
    pub excerpt: String,
    pub observed_at: LogicalTick,
    pub security: SecurityMetadata,
}

impl Evidence {
    pub(crate) fn new(
        id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        kind: EvidenceKind,
        excerpt: String,
        observed_at: LogicalTick,
        security: SecurityMetadata,
    ) -> Self {
        Self {
            id,
            artifact_id,
            claim_id,
            kind,
            excerpt,
            observed_at,
            security,
        }
    }
}
