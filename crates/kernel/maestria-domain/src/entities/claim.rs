use std::collections::BTreeSet;

use crate::ids::{ArtifactId, ClaimId, EvidenceId};
use crate::security::SecurityMetadata;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimStatus {
    Draft,
    Proposed,
    Verified,
    Disputed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub id: ClaimId,
    pub artifact_id: ArtifactId,
    pub text: String,
    pub status: ClaimStatus,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub security: SecurityMetadata,
}

impl Claim {
    pub(crate) fn new(
        id: ClaimId,
        artifact_id: ArtifactId,
        text: String,
        security: SecurityMetadata,
    ) -> Self {
        Self {
            id,
            artifact_id,
            text,
            status: ClaimStatus::Draft,
            evidence_ids: BTreeSet::new(),
            security,
        }
    }
}
