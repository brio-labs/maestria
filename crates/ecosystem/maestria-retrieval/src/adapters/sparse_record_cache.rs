//! Request-scoped caches of the sparse lane's durable record repos.
//!
//! The chunks of one artifact are visited together by a search; caching the
//! artifact row and prefetching the artifact's evidence rows on first touch
//! turns N per-chunk repository reads into one artifact read plus one
//! artifact-scoped list, while the authorization and verification per chunk
//! still run on every visit.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{Artifact, ArtifactId, Evidence, EvidenceId};
use maestria_ports::{ArtifactRepository, EvidenceRepository, PortError};

pub(super) struct RecordCache {
    artifacts: RefCell<BTreeMap<ArtifactId, Artifact>>,
    evidence: RefCell<BTreeMap<EvidenceId, Evidence>>,
    /// Artifacts whose content-addressed snapshot was verified this request.
    verified: RefCell<BTreeSet<ArtifactId>>,
}

impl RecordCache {
    pub(super) fn new() -> Self {
        Self {
            artifacts: RefCell::new(BTreeMap::new()),
            evidence: RefCell::new(BTreeMap::new()),
            verified: RefCell::new(BTreeSet::new()),
        }
    }

    pub(super) fn is_verified(&self, artifact_id: ArtifactId) -> bool {
        self.verified.borrow().contains(&artifact_id)
    }

    pub(super) fn mark_verified(&self, artifact_id: ArtifactId) {
        self.verified.borrow_mut().insert(artifact_id);
    }

    /// The artifact row, loading it once per request and prefetching every
    /// evidence row it owns (its chunks all follow in the visit order).
    pub(super) fn artifact(
        &self,
        artifacts: &dyn ArtifactRepository,
        evidence: &dyn EvidenceRepository,
        artifact_id: ArtifactId,
    ) -> Result<Option<Artifact>, PortError> {
        if let Some(artifact) = self.artifacts.borrow().get(&artifact_id) {
            return Ok(Some(artifact.clone()));
        }
        let Some(artifact) = artifacts.get(artifact_id)? else {
            return Ok(None);
        };
        for evidence in evidence.list_for_artifact(artifact_id)? {
            self.evidence.borrow_mut().insert(evidence.id, evidence);
        }
        self.artifacts
            .borrow_mut()
            .insert(artifact_id, artifact.clone());
        Ok(Some(artifact))
    }

    /// The evidence row, falling back to the exact-id read when the artifact
    /// prefetch cannot see it (e.g. a corrupted row owned by another
    /// artifact — exactly the case that must surface as a conflict).
    pub(super) fn evidence(
        &self,
        evidence_repo: &dyn EvidenceRepository,
        evidence_id: EvidenceId,
    ) -> Result<Option<Evidence>, PortError> {
        if let Some(evidence) = self.evidence.borrow().get(&evidence_id) {
            return Ok(Some(evidence.clone()));
        }
        let evidence = evidence_repo.get(evidence_id)?;
        if let Some(evidence) = evidence.as_ref() {
            self.evidence
                .borrow_mut()
                .insert(evidence.id, evidence.clone());
        }
        Ok(evidence)
    }
}
