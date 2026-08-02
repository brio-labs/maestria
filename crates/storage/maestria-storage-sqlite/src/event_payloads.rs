use super::evidence_payloads::{
    StoredClaimStatus, StoredEvidenceKind, StoredTaskPriority, StoredTaskStatus,
};
use super::ocr_event_payloads::StoredOcrPage;
use super::relation_payloads::{StoredRelationEndpoint, StoredRelationKind};
use super::stored_content::StoredContentHash;
use super::stored_evidence_pack::StoredEvidencePackMetadataRecord;
use super::stored_generations::{
    StoredIndexFingerprint, StoredIndexLifecycle, StoredRepresentationName,
};
use super::stored_model_agent::{StoredModelAgentProposalRequest, StoredModelAgentProposalResult};
use super::stored_search::{StoredSearchOutcome, StoredSearchPlan};
use super::stored_security::StoredSecurityMetadata;
use super::stored_structure::StoredStructureNode;
use maestria_domain::DomainEvent;
use maestria_ports::PortError;
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
        representations: Vec<crate::payloads::StoredParsedRepresentation>,
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
        validation_report_id: u64,
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
    UserIntentObserved {
        task_id: u64,
        title: String,
    },
    ArtifactParsed {
        artifact_id: u64,
        status: crate::payloads::StoredParseStatus,
        chunks_added: u32,
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
        cards_added: u32,
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
    },
    IndexGenerationTransitioned {
        id: u64,
        from: StoredIndexLifecycle,
        to: StoredIndexLifecycle,
        replaced_active_id: Option<u64>,
    },
    SourceBecameStale {
        artifact_id: u64,
        source_path: String,
        content_hash: StoredContentHash,
    },
}

impl StoredEventPayload {
    pub(crate) fn from_domain(event: &DomainEvent) -> Result<Self, PortError> {
        Self::try_from_domain_stale(event)
            .or_else(|| Self::try_from_domain_ocr(event))
            .or_else(|| Self::try_from_domain_artifact(event))
            .or_else(|| Self::try_from_domain_task(event))
            .or_else(|| Self::try_from_domain_claim(event))
            .or_else(|| Self::try_from_domain_memory(event))
            .or_else(|| Self::try_from_domain_misc(event))
            .ok_or_else(|| PortError::InternalContext {
                context: "encode domain event",
                source: "unknown DomainEvent variant".to_string(),
            })
    }

    pub(crate) fn into_domain(self) -> Result<DomainEvent, PortError> {
        if matches!(&self, Self::EvidenceRecorded { .. }) {
            return self.try_into_domain_evidence();
        }
        self.try_into_domain_stale()
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_ocr))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_artifact))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_task))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_claim))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_memory))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_misc))
            .map_err(|error| match error {
                FamilyDecodeError::Foreign(_) => PortError::InternalContext {
                    context: "decode stored event payload",
                    source: "unknown StoredEventPayload variant".to_string(),
                },
                FamilyDecodeError::Invalid(error) => error,
            })
    }

    pub(crate) fn kind(&self) -> Result<&'static str, PortError> {
        self.try_kind_stale()
            .or_else(|| self.try_kind_ocr())
            .or_else(|| self.try_kind_artifact())
            .or_else(|| self.try_kind_task())
            .or_else(|| self.try_kind_claim())
            .or_else(|| self.try_kind_memory())
            .or_else(|| self.try_kind_misc())
            .ok_or_else(|| PortError::InternalContext {
                context: "identify stored event payload kind",
                source: "unknown StoredEventPayload variant".to_string(),
            })
    }

    pub(crate) fn filter_artifact_id(&self) -> Option<u64> {
        self.try_filter_artifact_id_stale()
            .or_else(|| self.try_filter_artifact_id_ocr())
            .or_else(|| self.try_filter_artifact_id_artifact())
            .or_else(|| self.try_filter_artifact_id_task())
            .or_else(|| self.try_filter_artifact_id_claim())
            .or_else(|| self.try_filter_artifact_id_memory())
            .or_else(|| self.try_filter_artifact_id_misc())
    }

    fn try_from_domain_stale(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::SourceBecameStale {
                artifact_id,
                source_path,
                content_hash,
            } => Some(Self::SourceBecameStale {
                artifact_id: artifact_id.value(),
                source_path: source_path.clone(),
                content_hash: StoredContentHash::from_domain(content_hash),
            }),
            _ => None,
        }
    }

    fn try_into_domain_stale(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            Self::SourceBecameStale {
                artifact_id,
                source_path,
                content_hash,
            } => Ok(DomainEvent::SourceBecameStale {
                artifact_id: maestria_domain::ArtifactId::new(artifact_id),
                source_path,
                content_hash: content_hash
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
        }
    }

    fn try_kind_stale(&self) -> Option<&'static str> {
        match self {
            Self::SourceBecameStale { .. } => Some("source_became_stale"),
            _ => None,
        }
    }

    fn try_filter_artifact_id_stale(&self) -> Option<u64> {
        match self {
            Self::SourceBecameStale { artifact_id, .. } => Some(*artifact_id),
            _ => None,
        }
    }
}

/// Decode failure from a stored-payload family converter.
///
/// `Foreign` means the variant does not belong to that family and the next
/// family should try it; `Invalid` carries a real decode error that must
/// surface instead of being treated as an unknown variant.
#[derive(Debug)]
pub(crate) enum FamilyDecodeError {
    Foreign(Box<StoredEventPayload>),
    Invalid(PortError),
}

impl FamilyDecodeError {
    /// Hand the payload to the next family unless a real decode error occurred.
    fn or_next(
        self,
        family: fn(StoredEventPayload) -> Result<DomainEvent, FamilyDecodeError>,
    ) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            FamilyDecodeError::Foreign(payload) => family(*payload),
            other => Err(other),
        }
    }
}

/// v3 stored encoding of `maestria_domain::ApprovalOutcome`.
///
/// Statuses use `StoredTaskStatus` because the domain `TaskStatus` carries no
/// serde representation; the variant structure mirrors the domain outcome so
/// replay cannot observe a coordinated-flags state.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredApprovalOutcome {
    Acknowledged {
        task_id: Option<u64>,
        approved: bool,
    },
    TaskTransition {
        task_id: u64,
        approved: bool,
        from_status: StoredTaskStatus,
        to_status: StoredTaskStatus,
    },
}
