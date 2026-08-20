use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use super::store::lock_map;
use crate::PortError;
use maestria_domain::{ApprovalId, TaskId};
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, Evidence, EvidenceId, GrantTokenDigest,
    RealmReadGrant,
};
// ── ArtifactRepository ──────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct InMemoryArtifactRepository {
    artifacts: Arc<Mutex<BTreeMap<ArtifactId, Artifact>>>,
}

impl InMemoryArtifactRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::ArtifactRepository for InMemoryArtifactRepository {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError> {
        let guard = self
            .artifacts
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "artifact repository lock poisoned",
                source: "artifact repository mutex is poisoned".to_string(),
            })?;
        Ok(guard.get(&artifact_id).cloned())
    }

    fn put(&self, artifact: Artifact) -> Result<(), PortError> {
        let mut guard = self
            .artifacts
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "artifact repository lock poisoned",
                source: "artifact repository mutex is poisoned".to_string(),
            })?;
        guard.insert(artifact.id, artifact);
        Ok(())
    }
}

// ── ChunkRepository ─────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct InMemoryChunkRepository {
    chunks: Arc<Mutex<BTreeMap<ChunkId, Chunk>>>,
}

impl InMemoryChunkRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::ChunkRepository for InMemoryChunkRepository {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError> {
        let guard = lock_map(&self.chunks, "chunk repository lock poisoned")?;
        Ok(guard.get(&chunk_id).cloned())
    }
    fn find_artifact_id(&self, chunk_id: ChunkId) -> Result<Option<ArtifactId>, PortError> {
        let guard = lock_map(&self.chunks, "chunk repository lock poisoned")?;
        Ok(guard.get(&chunk_id).map(|chunk| chunk.artifact_id))
    }

    fn put(&self, chunk: Chunk) -> Result<(), PortError> {
        let mut guard = lock_map(&self.chunks, "chunk repository lock poisoned")?;
        guard.insert(chunk.id, chunk);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError> {
        let guard = lock_map(&self.chunks, "chunk repository lock poisoned")?;
        let mut chunks = guard
            .values()
            .filter(|chunk| chunk.artifact_id == artifact_id)
            .cloned()
            .collect::<Vec<_>>();
        chunks.sort_by_key(|chunk| (chunk.order, chunk.id));
        Ok(chunks)
    }
}

// ── CardRepository ──────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct InMemoryCardRepository {
    cards: Arc<Mutex<BTreeMap<CardId, Card>>>,
}

impl InMemoryCardRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::CardRepository for InMemoryCardRepository {
    fn get(&self, card_id: CardId) -> Result<Option<Card>, PortError> {
        let guard = lock_map(&self.cards, "card repository lock poisoned")?;
        Ok(guard.get(&card_id).cloned())
    }

    fn put(&self, card: Card) -> Result<(), PortError> {
        let mut guard = lock_map(&self.cards, "card repository lock poisoned")?;
        guard.insert(card.id, card);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Card>, PortError> {
        let guard = lock_map(&self.cards, "card repository lock poisoned")?;
        Ok(guard
            .values()
            .filter(|card| card.artifact_id == artifact_id)
            .cloned()
            .collect())
    }
}

// ── EvidenceRepository ──────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct InMemoryEvidenceRepository {
    evidences: Arc<Mutex<BTreeMap<EvidenceId, Evidence>>>,
}

impl InMemoryEvidenceRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::EvidenceRepository for InMemoryEvidenceRepository {
    fn get(&self, evidence_id: EvidenceId) -> Result<Option<Evidence>, PortError> {
        let guard = self
            .evidences
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "evidence repository lock poisoned",
                source: "evidence repository mutex is poisoned".to_string(),
            })?;
        Ok(guard.get(&evidence_id).cloned())
    }

    fn put(&self, evidence: Evidence) -> Result<(), PortError> {
        let mut guard = self
            .evidences
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "evidence repository lock poisoned",
                source: "evidence repository mutex is poisoned".to_string(),
            })?;
        if let Some(existing) = guard.get(&evidence.id) {
            if existing == &evidence {
                return Ok(());
            }
            return Err(PortError::Conflict {
                message: format!(
                    "evidence {} already exists with different content; evidence is immutable",
                    evidence.id.value()
                ),
            });
        }
        guard.insert(evidence.id, evidence);
        Ok(())
    }

    fn replace(&self, evidence: Evidence) -> Result<(), PortError> {
        let mut guard = self
            .evidences
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "evidence repository lock poisoned",
                source: "evidence repository mutex is poisoned".to_string(),
            })?;
        guard.insert(evidence.id, evidence);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError> {
        let guard = self
            .evidences
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "evidence repository lock poisoned",
                source: "evidence repository mutex is poisoned".to_string(),
            })?;
        Ok(guard
            .values()
            .filter(|evidence| evidence.artifact_id == artifact_id)
            .cloned()
            .collect())
    }
}

// ── RealmReadGrantRepository ─────────────────────────────────────────

#[derive(Clone, Default)]
pub struct InMemoryRealmReadGrantRepository {
    grants: Arc<Mutex<BTreeMap<GrantTokenDigest, RealmReadGrant>>>,
}

impl InMemoryRealmReadGrantRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::RealmReadGrantRepository for InMemoryRealmReadGrantRepository {
    fn get(&self, token_digest: &GrantTokenDigest) -> Result<Option<RealmReadGrant>, PortError> {
        let grants = lock_map(&self.grants, "realm read grant repository lock poisoned")?;
        Ok(grants.get(token_digest).cloned())
    }

    fn put(&self, grant: RealmReadGrant) -> Result<(), PortError> {
        let mut grants = lock_map(&self.grants, "realm read grant repository lock poisoned")?;
        if grant.state() == maestria_domain::RealmReadGrantState::Active
            && grants.iter().any(|(digest, existing)| {
                digest != grant.token_digest()
                    && existing.state() == maestria_domain::RealmReadGrantState::Active
                    && existing.consumer_realm() == grant.consumer_realm()
            })
        {
            return Err(PortError::Conflict {
                message: "consumer realm already has an active read grant".to_string(),
            });
        }
        grants.insert(grant.token_digest().clone(), grant);
        Ok(())
    }

    fn list(&self) -> Result<Vec<RealmReadGrant>, PortError> {
        let grants = lock_map(&self.grants, "realm read grant repository lock poisoned")?;
        Ok(grants.values().cloned().collect())
    }

    fn delete_not_in(&self, token_digests: &BTreeSet<GrantTokenDigest>) -> Result<(), PortError> {
        let mut grants = lock_map(&self.grants, "realm read grant repository lock poisoned")?;
        grants.retain(|digest, _| token_digests.contains(digest));
        Ok(())
    }
}

// ── ApprovalRepository ───────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct InMemoryApprovalRepository {
    records: Arc<Mutex<BTreeMap<ApprovalId, crate::ApprovalRecord>>>,
}

impl InMemoryApprovalRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::ApprovalRepository for InMemoryApprovalRepository {
    fn save(&self, record: &crate::ApprovalRecord) -> Result<(), crate::PortError> {
        let mut guard = self
            .records
            .lock()
            .map_err(|_| crate::PortError::InternalContext {
                context: "approval repository lock poisoned",
                source: "approval repository mutex is poisoned".to_string(),
            })?;
        guard.insert(record.id, record.clone());
        Ok(())
    }

    fn find_pending(&self) -> Result<Vec<crate::ApprovalRecord>, crate::PortError> {
        let guard = self
            .records
            .lock()
            .map_err(|_| crate::PortError::InternalContext {
                context: "approval repository lock poisoned",
                source: "approval repository mutex is poisoned".to_string(),
            })?;
        Ok(guard
            .values()
            .filter(|r| r.status == crate::ApprovalStatus::Pending)
            .cloned()
            .collect())
    }
    fn find_all(&self) -> Result<Vec<crate::ApprovalRecord>, crate::PortError> {
        let guard = self
            .records
            .lock()
            .map_err(|_| crate::PortError::InternalContext {
                context: "approval repository lock poisoned",
                source: "approval repository mutex is poisoned".to_string(),
            })?;
        Ok(guard.values().cloned().collect())
    }

    fn find_by_id(
        &self,
        id: ApprovalId,
    ) -> Result<Option<crate::ApprovalRecord>, crate::PortError> {
        let guard = self
            .records
            .lock()
            .map_err(|_| crate::PortError::InternalContext {
                context: "approval repository lock poisoned",
                source: "approval repository mutex is poisoned".to_string(),
            })?;
        Ok(guard.get(&id).cloned())
    }

    fn resolve(
        &self,
        id: ApprovalId,
        approved: bool,
    ) -> Result<Option<crate::ApprovalRecord>, crate::PortError> {
        let mut guard = self
            .records
            .lock()
            .map_err(|_| crate::PortError::InternalContext {
                context: "approval repository lock poisoned",
                source: "approval repository mutex is poisoned".to_string(),
            })?;
        if let Some(record) = guard.get_mut(&id)
            && record.status == crate::ApprovalStatus::Pending
        {
            record.status = if approved {
                crate::ApprovalStatus::Approved
            } else {
                crate::ApprovalStatus::Denied
            };
            return Ok(Some(record.clone()));
        }
        Ok(None)
    }

    fn find_by_task_id(
        &self,
        task_id: TaskId,
    ) -> Result<Vec<crate::ApprovalRecord>, crate::PortError> {
        let guard = self
            .records
            .lock()
            .map_err(|_| crate::PortError::InternalContext {
                context: "approval repository lock poisoned",
                source: "approval repository mutex is poisoned".to_string(),
            })?;
        Ok(guard
            .values()
            .filter(|r| r.task_id == Some(task_id))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn satisfies_shared_approval_repository_contract() -> Result<(), Box<dyn std::error::Error>> {
        crate::contract_tests::assert_approval_repository_contract(
            &InMemoryApprovalRepository::new(),
        )
    }
}
