#![forbid(unsafe_code)]

//! Deterministic domain kernel for Maestria.
//!
//! This module is pure and side-effect free. All environment interaction is
//! represented via `MaestriaEffect` values and executed by a runtime layer.

mod effects;
mod entities;
mod errors;
mod events;
mod evidence_pack;
mod evidence_source;
mod federated_evidence_bounds;
mod generations;
mod grant_token_digest;
mod ids;
mod input;
mod inputs;
mod kernel_state;
mod model_agent;
mod notebook;
mod notebook_inputs;
/// Responsibility map:
/// - `effects`: module responsibility.
/// - `evidence_source`: immutable text and snapshot evidence boundaries.
/// - `entities`: module responsibility.
/// - `errors`: module responsibility.
/// - `events`: module responsibility.
/// - `evidence_pack`: module responsibility.
/// - `federated_evidence_bounds`: validated finite limits for federated retrieval.
/// - `generations`: module responsibility.
/// - `ids`: module responsibility.
/// - `grant_token_digest`: domain-separated federation credential digests.
/// - `input`: module responsibility.
/// - `inputs`: module responsibility.
/// - `notebook_inputs`: notebook input command contracts.
/// - `model_agent`: model-agent proposal execution and result types.
/// - `notebook`: notebook identity, source selection, and draft revision contracts.
/// - `kernel_state`: module responsibility.
/// - `ocr`: module responsibility.
/// - `provenance`: module responsibility.
/// - `replay`: module responsibility.
/// - `realm_identity`: validated stable instance realm identities.
/// - `search`: module responsibility.
/// - `security`: module responsibility.
/// - `security_snapshot`: authorization and integrity security snapshots.
/// - `sparse_namespace`: learned-sparse instance and trust-zone identity.
/// - `task_status`: task status enum and transition policy.
/// - `types`: module responsibility.
mod ocr;
mod provenance;
mod realm_identity;
mod replay;
mod search;
mod security;
mod security_snapshot;
mod sparse_namespace;
mod task_status;
mod types;

pub use crate::effects::{
    DiagnosticEvent, FetchWebRequest, HarnessExecution, IndexFullTextRequest, IndexVectorRequest,
    KernelOutput, MaestriaEffect, NotebookDraftBlobRequest, OcrEffect, ParseArtifactRequest,
    ParseArtifactSource, QueryHarnessProposalRequest, QueryHarnessRequest, RequestApprovalRequest,
    RunValidationRequest, SearchKnowledgeRequest, UpdateGraphRequest, ValidationTarget,
};
pub use crate::entities::{
    Artifact, Card, Chunk, Claim, ClaimStatus, ContentRange, ContentRangeError, Evidence,
    FederatedAccessRecord, FederatedReadAccess, FederatedReadOperation, IndexStatus,
    MIN_PROMOTION_CONFIDENCE_MILLI, Memory, MemoryCandidate, MemoryStatus, OutputStream,
    PendingArtifact, RealmReadGrant, RealmReadGrantState, Relation, RelationEndpoint, RelationKind,
    Task, TaskPriority, TestStatus, ValidationReportRecord,
};
pub use crate::errors::DomainError;
pub use crate::events::{ApprovalOutcome, DomainEvent, DomainEventEnvelope};
pub use crate::evidence_pack::{
    ClaimCoverageStatusRecord, ClaimEvidenceCoverageRecord, EvidenceFreshnessRecord,
    EvidencePackCompressionRecord, EvidencePackMetadataRecord, EvidencePackReplayKeyRecord,
    EvidencePackReproducibilityRecord, SourceIndependenceRecord,
};
pub use crate::evidence_source::{
    EvidenceKind, LineRange, LineRangeError, SnapshotRef, SnapshotVerificationError,
    TextSnapshotVerificationError, WebEvidenceMetadata, verify_snapshot_bytes,
    verify_text_snapshot,
};
pub use crate::federated_evidence_bounds::{
    FederatedEvidenceBounds, FederatedEvidenceBoundsError, MAX_FEDERATED_EVIDENCE_BYTES,
    MAX_FEDERATED_RESULTS, MIN_FEDERATED_EVIDENCE_BYTES, MIN_FEDERATED_RESULTS,
};
pub use crate::generations::{
    FingerprintRevision, IndexFingerprint, IndexGeneration, IndexGenerationRegistry,
    IndexLifecycle, ModelName, PreprocessingVersion, ProviderName, QuantizationScheme,
    RepresentationName,
};
pub use crate::grant_token_digest::{GrantTokenDigest, GrantTokenDigestError};
pub use crate::ids::{
    ApprovalId, ArtifactId, ArtifactVersionId, BlobId, CardId, ChunkId, ClaimId, ConflictSetId,
    CorpusSnapshotId, CorrelationId, DEFAULT_CORPUS_SNAPSHOT_ID, DEFAULT_INSTANCE_SCOPE_ID,
    DOMAIN_VERSION, DuplicateClusterId, EventId, EvidenceId, HarnessRunId, IndexGenerationId,
    JournalGeneration, LogicalTick, MemoryCandidateId, MemoryId, NotebookDraftId, NotebookId,
    QueryId, RelationId, ScopeId, SearchTraceId, SequenceNumber, SnapshotId, StructureNodeId,
    TaskId, ValidationReportId,
};
pub use crate::inputs::{
    ApprovalDecision, ArtifactDetected, ChangeTaskStatusInput, CompleteTaskInput,
    ContradictMemoryInput, CreateCardInput, CreateClaimInput, CreateMemoryCandidateInput,
    CreateRelationInput, DeprecateMemoryInput, DomainInput, FetchWebRequested,
    FullTextIndexCompleted, HarnessRunCompleted, HarnessRunRequested, IssueRealmReadGrantInput,
    LinkEvidenceToClaimInput, LinkEvidenceToTaskInput, OcrCompleted, OcrFailed, OcrRequested,
    OpenTaskInput, ParserResult, ParserStarted, PromoteMemoryInput, ProposeMemoryCandidateInput,
    RecordEvidenceInput, RecordFederatedAccessInput, RecordValidationReportInput,
    RegisterArtifactInput, RegisterChunkInput, RequestTaskValidation, RevokeRealmReadGrantInput,
    SearchExecutedInput, SearchKnowledgeCompleted, SearchKnowledgeRequested, SearchResultSet,
    SourceRemoved, StartFullTextIndex, StartIndexGenerationInput, SupersedeMemoryInput,
    TransitionIndexGenerationInput, UserIntent, ValidationCompleted,
};
pub use crate::kernel_state::KernelState;
pub use crate::model_agent::{
    ModelAgentHarnessResult, ModelAgentMemoryDecision, ModelAgentMemoryResult,
    ModelAgentProposalExecution, ModelAgentProposalRequest, ModelAgentProposalResult,
    ModelAgentSearchResult, ModelAgentValidationResult,
};
pub use crate::notebook::{
    FrozenNotebookCitation, Notebook, NotebookDraft, NotebookDraftRevision, NotebookDraftTitle,
    NotebookTitle, NotebookValueError, SourceIdentityKey, validate_frozen_citations,
};
pub use crate::notebook_inputs::{
    AttachNotebookSourceInput, CreateNotebookInput, DeleteNotebookDraftInput, DeleteNotebookInput,
    DetachNotebookSourceInput, NotebookDraftBlobStoreFailed, NotebookDraftBlobStored,
    RenameNotebookInput, SaveNotebookDraftRequested,
};
pub use crate::ocr::{
    OcrCompletion, OcrDisclosure, OcrIntent, OcrPageText, OcrProviderIdentity, OcrRequestId,
    OcrRetentionPolicy, OcrValidationError,
};
pub use crate::provenance::{
    ParseStatus, ParsedRepresentation, RepresentationKind, SourceSpan, SourceSpanError,
    content_hash, evidence_id_for, excerpt_for, line_range_for_chunk, web_artifact_id_for,
    web_evidence_id_for,
};
pub use crate::realm_identity::{RealmId, RealmIdError};
pub use crate::replay::{replay_events, replay_inputs};
pub use crate::search::{
    ArtifactVersion, ConflictSet, ContentHash, CorpusScope, DiversityPlacement,
    DiversitySkipReason, EvidenceCandidate, EvidenceCandidateDto, EvidenceCoverage,
    EvidenceCoverageDto, EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus,
    LearnedSparseContribution, LearnedSparseReason, Modality, ModalitySet,
    RETRIEVAL_SCORE_SCHEMA_VERSION, RerankPosition, RetrievalLaneScore, RetrievalModelFingerprint,
    RetrievalRawRank, RetrievalReason, RetrievalScoreFingerprint, RetrievalScoreKind,
    RetrievalScoreScale, RetrievalScoreSet, SearchBudget, SearchBudgetLimits,
    SearchCompatibilityError, SearchDegradation, SearchExecution, SearchExecutionBudget,
    SearchExecutionCompletion, SearchExecutionResource, SearchExecutionUsage,
    SearchExpansionStrategy, SearchIntent, SearchLaneStatus, SearchOutcome, SearchPlan,
    SearchPlanBuilder, SearchRewriteAccounting, SearchRewriteOrigin, SearchRewriteStage,
    SearchRouteDecision, SearchStage, SearchStatus, SearchStopReason, SearchTrace,
    SearchTraceCandidate, SearchTraceCandidateDto, SearchTraceConstraintScore,
    SearchTraceDiversity, SearchTraceDiversityCandidate, SearchTraceExpansion, SearchTraceFilter,
    SearchTraceLane, SearchTraceLaneCandidate, SearchTraceLaneCandidateDto, SearchTraceRerank,
    SearchTraceRerankCandidate, SearchTraceRewrite, SourceLocation, StopConditions, StructureNode,
    StructureNodeType, TrustLabel, saturating_u32, saturating_u64, saturating_usize,
};
pub use crate::security::{
    Authority, IntegrityState, ReviewStatus, SecurityMetadata, Sensitivity, TrustZone,
};
pub use crate::security_snapshot::{RetrievalPolicySnapshot, RetrievalPolicySnapshotError};
pub use crate::sparse_namespace::{SparseNamespace, SparseNamespaceError};
pub use crate::task_status::TaskStatus;
