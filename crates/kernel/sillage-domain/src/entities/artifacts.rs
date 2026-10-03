use std::collections::BTreeSet;

use crate::ids::{ArtifactId, CardId, ChunkId, ClaimId, EvidenceId};
use crate::security::SecurityMetadata;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndexStatus {
    #[default]
    Unindexed,
    Pending,
    Indexed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub id: ArtifactId,
    pub title: String,
    pub chunk_ids: BTreeSet<ChunkId>,
    pub card_ids: BTreeSet<CardId>,
    pub claim_ids: BTreeSet<ClaimId>,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub index_status: IndexStatus,
    pub content_hash: Option<crate::search::ContentHash>,
    pub parse_status: Option<crate::provenance::ParseStatus>,
    pub security: SecurityMetadata,
}

impl Artifact {
    pub(crate) fn with_title(id: ArtifactId, title: String) -> Self {
        Self {
            id,
            title,
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::default(),
            content_hash: None,
            parse_status: None,
            security: SecurityMetadata::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingArtifact {
    pub artifact_id: ArtifactId,
    pub title: String,
    pub content_hash: crate::search::ContentHash,
}
