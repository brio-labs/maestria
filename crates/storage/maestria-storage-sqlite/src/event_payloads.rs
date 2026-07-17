use super::evidence_payloads::{
    StoredClaimStatus, StoredEvidenceKind, StoredTaskPriority, StoredTaskStatus,
};
use super::relation_payloads::{StoredRelationEndpoint, StoredRelationKind};
use maestria_domain::DomainEvent;
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event_kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredEventPayload {
    ArtifactRegistered {
        artifact_id: u64,
        title: String,
        #[serde(default)]
        security: maestria_domain::SecurityMetadata,
    },
    ChunkRegistered {
        chunk_id: u64,
        artifact_id: u64,
        #[serde(default)]
        node_id: u64,
        #[serde(default)]
        source_span: crate::payloads::StoredSourceSpan,
        #[serde(default)]
        representations: Vec<crate::payloads::StoredParsedRepresentation>,
        order: u32,
        text: String,
    },
    CardCreated {
        card_id: u64,
        artifact_id: u64,
        #[serde(default)]
        node_id: u64,
        #[serde(default)]
        source_span: crate::payloads::StoredSourceSpan,
        title: String,
        body: String,
        #[serde(default)]
        security: maestria_domain::SecurityMetadata,
    },
    ClaimCreated {
        claim_id: u64,
        artifact_id: u64,
        text: String,
        evidence_ids: Vec<u64>,
        #[serde(default)]
        security: maestria_domain::SecurityMetadata,
    },
    EvidenceRecorded {
        evidence_id: u64,
        artifact_id: u64,
        claim_id: Option<u64>,
        evidence_kind: StoredEvidenceKind,
        excerpt: String,
        observed_at: u64,
        #[serde(default)]
        security: maestria_domain::SecurityMetadata,
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
        #[serde(default)]
        security: maestria_domain::SecurityMetadata,
    },
    MemoryCandidateCreated {
        candidate_id: u64,
        claim_id: u64,
        evidence_ids: Vec<u64>,
        confidence_milli: u16,
        #[serde(default)]
        security: maestria_domain::SecurityMetadata,
    },
    MemoryPromoted {
        memory_id: u64,
        candidate_id: u64,
        #[serde(default)]
        security: maestria_domain::SecurityMetadata,
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
        #[serde(default = "crate::payloads::default_status_parsed")]
        status: crate::payloads::StoredParseStatus,
        chunks_added: u32,
    },
    DocumentTreeCaptured {
        artifact_id: u64,
        artifact_version_id: u64,
        content_hash: maestria_domain::ContentHash,
        root_id: u64,
        nodes: Vec<maestria_domain::StructureNode>,
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
    ApprovalRecorded {
        approval_id: u64,
        task_id: u64,
        approved: bool,
        from_status: Option<StoredTaskStatus>,
        to_status: Option<StoredTaskStatus>,
    },
    TickObserved {
        at: u64,
    },
    SearchExecuted {
        query: String,
        limit: u64,
        evidence_ids: Vec<u64>,
        #[serde(default)]
        pack_metadata: Option<Box<maestria_domain::EvidencePackMetadataRecord>>,
        at: u64,
    },
    SearchKnowledgeCompleted {
        #[serde(default)]
        task_id: Option<u64>,
        #[serde(default)]
        plan: Option<Box<maestria_domain::SearchPlan>>,
        outcome: maestria_domain::SearchOutcome,
    },
    PendingIndex {
        artifact_id: u64,
        content_hash: String,
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
        content_hash: String,
        blob_id: u64,
    },
    IndexGenerationStarted {
        id: u64,
        name: maestria_domain::RepresentationName,
        corpus_snapshot: u64,
        fingerprint: maestria_domain::IndexFingerprint,
    },
    IndexGenerationTransitioned {
        id: u64,
        from: maestria_domain::IndexLifecycle,
        to: maestria_domain::IndexLifecycle,
        replaced_active_id: Option<u64>,
    },
}

impl StoredEventPayload {
    pub(crate) fn from_domain(event: &DomainEvent) -> Result<Self, PortError> {
        Self::try_from_domain_artifact(event)
            .or_else(|| Self::try_from_domain_task(event))
            .or_else(|| Self::try_from_domain_claim(event))
            .or_else(|| Self::try_from_domain_memory(event))
            .or_else(|| Self::try_from_domain_misc(event))
            .ok_or_else(|| PortError::Internal {
                message: "unknown DomainEvent variant".to_string(),
            })
    }

    pub(crate) fn into_domain(self) -> Result<DomainEvent, PortError> {
        self.try_into_domain_artifact()
            .or_else(|s| (*s).try_into_domain_task())
            .or_else(|s| (*s).try_into_domain_claim())
            .or_else(|s| (*s).try_into_domain_memory())
            .or_else(|s| (*s).try_into_domain_misc())
            .map_err(|_| PortError::Internal {
                message: "unknown StoredEventPayload variant".to_string(),
            })
    }

    pub(crate) fn kind(&self) -> Result<&'static str, PortError> {
        self.try_kind_artifact()
            .or_else(|| self.try_kind_task())
            .or_else(|| self.try_kind_claim())
            .or_else(|| self.try_kind_memory())
            .or_else(|| self.try_kind_misc())
            .ok_or_else(|| PortError::Internal {
                message: "unknown StoredEventPayload variant".to_string(),
            })
    }

    pub(crate) fn filter_artifact_id(&self) -> Option<u64> {
        self.try_filter_artifact_id_artifact()
            .or_else(|| self.try_filter_artifact_id_task())
            .or_else(|| self.try_filter_artifact_id_claim())
            .or_else(|| self.try_filter_artifact_id_memory())
            .or_else(|| self.try_filter_artifact_id_misc())
    }
}
