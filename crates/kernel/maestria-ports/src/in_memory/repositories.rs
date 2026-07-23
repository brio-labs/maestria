use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::PortError;
use maestria_domain::{ApprovalId, TaskId};
use maestria_domain::{Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, Evidence, EvidenceId};

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
        let guard = self.artifacts.lock().map_err(|_| PortError::Internal {
            message: "artifact store lock poisoned".to_string(),
        })?;
        Ok(guard.get(&artifact_id).cloned())
    }

    fn put(&self, artifact: Artifact) -> Result<(), PortError> {
        let mut guard = self.artifacts.lock().map_err(|_| PortError::Internal {
            message: "artifact store lock poisoned".to_string(),
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
        let guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "chunk store lock poisoned".to_string(),
        })?;
        Ok(guard.get(&chunk_id).cloned())
    }

    fn put(&self, chunk: Chunk) -> Result<(), PortError> {
        let mut guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "chunk store lock poisoned".to_string(),
        })?;
        guard.insert(chunk.id, chunk);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError> {
        let guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "chunk store lock poisoned".to_string(),
        })?;
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
        let guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "card store lock poisoned".to_string(),
        })?;
        Ok(guard.get(&card_id).cloned())
    }

    fn put(&self, card: Card) -> Result<(), PortError> {
        let mut guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "card store lock poisoned".to_string(),
        })?;
        guard.insert(card.id, card);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Card>, PortError> {
        let guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "card store lock poisoned".to_string(),
        })?;
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
        let guard = self.evidences.lock().map_err(|_| PortError::Internal {
            message: "evidence store lock poisoned".to_string(),
        })?;
        Ok(guard.get(&evidence_id).cloned())
    }

    fn put(&self, evidence: Evidence) -> Result<(), PortError> {
        let mut guard = self.evidences.lock().map_err(|_| PortError::Internal {
            message: "evidence store lock poisoned".to_string(),
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
        let mut guard = self.evidences.lock().map_err(|_| PortError::Internal {
            message: "evidence store lock poisoned".to_string(),
        })?;
        guard.insert(evidence.id, evidence);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError> {
        let guard = self.evidences.lock().map_err(|_| PortError::Internal {
            message: "evidence store lock poisoned".to_string(),
        })?;
        Ok(guard
            .values()
            .filter(|evidence| evidence.artifact_id == artifact_id)
            .cloned()
            .collect())
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
            .map_err(|_| crate::PortError::Internal {
                message: "in-memory approval repo lock poisoned".to_string(),
            })?;
        guard.insert(record.id, record.clone());
        Ok(())
    }

    fn find_pending(&self) -> Result<Vec<crate::ApprovalRecord>, crate::PortError> {
        let guard = self
            .records
            .lock()
            .map_err(|_| crate::PortError::Internal {
                message: "in-memory approval repo lock poisoned".to_string(),
            })?;
        Ok(guard
            .values()
            .filter(|r| r.status == crate::ApprovalStatus::Pending)
            .cloned()
            .collect())
    }

    fn find_by_id(
        &self,
        id: ApprovalId,
    ) -> Result<Option<crate::ApprovalRecord>, crate::PortError> {
        let guard = self
            .records
            .lock()
            .map_err(|_| crate::PortError::Internal {
                message: "in-memory approval repo lock poisoned".to_string(),
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
            .map_err(|_| crate::PortError::Internal {
                message: "in-memory approval repo lock poisoned".to_string(),
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
            .map_err(|_| crate::PortError::Internal {
                message: "in-memory approval repo lock poisoned".to_string(),
            })?;
        Ok(guard
            .values()
            .filter(|r| r.task_id == task_id)
            .cloned()
            .collect())
    }
}
