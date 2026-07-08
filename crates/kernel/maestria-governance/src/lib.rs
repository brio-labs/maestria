#![forbid(unsafe_code)]

//! Governance boundary for Maestria.
//!
//! This crate is intentionally side-effect free: it classifies and gates domain
//! intentions but performs no I/O. Runtime ports and adapter implementations are
//! expected to live elsewhere.

use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use maestria_domain::{
    Artifact, ArtifactId, Card, ChunkId, Claim, DomainEvent, DomainEventEnvelope, EvidenceId,
    MaestriaEffect, Task, TaskStatus,
};

pub const GOVERNANCE_VERSION: &str = "0.1.0";

macro_rules! newtype_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn value(&self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

newtype_id!(BlobId);
newtype_id!(HarnessRunId);
newtype_id!(MemoryCandidateId);

//
// Scope and policy surface
//

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scope {
    read_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
    allowed_harnesses: Vec<String>,
    blocked_commands: Vec<String>,
    web_allowed: bool,
}

impl Scope {
    pub fn new(
        read_roots: Vec<PathBuf>,
        write_roots: Vec<PathBuf>,
        allowed_harnesses: Vec<String>,
        blocked_commands: Vec<String>,
        web_allowed: bool,
    ) -> Self {
        Self {
            read_roots,
            write_roots,
            allowed_harnesses,
            blocked_commands,
            web_allowed,
        }
    }

    pub fn allows_read(&self, path: &Path) -> bool {
        self.read_roots.iter().any(|root| path.starts_with(root))
            || self.write_roots.iter().any(|root| path.starts_with(root))
    }

    pub fn allows_write(&self, path: &Path) -> bool {
        self.write_roots.iter().any(|root| path.starts_with(root))
    }

    pub fn command_allowed(&self, command: &str) -> bool {
        let command = command.trim().to_lowercase();
        if command.is_empty() {
            return false;
        }
        !self.blocked_commands.iter().any(|entry| {
            let entry = entry.as_str().trim().to_lowercase();
            command == entry || command.starts_with(&format!("{entry} "))
        })
    }

    pub fn harness_allowed(&self, harness: &str) -> bool {
        self.allowed_harnesses.iter().any(|entry| entry == harness)
    }

    pub fn web_allowed(&self) -> bool {
        self.web_allowed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeGuard {
    scope: Scope,
}

impl ScopeGuard {
    pub fn new(scope: Scope) -> Self {
        Self { scope }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    pub fn allows_read(&self, path: &Path) -> bool {
        self.scope.allows_read(path)
    }

    pub fn allows_write(&self, path: &Path) -> bool {
        self.scope.allows_write(path)
    }

    pub fn command_allowed(&self, command: &str) -> bool {
        self.scope.command_allowed(command)
    }

    pub fn harness_allowed(&self, harness: &str) -> bool {
        self.scope.harness_allowed(harness)
    }

    pub fn web_allowed(&self) -> bool {
        self.scope.web_allowed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskClass {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    RequireApproval { reason: String },
    Deny { reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn requires_approval(&self) -> bool {
        matches!(self, Self::RequireApproval { .. })
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomyProfile {
    ReadOnly,
    Assisted,
    ScopedAutonomy,
    StrictResearch,
    TrustedWorkspace,
}

#[derive(Debug, Clone, Copy)]
pub struct ApprovalRequest<'a> {
    pub effect: &'a MaestriaEffect,
    pub profile: AutonomyProfile,
    pub scope: &'a ScopeGuard,
}

pub trait ClassifyRisk {
    fn classify(&self, effect: &MaestriaEffect, scope: &ScopeGuard) -> RiskClass;
}

#[derive(Debug)]
pub struct DefaultRiskClassifier;

impl ClassifyRisk for DefaultRiskClassifier {
    fn classify(&self, effect: &MaestriaEffect, scope: &ScopeGuard) -> RiskClass {
        match effect {
            MaestriaEffect::PersistEvent { .. } | MaestriaEffect::StoreBlob(_) => RiskClass::Low,
            MaestriaEffect::RunValidation(_) | MaestriaEffect::RequestApproval(_) => {
                RiskClass::Medium
            }
            MaestriaEffect::IndexFullText(_) | MaestriaEffect::EmbedChunks(_) => RiskClass::Medium,
            MaestriaEffect::EmitDiagnostic(_) => RiskClass::Low,
            MaestriaEffect::QueryHarness(req) => {
                let command = req.command.to_lowercase();
                if command.starts_with("rm") || command.contains("delete") {
                    if scope.web_allowed() {
                        RiskClass::High
                    } else {
                        RiskClass::Critical
                    }
                } else {
                    RiskClass::Medium
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct ApprovalGateDecision {
    pub decision: PolicyDecision,
    pub risk: RiskClass,
}

pub trait ApprovalGate {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalGateDecision;
}

#[derive(Debug)]
pub struct DefaultApprovalGate;

impl DefaultApprovalGate {
    fn requires_approval_for(&self, profile: AutonomyProfile, risk: RiskClass) -> bool {
        matches!(
            (profile, risk),
            (AutonomyProfile::ReadOnly, RiskClass::Medium)
                | (AutonomyProfile::ReadOnly, RiskClass::High)
                | (AutonomyProfile::ReadOnly, RiskClass::Critical)
                | (AutonomyProfile::Assisted, RiskClass::High)
                | (AutonomyProfile::Assisted, RiskClass::Critical)
                | (AutonomyProfile::ScopedAutonomy, RiskClass::High)
                | (AutonomyProfile::StrictResearch, RiskClass::Critical)
                | (AutonomyProfile::TrustedWorkspace, RiskClass::Critical)
        )
    }

    fn denied(&self, profile: AutonomyProfile, risk: RiskClass) -> bool {
        matches!(
            (profile, risk),
            (AutonomyProfile::ReadOnly, RiskClass::Critical)
                | (AutonomyProfile::Assisted, RiskClass::Critical)
                | (AutonomyProfile::ScopedAutonomy, RiskClass::Critical)
                | (AutonomyProfile::StrictResearch, RiskClass::High)
        )
    }
}

impl ApprovalGate for DefaultApprovalGate {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalGateDecision {
        let classifier = DefaultRiskClassifier;
        let risk = classifier.classify(request.effect, request.scope);
        let reason = match (request.profile, risk) {
            (AutonomyProfile::ReadOnly, RiskClass::Low) => {
                "read-only profile allows low-risk actions".to_string()
            }
            (AutonomyProfile::ReadOnly, _) => {
                "read-only profile blocks non-read operations without explicit approval".to_string()
            }
            (AutonomyProfile::Assisted, RiskClass::Low) => {
                "assisted profile allows low-risk actions".to_string()
            }
            (AutonomyProfile::Assisted, RiskClass::Medium) => {
                "assisted profile requires approval for medium risk actions".to_string()
            }
            (AutonomyProfile::Assisted, RiskClass::High | RiskClass::Critical) => {
                "assisted profile blocks high-risk actions without review".to_string()
            }
            (AutonomyProfile::ScopedAutonomy, RiskClass::Low) => {
                "scoped-autonomy profile allows low-risk actions".to_string()
            }
            (AutonomyProfile::ScopedAutonomy, RiskClass::Medium) => {
                "scoped-autonomy profile requires approval for medium risk".to_string()
            }
            (AutonomyProfile::ScopedAutonomy, RiskClass::High | RiskClass::Critical) => {
                "scoped-autonomy profile blocks high-risk actions".to_string()
            }
            (AutonomyProfile::StrictResearch, RiskClass::Medium | RiskClass::Low) => {
                "strict-research profile allows low/medium research actions".to_string()
            }
            (AutonomyProfile::StrictResearch, RiskClass::High) => {
                "strict-research profile requires approval for high risk actions".to_string()
            }
            (AutonomyProfile::StrictResearch, RiskClass::Critical) => {
                "strict-research profile blocks critical-risk actions".to_string()
            }
            (AutonomyProfile::TrustedWorkspace, RiskClass::High | RiskClass::Critical) => {
                "trusted-workspace profile requires approval for high risk actions".to_string()
            }
            (AutonomyProfile::TrustedWorkspace, _) => {
                "trusted-workspace profile allows non-critical actions".to_string()
            }
        };

        if self.denied(request.profile, risk) {
            ApprovalGateDecision {
                decision: PolicyDecision::Deny { reason },
                risk,
            }
        } else if self.requires_approval_for(request.profile, risk) {
            ApprovalGateDecision {
                decision: PolicyDecision::RequireApproval { reason },
                risk,
            }
        } else {
            ApprovalGateDecision {
                decision: PolicyDecision::Allow,
                risk,
            }
        }
    }
}

#[derive(Debug)]
pub struct ValidationRequest {
    pub task: Task,
    pub validation_report_present: bool,
    pub had_warning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationDecision {
    AllowCompletion,
    BlockedByMissingValidation { reason: String },
    BlockedByPolicy { reason: String },
}

pub trait ValidationGate {
    fn evaluate(&self, request: &ValidationRequest) -> ValidationDecision;
}

#[derive(Debug)]
pub struct DefaultValidationGate {
    allow_warnings: bool,
}

impl DefaultValidationGate {
    pub const fn new(allow_warnings: bool) -> Self {
        Self { allow_warnings }
    }
}

impl ValidationGate for DefaultValidationGate {
    fn evaluate(&self, request: &ValidationRequest) -> ValidationDecision {
        if !request.validation_report_present {
            return ValidationDecision::BlockedByMissingValidation {
                reason: "task completion requires validation report".to_string(),
            };
        }

        if request.task.status == TaskStatus::Completed {
            if request.had_warning && !self.allow_warnings {
                return ValidationDecision::BlockedByPolicy {
                    reason: "warnings are blocked in this policy".to_string(),
                };
            }
            ValidationDecision::AllowCompletion
        } else {
            ValidationDecision::BlockedByPolicy {
                reason: format!(
                    "task status {:?} is not completion state",
                    request.task.status
                ),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryCandidate {
    pub id: MemoryCandidateId,
    pub artifact_id: ArtifactId,
    pub evidence_ids: Vec<EvidenceId>,
    pub claim: Claim,
    pub confidence: f32,
}

impl MemoryCandidate {
    pub fn has_evidence(&self) -> bool {
        !self.evidence_ids.is_empty()
    }
}

#[derive(Debug)]
pub struct MemoryPromotionRequest {
    pub candidate: MemoryCandidate,
    pub user_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryPromotionDecision {
    Promote,
    RequireEvidence { reason: String },
    RequireReview { reason: String },
    Deny { reason: String },
}

pub trait MemoryPromotionGate {
    fn evaluate(&self, request: &MemoryPromotionRequest) -> MemoryPromotionDecision;
}

#[derive(Debug)]
pub struct DefaultMemoryPromotionGate;

impl MemoryPromotionGate for DefaultMemoryPromotionGate {
    fn evaluate(&self, request: &MemoryPromotionRequest) -> MemoryPromotionDecision {
        if !request.candidate.has_evidence() {
            return MemoryPromotionDecision::RequireEvidence {
                reason: "memory candidate must contain at least one evidence id".to_string(),
            };
        }

        if request.candidate.confidence < 0.5 {
            return MemoryPromotionDecision::RequireReview {
                reason: "low confidence memory candidate".to_string(),
            };
        }

        if request.user_approved {
            MemoryPromotionDecision::Promote
        } else {
            MemoryPromotionDecision::RequireReview {
                reason: "user approval required for promotion".to_string(),
            }
        }
    }
}

//
// Adapter/port contracts
//

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    NotFound,
    Conflict { message: String },
    InvalidInput { message: String },
    Downstream { message: String },
    Internal { message: String },
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found"),
            Self::Conflict { message } => write!(f, "conflict: {message}"),
            Self::InvalidInput { message } => write!(f, "invalid input: {message}"),
            Self::Downstream { message } => write!(f, "downstream error: {message}"),
            Self::Internal { message } => write!(f, "internal error: {message}"),
        }
    }
}

impl std::error::Error for PortError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFilter {
    pub artifact_id: Option<ArtifactId>,
}

pub trait ArtifactRepository {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError>;
    fn put(&self, artifact: Artifact) -> Result<(), PortError>;
}

pub trait EventLog {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError>;
    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError>;
}

pub trait BlobStore {
    fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError>;
    fn get(&self, id: BlobId) -> Result<Vec<u8>, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedChunk {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub q: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub chunk: IndexedChunk,
    pub score: u32,
}

pub trait FullTextIndex {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError>;
    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub size: usize,
    pub extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHandle {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseContext {
    pub artifact_id: ArtifactId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChunk {
    pub chunk_id: ChunkId,
    pub artifact_id: ArtifactId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArtifact {
    pub artifact_id: ArtifactId,
    pub chunks: Vec<ParsedChunk>,
    pub cards: Vec<Card>,
}

pub trait Parser {
    fn id(&self) -> &'static str;
    fn supports(&self, file: &FileMetadata) -> bool;
    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessCommandClass {
    Shell,
    Browser,
    Fetch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessCapabilities {
    pub command_classes: Vec<HarnessCommandClass>,
    pub write_enabled: bool,
    pub read_enabled: bool,
    pub web_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRequest {
    pub run_id: HarnessRunId,
    pub command: String,
    pub working_directory: PathBuf,
    pub duration_budget: Duration,
    pub class: HarnessCommandClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessOutcome {
    pub run_id: HarnessRunId,
    pub command: String,
    pub exit_code: i32,
    pub scope_checked: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
    pub artifacts_created: Vec<BlobId>,
    pub diff_summary: Option<String>,
    pub validation_hints: Vec<String>,
}

pub trait HarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError>;
    fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, PortError>;
}

//
// In-memory adapter fakes for compile-time/contract tests.
//

#[derive(Clone, Default)]
pub struct InMemoryArtifactRepository {
    artifacts: Arc<Mutex<BTreeMap<ArtifactId, Artifact>>>,
}

impl InMemoryArtifactRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ArtifactRepository for InMemoryArtifactRepository {
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

#[derive(Clone, Default)]
pub struct InMemoryEventLog {
    events: Arc<Mutex<Vec<DomainEventEnvelope>>>,
}

impl InMemoryEventLog {
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventLog for InMemoryEventLog {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let mut guard = self.events.lock().map_err(|_| PortError::Internal {
            message: "event log lock poisoned".to_string(),
        })?;
        guard.push(event);
        Ok(())
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let guard = self.events.lock().map_err(|_| PortError::Internal {
            message: "event log lock poisoned".to_string(),
        })?;
        let mut entries = guard.clone();
        if let Some(artifact_id) = filter.artifact_id {
            entries.retain(|entry| match &entry.event {
                DomainEvent::ArtifactRegistered {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::ChunkRegistered {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::CardCreated {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::ClaimCreated {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::ArtifactParsed {
                    artifact_id: current,
                    ..
                } => *current == artifact_id,
                _ => false,
            });
        }
        Ok(entries)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryBlobStore {
    blobs: Arc<Mutex<BTreeMap<BlobId, Vec<u8>>>>,
    next_id: Arc<Mutex<u64>>,
}

impl InMemoryBlobStore {
    pub fn new() -> Self {
        Self {
            blobs: Default::default(),
            next_id: Arc::new(Mutex::new(1)),
        }
    }
}

impl BlobStore for InMemoryBlobStore {
    fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError> {
        let mut id_guard = self.next_id.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;
        let mut blob_guard = self.blobs.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;

        let id = BlobId::new(*id_guard);
        *id_guard = id.value().saturating_add(1);
        blob_guard.insert(id, bytes);
        Ok(id)
    }

    fn get(&self, id: BlobId) -> Result<Vec<u8>, PortError> {
        let guard = self.blobs.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;
        guard.get(&id).cloned().ok_or(PortError::NotFound)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryFullTextIndex {
    chunks: Arc<Mutex<Vec<IndexedChunk>>>,
}

impl InMemoryFullTextIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl FullTextIndex for InMemoryFullTextIndex {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        let mut guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        guard.extend(chunks);
        Ok(())
    }

    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError> {
        let guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        let mut hits = guard
            .iter()
            .filter(|chunk| {
                let needle = query.q.to_lowercase();
                chunk.text.to_lowercase().contains(&needle)
            })
            .map(|chunk| SearchHit {
                chunk: chunk.clone(),
                score: u32::try_from(chunk.text.len()).unwrap_or(u32::MAX),
            })
            .collect::<Vec<_>>();

        hits.sort_by_key(|b| std::cmp::Reverse(b.score));
        if hits.len() > query.limit {
            hits.truncate(query.limit);
        }
        Ok(hits)
    }
}

#[derive(Clone)]
pub struct InMemoryParser;

impl Default for InMemoryParser {
    fn default() -> Self {
        Self
    }
}

impl InMemoryParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for InMemoryParser {
    fn id(&self) -> &'static str {
        "in-memory-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        match file.extension.as_deref() {
            Some(ext) => matches!(ext, "md" | "txt" | "rs" | "toml"),
            None => false,
        }
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        if file.bytes.is_empty() {
            return Err(PortError::InvalidInput {
                message: "input file is empty".to_string(),
            });
        }

        let text = String::from_utf8(file.bytes).map_err(|err| PortError::InvalidInput {
            message: format!("file bytes are not utf8: {err}"),
        })?;

        let chunk = ParsedChunk {
            chunk_id: ChunkId::new(context.artifact_id.value()),
            artifact_id: context.artifact_id,
            text,
        };
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            chunks: vec![chunk],
            cards: Vec::new(),
        })
    }
}

#[derive(Clone)]
pub struct InMemoryHarnessAdapter {
    capabilities: HarnessCapabilities,
}

impl Default for InMemoryHarnessAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryHarnessAdapter {
    pub fn new() -> Self {
        Self {
            capabilities: HarnessCapabilities {
                command_classes: vec![HarnessCommandClass::Shell, HarnessCommandClass::Browser],
                write_enabled: true,
                read_enabled: true,
                web_enabled: false,
            },
        }
    }
}

impl HarnessAdapter for InMemoryHarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError> {
        Ok(self.capabilities.clone())
    }

    fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, PortError> {
        if request.command.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "command must not be empty".to_string(),
            });
        }

        let mut stdout = Vec::new();
        stdout.extend_from_slice(format!("executed {}", request.command).as_bytes());

        Ok(HarnessOutcome {
            run_id: request.run_id,
            command: request.command,
            exit_code: 0,
            scope_checked: true,
            stdout,
            stderr: Vec::new(),
            duration: Duration::from_millis(1),
            artifacts_created: Vec::new(),
            diff_summary: None,
            validation_hints: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{EventId, SequenceNumber, TaskId};
    fn candidate_with_artifact(id: u64, has_evidence: bool) -> MemoryCandidate {
        let claim = Claim {
            id: maestria_domain::ClaimId::new(id),
            artifact_id: ArtifactId::new(id),
            text: "candidate".to_string(),
            status: maestria_domain::ClaimStatus::Draft,
            evidence_ids: if has_evidence {
                let mut set = std::collections::BTreeSet::new();
                set.insert(EvidenceId::new(id));
                set
            } else {
                Default::default()
            },
        };

        MemoryCandidate {
            id: MemoryCandidateId::new(id),
            artifact_id: ArtifactId::new(id),
            evidence_ids: claim.evidence_ids.iter().copied().collect(),
            claim,
            confidence: 0.9,
        }
    }

    #[test]
    fn scope_guard_checks_read_write_paths() {
        let scope = Scope::new(
            vec![PathBuf::from("/allowed/read")],
            vec![PathBuf::from("/allowed/write")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            true,
        );
        let guard = ScopeGuard::new(scope);

        assert!(guard.allows_read(Path::new("/allowed/read/docs/note.md")));
        assert!(guard.allows_write(Path::new("/allowed/write/output.md")));
        assert!(!guard.allows_write(Path::new("/allowed/read/docs/note.md")));
        assert!(!guard.command_allowed("rm -rf /tmp"));
        assert!(guard.harness_allowed("shell"));
        assert!(guard.web_allowed());
    }

    #[test]
    fn approval_profile_changes_decision_without_domain_changes() {
        let scope = Scope::new(
            vec![PathBuf::from("/data")],
            vec![PathBuf::from("/data")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            true,
        );
        let guard = ScopeGuard::new(scope);

        let effect = MaestriaEffect::PersistEvent {
            event: DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(1),
                title: "notes".to_string(),
            },
        };
        let read_only = ApprovalRequest {
            effect: &effect,
            profile: AutonomyProfile::ReadOnly,
            scope: &guard,
        };
        let assisted = ApprovalRequest {
            profile: AutonomyProfile::Assisted,
            ..read_only
        };

        let gate = DefaultApprovalGate;
        let read_only_decision = gate.decide(&read_only);
        let assisted_decision = gate.decide(&assisted);

        assert!(read_only_decision.decision.is_allowed());
        assert!(assisted_decision.decision.is_allowed());
        assert!(read_only_decision.risk <= assisted_decision.risk);
        assert!(matches!(
            gate.decide(&ApprovalRequest {
                profile: AutonomyProfile::StrictResearch,
                effect: &effect,
                scope: &guard,
            })
            .decision,
            PolicyDecision::Allow
        ));
    }

    #[test]
    fn risky_effects_require_approval_gate() {
        let scope = Scope::new(
            vec![PathBuf::from("/data")],
            vec![PathBuf::from("/data")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            false,
        );
        let guard = ScopeGuard::new(scope);
        let risky_effect = MaestriaEffect::QueryHarness(maestria_domain::QueryHarnessRequest {
            command: "rm -rf /tmp".into(),
        });

        let request = ApprovalRequest {
            effect: &risky_effect,
            profile: AutonomyProfile::ScopedAutonomy,
            scope: &guard,
        };
        let gate = DefaultApprovalGate;
        let decision = gate.decide(&request);

        assert!(matches!(
            decision.decision,
            PolicyDecision::Deny { .. } | PolicyDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn validation_gate_requires_report() {
        let task = Task {
            id: TaskId::new(12),
            title: "example".to_string(),
            priority: maestria_domain::TaskPriority::Normal,
            status: TaskStatus::Completed,
            artifact_ids: Default::default(),
            evidence_ids: Default::default(),
        };

        let gate = DefaultValidationGate::new(true);
        let decision = gate.evaluate(&ValidationRequest {
            task,
            validation_report_present: false,
            had_warning: false,
        });
        assert!(matches!(
            decision,
            ValidationDecision::BlockedByMissingValidation { .. }
        ));
    }

    #[test]
    fn memory_promotion_gate_requires_evidence() {
        let candidate = candidate_with_artifact(42, false);
        let request = MemoryPromotionRequest {
            candidate,
            user_approved: true,
        };

        let decision = DefaultMemoryPromotionGate.evaluate(&request);
        assert!(matches!(
            decision,
            MemoryPromotionDecision::RequireEvidence { .. }
        ));
    }

    #[test]
    fn in_memory_contract_adapters_round_trip() {
        let artifact_repo = InMemoryArtifactRepository::new();
        let event_log = InMemoryEventLog::new();
        let blob_store = InMemoryBlobStore::new();
        let index = InMemoryFullTextIndex::new();
        let parser = InMemoryParser::new();
        let harness = InMemoryHarnessAdapter::new();

        let artifact = Artifact {
            id: ArtifactId::new(1),
            title: "notes.md".to_string(),
            chunk_ids: Default::default(),
            card_ids: Default::default(),
            claim_ids: Default::default(),
            evidence_ids: Default::default(),
        };
        artifact_repo.put(artifact.clone()).expect("artifact put");
        assert_eq!(
            artifact_repo.get(ArtifactId::new(1)).expect("artifact get"),
            Some(artifact.clone())
        );

        let event = DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "notes".to_string(),
        };
        event_log
            .append(DomainEventEnvelope {
                id: EventId::new(1),
                sequence: SequenceNumber::new(1),
                event,
            })
            .expect("event appended");

        let filtered = event_log
            .scan(EventFilter {
                artifact_id: Some(ArtifactId::new(1)),
            })
            .expect("event scan");
        assert_eq!(filtered.len(), 1);

        let blob = blob_store.put(vec![1, 2, 3]).expect("blob put");
        assert_eq!(blob_store.get(blob).expect("blob get"), vec![1, 2, 3]);

        index
            .index_chunks(vec![IndexedChunk {
                artifact_id: ArtifactId::new(1),
                chunk_id: ChunkId::new(10),
                text: "hello search".to_string(),
            }])
            .expect("index chunks");
        let hits = index
            .search(SearchQuery {
                q: "hello".to_string(),
                limit: 10,
            })
            .expect("search");
        assert_eq!(hits.len(), 1);

        let parsed = parser
            .parse(
                FileHandle {
                    path: PathBuf::from("notes.md"),
                    bytes: b"alpha".to_vec(),
                },
                ParseContext {
                    artifact_id: ArtifactId::new(1),
                },
            )
            .expect("parse");
        assert_eq!(parsed.artifact_id, ArtifactId::new(1));
        assert_eq!(parsed.chunks.len(), 1);

        let capabilities = harness.capabilities().expect("capabilities");
        assert!(capabilities.read_enabled);
        let outcome = harness
            .execute(HarnessRequest {
                run_id: HarnessRunId::new(7),
                command: "echo ok".into(),
                working_directory: PathBuf::from("/tmp"),
                duration_budget: Duration::from_secs(1),
                class: HarnessCommandClass::Shell,
            })
            .expect("execute");
        assert_eq!(outcome.exit_code, 0);
    }
}
