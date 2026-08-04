//! Shared validator-test fixtures (Rule 26: fixtures are shared through
//! explicit helpers, never copied between test modules).

use crate::types::ValidationContext;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, Claim, ClaimId, ClaimStatus, ContentHash, Evidence, EvidenceId,
    EvidenceKind, LineRange, LogicalTick, MemoryCandidate, MemoryCandidateId, SecurityMetadata,
    SnapshotRef, Task, TaskId, TaskPriority, TaskStatus,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct ContextFixture {
    pub(crate) task: Option<Task>,
    pub(crate) artifacts: BTreeMap<ArtifactId, Artifact>,
    pub(crate) claims: BTreeMap<ClaimId, Claim>,
    pub(crate) evidences: BTreeMap<EvidenceId, Evidence>,
    pub(crate) memory_candidates: BTreeMap<MemoryCandidateId, MemoryCandidate>,
    pub(crate) harness_exit_code: Option<i32>,
}

impl ContextFixture {
    pub(crate) fn context(&self) -> ValidationContext<'_> {
        ValidationContext {
            task: self.task.as_ref(),
            artifacts: &self.artifacts,
            claims: &self.claims,
            evidences: &self.evidences,
            memory_candidates: &self.memory_candidates,
            harness_exit_code: self.harness_exit_code,
            search: None,
        }
    }
}

pub(crate) fn claim(id: u64, evidence_ids: impl IntoIterator<Item = EvidenceId>) -> Claim {
    Claim {
        id: ClaimId::new(id),
        artifact_id: ArtifactId::new(id),
        text: format!("claim {id}"),
        status: ClaimStatus::Proposed,
        evidence_ids: evidence_ids.into_iter().collect(),
        security: SecurityMetadata::default(),
    }
}

pub(crate) fn evidence(
    id: u64,
    claim_id: Option<ClaimId>,
) -> Result<Evidence, Box<dyn std::error::Error>> {
    Ok(Evidence {
        id: EvidenceId::new(id),
        artifact_id: ArtifactId::new(1),
        claim_id,
        kind: EvidenceKind::FileSpan {
            path: "src/lib.rs".to_string(),
            range: LineRange::new(1, 8)?,
            snapshot: SnapshotRef::new(
                BlobId::new(1),
                ContentHash::new(format!("sha256:{}", "a".repeat(64)))?,
            ),
        },
        excerpt: "evidence excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: SecurityMetadata::default(),
    })
}

pub(crate) fn task(id: u64, status: TaskStatus) -> Task {
    Task {
        id: TaskId::new(id),
        title: format!("task {id}"),
        priority: TaskPriority::Normal,
        status,
        artifact_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
    }
}

pub(crate) fn memory_candidate(
    id: u64,
    evidence_ids: impl IntoIterator<Item = EvidenceId>,
) -> Result<MemoryCandidate, maestria_domain::DomainError> {
    MemoryCandidate::try_new(
        MemoryCandidateId::new(id),
        ClaimId::new(id),
        evidence_ids.into_iter().collect(),
        900,
        SecurityMetadata::default(),
    )
}
