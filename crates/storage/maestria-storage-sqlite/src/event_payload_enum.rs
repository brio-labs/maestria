use super::super::evidence_payloads::{StoredClaimStatus, StoredEvidenceKind};
use super::super::notebook_event_payloads::StoredNotebookCitation;
use super::super::ocr_event_payloads::StoredOcrPage;
use super::super::relation_payloads::{StoredRelationEndpoint, StoredRelationKind};
use super::super::stored_content::StoredContentHash;
use super::super::stored_evidence_pack::StoredEvidencePackMetadataRecord;
use super::super::stored_generations::{
    StoredIndexFingerprint, StoredIndexLifecycle, StoredRepresentationName,
};
use super::super::stored_model_agent::{
    StoredModelAgentProposalRequest, StoredModelAgentProposalResult,
};
use super::super::stored_search::{StoredSearchOutcome, StoredSearchPlan};
use super::super::stored_security::StoredSecurityMetadata;
use super::super::stored_structure::StoredStructureNode;
use super::super::task_event_payloads::{StoredTaskPriority, StoredTaskStatus};
use super::StoredApprovalOutcome;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event_kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredEventPayload {
    ArtifactRegistered {
        artifact_id: u64,
        title: String,
        security: StoredSecurityMetadata,
    },
    ChunkRegistered {
        chunk_id: u64,
        artifact_id: u64,
        node_id: u64,
        source_span: crate::payloads::StoredSourceSpan,
        /// Current writes store the kinds with empty contents — `raw` /
        /// `retrieval` mirror the chunk text, so full contents tripled
        /// every payload. Legacy rows carry real contents; both decode.
        representations: Vec<super::super::provenance_payloads::StoredParsedRepresentation>,
        #[serde(default)]
        representations_digest: String,
        order: u32,
        text: String,
    },
    CardCreated {
        card_id: u64,
        artifact_id: u64,
        node_id: u64,
        source_span: crate::payloads::StoredSourceSpan,
        title: String,
        body: String,
        security: StoredSecurityMetadata,
    },
    ClaimCreated {
        claim_id: u64,
        artifact_id: u64,
        text: String,
        evidence_ids: Vec<u64>,
        security: StoredSecurityMetadata,
    },
    EvidenceRecorded {
        evidence_id: u64,
        artifact_id: u64,
        claim_id: Option<u64>,
        evidence_kind: StoredEvidenceKind,
        excerpt: String,
        observed_at: u64,
        security: StoredSecurityMetadata,
    },
    TaskOpened {
        task_id: u64,
        title: String,
        priority: StoredTaskPriority,
        artifact_id: Option<u64>,
    },
    TaskStatusChanged {
        task_id: u64,
        from: StoredTaskStatus,
        to: StoredTaskStatus,
    },
    TaskCompletionRecorded {
        task_id: u64,
        status: StoredTaskStatus,
    },
    TaskEvidenceLinked {
        task_id: u64,
        evidence_id: u64,
    },
    ClaimValidationUpdated {
        claim_id: u64,
        status: StoredClaimStatus,
    },
    ClaimEvidenceLinked {
        claim_id: u64,
        evidence_id: u64,
    },
    RelationCreated {
        relation_id: u64,
        source: StoredRelationEndpoint,
        kind: StoredRelationKind,
        target: StoredRelationEndpoint,
        evidence_id: Option<u64>,
        confidence_milli: u16,
        security: StoredSecurityMetadata,
    },
    MemoryCandidateCreated {
        candidate_id: u64,
        claim_id: u64,
        evidence_ids: Vec<u64>,
        confidence_milli: u16,
        security: StoredSecurityMetadata,
    },
    MemoryPromoted {
        memory_id: u64,
        candidate_id: u64,
        security: StoredSecurityMetadata,
    },
    MemoryContradicted {
        memory_id: u64,
        contradicting_candidate_id: u64,
    },
    MemoryDeprecated {
        memory_id: u64,
    },
    MemorySuperseded {
        memory_id: u64,
        by_memory_id: u64,
    },
    ValidationReportCreated {
        report_id: u64,
        task_id: Option<u64>,
        passed: bool,
        warnings: Vec<String>,
    },
    ArtifactParsed {
        artifact_id: u64,
        status: crate::payloads::StoredParseStatus,
    },
    DocumentTreeCaptured {
        artifact_id: u64,
        artifact_version_id: u64,
        content_hash: StoredContentHash,
        root_id: u64,
        nodes: Vec<StoredStructureNode>,
    },
    SearchCompleted {
        artifact_id: u64,
    },
    HarnessRunCompleted {
        task_id: Option<u64>,
        command: String,
        exit_code: i32,
    },
    ModelAgentProposalRequested {
        request: StoredModelAgentProposalRequest,
    },
    ModelAgentProposalCompleted {
        result: StoredModelAgentProposalResult,
    },
    ApprovalRecorded {
        approval_id: u64,
        outcome: StoredApprovalOutcome,
    },
    TickObserved {
        at: u64,
    },
    SearchExecuted {
        query: String,
        limit: u64,
        evidence_ids: Vec<u64>,
        pack_metadata: Option<Box<StoredEvidencePackMetadataRecord>>,
        at: u64,
    },
    SearchKnowledgeCompleted {
        task_id: Option<u64>,
        plan: Option<Box<StoredSearchPlan>>,
        outcome: StoredSearchOutcome,
    },
    PendingIndex {
        artifact_id: u64,
        content_hash: StoredContentHash,
    },
    FullTextIndexed {
        artifact_id: u64,
        chunk_id: u64,
    },
    ArtifactIndexed {
        artifact_id: u64,
    },
    ParserStarted {
        artifact_id: u64,
        title: String,
        source_path: String,
        content_hash: StoredContentHash,
        blob_id: u64,
    },
    OcrRequested {
        request_id: String,
        artifact_id: u64,
        source_blob: u64,
        source_hash: StoredContentHash,
        pages: Vec<u32>,
        provider: String,
        model: String,
        revision: String,
        provider_artifact_hash: String,
        preprocessing_version: String,
        remote: bool,
        retention: String,
    },
    OcrCompleted {
        artifact_id: u64,
        request_id: String,
        pages: Vec<StoredOcrPage>,
    },
    OcrFailed {
        artifact_id: u64,
        request_id: String,
        reason: String,
    },
    IndexGenerationStarted {
        id: u64,
        name: StoredRepresentationName,
        corpus_snapshot: u64,
        fingerprint: StoredIndexFingerprint,
        #[serde(default)]
        sparse_namespace: Option<maestria_domain::SparseNamespace>,
    },
    IndexGenerationTransitioned {
        id: u64,
        from: StoredIndexLifecycle,
        to: StoredIndexLifecycle,
        replaced_active_id: Option<u64>,
    },
    RealmReadGrantIssued {
        token_digest: String,
        provider_realm: String,
        consumer_realm: String,
        access: crate::payloads::realm_read_grant_event_payloads::StoredFederatedReadAccess,
        max_sensitivity:
            crate::payloads::realm_read_grant_event_payloads::StoredFederatedSensitivity,
        max_results: u64,
        max_evidence_bytes: u64,
    },
    RealmReadGrantRevoked {
        token_digest: String,
    },
    FederatedReadAccessRecorded {
        token_digest: String,
        provider_realm: String,
        consumer_realm: String,
        record: crate::payloads::realm_read_grant_event_payloads::StoredFederatedAccessRecord,
    },
    NotebookCreated {
        notebook_id: u64,
        title: String,
        created_at: u64,
        updated_at: u64,
    },
    NotebookRenamed {
        notebook_id: u64,
        title: String,
        updated_at: u64,
    },
    NotebookDeleted {
        notebook_id: u64,
    },
    NotebookSourceAttached {
        notebook_id: u64,
        source_key: String,
        updated_at: u64,
    },
    NotebookSourceDetached {
        notebook_id: u64,
        source_key: String,
        updated_at: u64,
    },
    NotebookDraftSaved {
        draft_id: u64,
        notebook_id: u64,
        title: String,
        body_blob: u64,
        body_hash: StoredContentHash,
        revision: u64,
        citations: Vec<StoredNotebookCitation>,
        created_at: u64,
        updated_at: u64,
    },
    NotebookDraftDeleted {
        notebook_id: u64,
        draft_id: u64,
        revision: u64,
    },
    SourceBecameStale {
        artifact_id: u64,
        source_path: String,
        content_hash: StoredContentHash,
    },
}
