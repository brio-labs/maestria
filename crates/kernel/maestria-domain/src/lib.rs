#![forbid(unsafe_code)]

//! Deterministic domain kernel for Maestria.
//!
//! This module is pure and side-effect free. All environment interaction is
//! represented via `MaestriaEffect` values and executed by a runtime layer.

/// Responsibility map:
/// - `effects`: module responsibility.
/// - `entities`: module responsibility.
/// - `errors`: module responsibility.
/// - `events`: module responsibility.
/// - `evidence_pack`: module responsibility.
/// - `generations`: module responsibility.
/// - `ids`: module responsibility.
/// - `input`: module responsibility.
/// - `inputs`: module responsibility.
/// - `kernel_state`: module responsibility.
/// - `provenance`: module responsibility.
/// - `replay`: module responsibility.
/// - `search`: module responsibility.
/// - `security`: module responsibility.
/// - `security_snapshot`: authorization and integrity security snapshots.
/// - `types`: module responsibility.
mod effects;
mod entities;
mod errors;
mod events;
mod evidence_pack;
mod generations;
mod ids;
mod input;
mod inputs;
mod kernel_state;
mod provenance;
mod replay;
mod search;
mod security;
mod security_snapshot;
mod types;

// Public API — explicit stable boundary exports.
pub use crate::effects::{
    DiagnosticEvent, FetchWebRequest, IndexFullTextRequest, IndexVectorRequest, KernelOutput,
    MaestriaEffect, ParseArtifactRequest, QueryHarnessProposalRequest, QueryHarnessRequest,
    RequestApprovalRequest, RunValidationRequest, SearchKnowledgeRequest, UpdateGraphRequest,
};
pub use crate::entities::{
    Artifact, Card, Chunk, Claim, ClaimStatus, ContentRange, Evidence, EvidenceKind, IndexStatus,
    Memory, MemoryCandidate, MemoryStatus, OutputStream, PendingArtifact, Relation,
    RelationEndpoint, RelationKind, Task, TaskPriority, TaskStatus, TestStatus,
    ValidationReportRecord, WebEvidenceMetadata,
};
pub use crate::errors::DomainError;
pub use crate::events::{DomainEvent, DomainEventEnvelope};
pub use crate::evidence_pack::{
    ClaimCoverageStatusRecord, ClaimEvidenceCoverageRecord, EvidenceFreshnessRecord,
    EvidencePackCompressionRecord, EvidencePackMetadataRecord, EvidencePackReplayKeyRecord,
    EvidencePackReproducibilityRecord, SourceIndependenceRecord,
};
pub use crate::generations::{
    IndexFingerprint, IndexGeneration, IndexGenerationRegistry, IndexLifecycle, RepresentationName,
};
pub use crate::ids::{
    ApprovalId, ArtifactId, ArtifactVersionId, BlobId, CardId, ChunkId, ClaimId, ConflictSetId,
    CorpusSnapshotId, DOMAIN_VERSION, DuplicateClusterId, EventId, EvidenceId, HarnessRunId,
    IndexGenerationId, LogicalTick, MemoryCandidateId, MemoryId, QueryId, RelationId, ScopeId,
    SearchTraceId, SequenceNumber, SnapshotId, StructureNodeId, TaskId, ValidationReportId,
};
pub use crate::inputs::{
    ApprovalDecision, ArtifactDetected, ChangeTaskStatusInput, CompleteTaskInput,
    ContradictMemoryInput, CreateCardInput, CreateClaimInput, CreateMemoryCandidateInput,
    CreateRelationInput, DeprecateMemoryInput, DomainInput, FetchWebRequested,
    FullTextIndexCompleted, HarnessRunCompleted, HarnessRunRequested, LinkEvidenceToClaimInput,
    LinkEvidenceToTaskInput, ModelAgentHarnessResult, ModelAgentMemoryDecision,
    ModelAgentMemoryResult, ModelAgentProposalRequest, ModelAgentProposalResult,
    ModelAgentSearchResult, ModelAgentTerminalStatus, ModelAgentValidationResult, OpenTaskInput,
    ParserResult, ParserStarted, PromoteMemoryInput, ProposeMemoryCandidateInput,
    RecordEvidenceInput, RecordValidationReportInput, RegisterArtifactInput, RegisterChunkInput,
    RequestTaskValidation, SearchExecutedInput, SearchKnowledgeCompleted, SearchKnowledgeRequested,
    SearchResultSet, SourceRemoved, StartFullTextIndex, StartIndexGenerationInput,
    SupersedeMemoryInput, TransitionIndexGenerationInput, UserIntent, ValidationCompleted,
};
pub use crate::kernel_state::KernelState;
pub use crate::provenance::{
    ParseStatus, ParsedRepresentation, RepresentationKind, SourceSpan, content_hash,
    evidence_id_for, excerpt_for, line_range_for_chunk, web_artifact_id_for, web_evidence_id_for,
};
pub use crate::replay::{replay_events, replay_inputs};
pub use crate::search::{
    ArtifactVersion, ConflictSet, ContentHash, CorpusScope, EvidenceCandidate, EvidenceCoverage,
    EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus,
    LearnedSparseContribution, LearnedSparseReason, Modality, ModalitySet,
    RETRIEVAL_SCORE_SCHEMA_VERSION, RerankCandidateStatus, RetrievalLaneScore,
    RetrievalModelFingerprint, RetrievalRawRank, RetrievalReason, RetrievalScoreFingerprint,
    RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet, SearchBudget,
    SearchCompatibilityError, SearchIntent, SearchLaneStatus, SearchOutcome, SearchPlan,
    SearchRewriteAccounting, SearchRewriteOrigin, SearchRewriteStage, SearchStage, SearchStatus,
    SearchStopReason, SearchTrace, SearchTraceCandidate, SearchTraceConstraintScore,
    SearchTraceDiversity, SearchTraceDiversityCandidate, SearchTraceExpansion, SearchTraceFilter,
    SearchTraceLane, SearchTraceLaneCandidate, SearchTraceRerank, SearchTraceRerankCandidate,
    SearchTraceRewrite, SourceLocation, StopConditions, StructureNode, StructureNodeType,
    TrustLabel,
};
pub use crate::security::{
    Authority, IntegrityState, ReviewStatus, SecurityMetadata, Sensitivity, TrustZone,
};
pub use crate::security_snapshot::{RetrievalPolicySnapshot, RetrievalPolicySnapshotError};
