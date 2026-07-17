use serde::{Deserialize, Serialize};

use crate::{
    ConflictSet, CorpusSnapshotId, DuplicateClusterId, EvidenceId, EvidenceRequirements,
    FreshnessStatus, IndexGenerationId, QueryId, RetrievalModelFingerprint, SearchStopReason,
    SearchTraceId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidencePackMetadataRecord {
    pub query_id: QueryId,
    pub search_trace: Option<SearchTraceId>,
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub fingerprint: RetrievalModelFingerprint,
    pub policy_fingerprint: Option<String>,
    pub claims_required: Vec<String>,
    pub requirements: EvidenceRequirements,
    pub claim_coverage: Vec<ClaimEvidenceCoverageRecord>,
    pub source_independence: Vec<SourceIndependenceRecord>,
    pub card_count: usize,
    pub distinct_sources: usize,
    pub distinct_documents: usize,
    pub distinct_sections: usize,
    pub primary_sources_verified: bool,
    pub freshness: Vec<EvidenceFreshnessRecord>,
    pub conflicts: Vec<ConflictSet>,
    pub counterevidence: Vec<EvidenceId>,
    pub missing_evidence: Vec<String>,
    pub compression: EvidencePackCompressionRecord,
    pub stop_reason: SearchStopReason,
    pub reproducibility: EvidencePackReproducibilityRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimEvidenceCoverageRecord {
    pub claim: String,
    pub evidence_ids: Vec<EvidenceId>,
    pub status: ClaimCoverageStatusRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimCoverageStatusRecord {
    Supported,
    Partial,
    Missing,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIndependenceRecord {
    pub source_key: String,
    pub evidence_ids: Vec<EvidenceId>,
    pub duplicate_cluster: Option<DuplicateClusterId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceFreshnessRecord {
    pub evidence_id: EvidenceId,
    pub status: FreshnessStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidencePackCompressionRecord {
    Verbatim {
        evidence_ids: Vec<EvidenceId>,
    },
    Compressed {
        source_evidence_ids: Vec<EvidenceId>,
        retained_evidence_ids: Vec<EvidenceId>,
        selector: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidencePackReplayKeyRecord {
    pub trace: SearchTraceId,
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub fingerprint: RetrievalModelFingerprint,
    pub policy_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidencePackReproducibilityRecord {
    Frozen(EvidencePackReplayKeyRecord),
    LiveNonReproducible { reason: String },
}
