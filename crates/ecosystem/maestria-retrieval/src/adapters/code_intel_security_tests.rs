use super::*;
use std::collections::BTreeSet;
use std::error::Error;
use std::sync::Arc;

use maestria_code_intel::{RecordProvenance, SourceRange, SymbolKind, SymbolMarkers, Visibility};
use maestria_domain::{
    Artifact, ArtifactId, ArtifactVersionId, Authority, BlobId, ContentHash, CorpusScope,
    DomainEvent, DomainEventEnvelope, EventId, Evidence, EvidenceId, EvidenceKind, IndexStatus,
    IntegrityState, LineRange, LogicalTick, ReviewStatus, ScopeId, SecurityMetadata, Sensitivity,
    SequenceNumber, SnapshotRef, StructureNodeId, TrustLabel, TrustZone, content_hash,
};
use maestria_governance::{RetrievalAuthorizationContext, RetrievalSecurityPolicy};
use maestria_ports::{
    ArtifactRepository, BlobStore, EvidenceRepository, InMemoryArtifactRepository,
    InMemoryBlobStore, InMemoryEvidenceRepository, PortError,
};

const SOURCE: &[u8] = b"fn compute() {\n    42\n}\n";
const REPOSITORY_ROOT: &str = "/root/repo";
const FILE_PATH: &str = "src/lib.rs";
const SOURCE_PATH: &str = "/root/repo/src/lib.rs";
const PARSER_GENERATION: &str = "cargo-rust-code-v2";

#[derive(Clone, Copy)]
enum FixtureMode {
    Complete,
    MissingSource,
    StaleSource,
    MissingArtifact,
    MissingEvidence,
}

struct FailingArtifactRepository;

impl ArtifactRepository for FailingArtifactRepository {
    fn get(&self, _: ArtifactId) -> Result<Option<Artifact>, PortError> {
        Err(PortError::Downstream {
            message: "artifact backend unavailable".to_string(),
        })
    }

    fn put(&self, _: Artifact) -> Result<(), PortError> {
        Err(PortError::Downstream {
            message: "artifact backend unavailable".to_string(),
        })
    }
}

struct Fixture {
    resolver: CodeIntelSecurityResolver,
    artifacts: Arc<InMemoryArtifactRepository>,
    evidence: Arc<InMemoryEvidenceRepository>,
    blobs: Arc<InMemoryBlobStore>,
    symbol: SymbolRecord,
    artifact: Artifact,
    evidence_record: Evidence,
    blob_id: BlobId,
    content_hash: ContentHash,
}

fn test_error(message: &str) -> Box<dyn Error> {
    Box::new(std::io::Error::other(message.to_string()))
}

fn authorized_security() -> SecurityMetadata {
    SecurityMetadata {
        trust_zone: TrustZone::Verified,
        authority: Authority::System,
        integrity: IntegrityState::Verified,
        sensitivity: Sensitivity::Public,
        review_status: ReviewStatus::Approved,
        prompt_injection_risk: false,
        poisoning_flags: Vec::new(),
        read_allowed: true,
        write_allowed: false,
        scope_id: Some(ScopeId::new(7)),
    }
}

fn symbol(content_hash: &ContentHash) -> SymbolRecord {
    SymbolRecord {
        record_id: "record-compute".to_string(),
        package: "pkg".to_string(),
        target: "lib".to_string(),
        kind: SymbolKind::Function,
        name: "compute".to_string(),
        qualified_name: "pkg::compute".to_string(),
        visibility: Visibility::Public,
        is_public_api: true,
        is_async: false,
        is_unsafe: false,
        is_test: false,
        is_bench: false,
        signature: Some("fn compute()".to_string()),
        imports: Vec::new(),
        markers: SymbolMarkers::default(),
        provenance: RecordProvenance {
            repository_root: REPOSITORY_ROOT.to_string(),
            commit_sha: "abc123".to_string(),
            worktree_identity: "worktree-1".to_string(),
            content_hash: content_hash.as_str().to_string(),
            file_path: FILE_PATH.to_string(),
            source_range: SourceRange {
                start_line: 1,
                end_line: 3,
            },
            parser_generation: PARSER_GENERATION.to_string(),
        },
    }
}

fn canonical_events(
    artifact_id: ArtifactId,
    artifact_version: ArtifactVersionId,
    blob_id: BlobId,
    content_hash: &ContentHash,
) -> Vec<DomainEventEnvelope> {
    vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id,
                title: FILE_PATH.to_string(),
                source_path: SOURCE_PATH.to_string(),
                content_hash: content_hash.as_str().to_string(),
                blob_id,
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::DocumentTreeCaptured {
                artifact_id,
                artifact_version_id: artifact_version,
                content_hash: content_hash.clone(),
                root_id: StructureNodeId::new(1),
                nodes: Vec::new(),
            },
        },
    ]
}

fn fixture(mode: FixtureMode) -> Result<Fixture, Box<dyn Error>> {
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    let blobs = Arc::new(InMemoryBlobStore::new());
    let blob_id = blobs.put(SOURCE.to_vec())?;
    let content_hash = ContentHash::new(content_hash(SOURCE))?;
    let artifact_id = ArtifactId::new(11);
    let artifact_version = ArtifactVersionId::new(22);
    let evidence_id = EvidenceId::new(33);
    let security = authorized_security();
    let range = LineRange::new(1, 3)?;
    let evidence_record = Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: SOURCE_PATH.to_string(),
            range,
            snapshot: SnapshotRef::new(blob_id, content_hash.clone()),
        },
        excerpt: "fn compute() { 42 }".to_string(),
        observed_at: LogicalTick::new(1),
        security: security.clone(),
    };
    let artifact = Artifact {
        id: artifact_id,
        title: FILE_PATH.to_string(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::from([evidence_id]),
        index_status: IndexStatus::Indexed,
        content_hash: Some(content_hash.as_str().to_string()),
        parse_status: None,
        security,
    };
    if !matches!(mode, FixtureMode::MissingArtifact) {
        artifacts.put(artifact.clone())?;
    }
    if !matches!(mode, FixtureMode::MissingEvidence) {
        evidence.put(evidence_record.clone())?;
    }

    let events = match mode {
        FixtureMode::MissingSource => Vec::new(),
        FixtureMode::StaleSource => {
            let mut events =
                canonical_events(artifact_id, artifact_version, blob_id, &content_hash);
            events.push(DomainEventEnvelope {
                id: EventId::new(3),
                sequence: SequenceNumber::new(3),
                event: DomainEvent::SourceBecameStale {
                    artifact_id,
                    source_path: SOURCE_PATH.to_string(),
                    content_hash: content_hash.as_str().to_string(),
                },
            });
            events
        }
        FixtureMode::Complete | FixtureMode::MissingArtifact | FixtureMode::MissingEvidence => {
            canonical_events(artifact_id, artifact_version, blob_id, &content_hash)
        }
    };
    let resolver = CodeIntelSecurityResolver::from_events(
        CodeIntelSecurityResolverParts {
            artifacts: artifacts.clone(),
            evidence: evidence.clone(),
            blobs: blobs.clone(),
        },
        &events,
    )?;
    Ok(Fixture {
        resolver,
        artifacts,
        evidence,
        blobs,
        symbol: symbol(&content_hash),
        artifact,
        evidence_record,
        blob_id,
        content_hash,
    })
}

fn authorization(
    policy: RetrievalSecurityPolicy,
) -> Result<RetrievalAuthorizationContext, Box<dyn Error>> {
    Ok(policy.authorization_context(&CorpusScope::Global)?)
}

fn default_authorization() -> Result<RetrievalAuthorizationContext, Box<dyn Error>> {
    authorization(RetrievalSecurityPolicy::default())
}

fn assert_internal<T>(result: Result<T, RetrievalError>, message: &str) {
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(actual)) if actual.contains(message)
    ));
}

fn replace_file_evidence(
    fixture: &Fixture,
    path: &str,
    range: LineRange,
) -> Result<(), Box<dyn Error>> {
    let mut replacement = fixture.evidence_record.clone();
    replacement.kind = EvidenceKind::FileSpan {
        path: path.to_string(),
        range,
        snapshot: SnapshotRef::new(fixture.blob_id, fixture.content_hash.clone()),
    };
    fixture.evidence.replace(replacement)?;
    Ok(())
}
fn replace_evidence_security(
    fixture: &Fixture,
    security: SecurityMetadata,
) -> Result<(), Box<dyn Error>> {
    let mut replacement = fixture.evidence_record.clone();
    replacement.security = security;
    fixture.evidence.replace(replacement)?;
    Ok(())
}

fn replace_evidence_snapshot(
    fixture: &Fixture,
    snapshot: SnapshotRef,
) -> Result<(), Box<dyn Error>> {
    let mut replacement = fixture.evidence_record.clone();
    replacement.kind = EvidenceKind::FileSpan {
        path: SOURCE_PATH.to_string(),
        range: LineRange::new(1, 3)?,
        snapshot,
    };
    fixture.evidence.replace(replacement)?;
    Ok(())
}

#[test]
fn evidence_acl_policy_is_evaluated_independently() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut security = fixture.evidence_record.security.clone();
    security.read_allowed = false;
    replace_evidence_security(&fixture, security)?;
    assert!(
        !fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?)?
    );
    Ok(())
}

#[test]
fn merged_security_taint_is_evaluated_independently() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut security = fixture.evidence_record.security.clone();
    security.scope_id = Some(ScopeId::new(8));
    replace_evidence_security(&fixture, security)?;
    let authorization = authorization(
        RetrievalSecurityPolicy::default().with_instance_scopes([ScopeId::new(7), ScopeId::new(8)]),
    )?;
    assert!(
        !fixture
            .resolver
            .authorizes(&fixture.symbol, &authorization)?
    );
    Ok(())
}

#[test]
fn evidence_snapshot_hash_mismatch_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mismatched_hash = ContentHash::new(content_hash(b"different source"))?;
    replace_evidence_snapshot(&fixture, SnapshotRef::new(fixture.blob_id, mismatched_hash))?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository evidence is missing",
    );
    Ok(())
}

#[test]
fn artifact_content_hash_mismatch_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut artifact = fixture.artifact.clone();
    artifact.content_hash = Some(content_hash(b"different source"));
    fixture.artifacts.put(artifact)?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository artifact 11 is stale or mismatched",
    );
    Ok(())
}

#[test]
fn authorized_binding_carries_canonical_identity_and_verified_trust() -> Result<(), Box<dyn Error>>
{
    let fixture = fixture(FixtureMode::Complete)?;
    let authorization = default_authorization()?;
    let binding = fixture
        .resolver
        .resolve(&fixture.symbol, &authorization)?
        .ok_or_else(|| test_error("canonical fixture was unexpectedly denied"))?;

    assert_eq!(binding.symbol.record_id, fixture.symbol.record_id);
    assert_eq!(binding.artifact_version, ArtifactVersionId::new(22));
    assert_eq!(binding.evidence.id, EvidenceId::new(33));
    assert_eq!(binding.evidence.artifact_id, ArtifactId::new(11));
    assert_eq!(trust_label(&binding.security), TrustLabel::Verified);
    Ok(())
}

#[test]
fn scope_policy_rejection_is_independent() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let authorization =
        authorization(RetrievalSecurityPolicy::default().required_scope(ScopeId::new(8)))?;
    assert!(
        !fixture
            .resolver
            .authorizes(&fixture.symbol, &authorization)?
    );
    Ok(())
}

#[test]
fn acl_policy_rejection_is_independent() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut artifact = fixture.artifact.clone();
    artifact.security.read_allowed = false;
    fixture.artifacts.put(artifact)?;
    let authorization = default_authorization()?;
    assert!(
        !fixture
            .resolver
            .authorizes(&fixture.symbol, &authorization)?
    );
    Ok(())
}

#[test]
fn required_trust_policy_rejection_is_independent() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let authorization =
        authorization(RetrievalSecurityPolicy::default().require_trust_zone(TrustZone::System))?;
    assert!(
        !fixture
            .resolver
            .authorizes(&fixture.symbol, &authorization)?
    );
    Ok(())
}

#[test]
fn maximum_sensitivity_policy_rejection_is_independent() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut artifact = fixture.artifact.clone();
    artifact.security.sensitivity = Sensitivity::Confidential;
    fixture.artifacts.put(artifact)?;
    let authorization =
        authorization(RetrievalSecurityPolicy::default().max_sensitivity(Sensitivity::Public))?;
    assert!(
        !fixture
            .resolver
            .authorizes(&fixture.symbol, &authorization)?
    );
    Ok(())
}

#[test]
fn quarantine_policy_rejection_is_independent() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut artifact = fixture.artifact.clone();
    artifact.security.trust_zone = TrustZone::Quarantined;
    fixture.artifacts.put(artifact)?;
    let authorization = default_authorization()?;
    assert!(
        !fixture
            .resolver
            .authorizes(&fixture.symbol, &authorization)?
    );
    Ok(())
}

#[test]
fn prompt_injection_policy_rejection_is_independent() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut artifact = fixture.artifact.clone();
    artifact.security.prompt_injection_risk = true;
    fixture.artifacts.put(artifact)?;
    let authorization = default_authorization()?;
    assert!(
        !fixture
            .resolver
            .authorizes(&fixture.symbol, &authorization)?
    );
    Ok(())
}

#[test]
fn missing_source_events_are_typed_errors() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::MissingSource)?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository source binding is missing",
    );
    Ok(())
}

#[test]
fn stale_source_event_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::StaleSource)?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository source binding is missing",
    );
    Ok(())
}

#[test]
fn missing_artifact_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::MissingArtifact)?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository artifact 11 is missing",
    );
    Ok(())
}

#[test]
fn missing_evidence_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::MissingEvidence)?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository evidence 33 is missing",
    );
    Ok(())
}

#[test]
fn content_hash_mismatch_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut symbol = fixture.symbol.clone();
    symbol.provenance.content_hash = content_hash(b"different source");
    assert_internal(
        fixture
            .resolver
            .authorizes(&symbol, &default_authorization()?),
        "repository code content hash mismatch",
    );
    Ok(())
}

#[test]
fn evidence_path_mismatch_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    replace_file_evidence(&fixture, "/root/repo/src/other.rs", LineRange::new(1, 3)?)?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository evidence is missing",
    );
    Ok(())
}

#[test]
fn evidence_range_mismatch_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    replace_file_evidence(&fixture, SOURCE_PATH, LineRange::new(1, 1)?)?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository evidence is missing",
    );
    Ok(())
}

#[test]
fn non_indexed_artifact_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut artifact = fixture.artifact.clone();
    artifact.index_status = IndexStatus::Pending;
    fixture.artifacts.put(artifact)?;
    assert_internal(
        fixture
            .resolver
            .authorizes(&fixture.symbol, &default_authorization()?),
        "canonical repository artifact 11 is stale or mismatched",
    );
    Ok(())
}

#[test]
fn artifact_repository_failure_is_propagated() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let events = canonical_events(
        ArtifactId::new(11),
        ArtifactVersionId::new(22),
        fixture.blob_id,
        &fixture.content_hash,
    );
    let resolver = CodeIntelSecurityResolver::from_events(
        CodeIntelSecurityResolverParts {
            artifacts: Arc::new(FailingArtifactRepository),
            evidence: fixture.evidence,
            blobs: fixture.blobs,
        },
        &events,
    )?;
    assert_internal(
        resolver.authorizes(&fixture.symbol, &default_authorization()?),
        "artifact backend unavailable",
    );
    Ok(())
}

#[test]
fn active_source_without_artifact_version_fails_matching_resolution() -> Result<(), Box<dyn Error>>
{
    let fixture = fixture(FixtureMode::Complete)?;
    let mut events = canonical_events(
        ArtifactId::new(11),
        ArtifactVersionId::new(22),
        fixture.blob_id,
        &fixture.content_hash,
    );
    events.truncate(1);
    let resolver = CodeIntelSecurityResolver::from_events(
        CodeIntelSecurityResolverParts {
            artifacts: fixture.artifacts,
            evidence: fixture.evidence,
            blobs: fixture.blobs,
        },
        &events,
    )?;
    assert_internal(
        resolver.authorizes(&fixture.symbol, &default_authorization()?),
        "canonical artifact version is missing",
    );
    Ok(())
}

#[test]
fn active_source_with_mismatched_version_hash_fails_matching_resolution()
-> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let mut events = canonical_events(
        ArtifactId::new(11),
        ArtifactVersionId::new(22),
        fixture.blob_id,
        &fixture.content_hash,
    );
    let mismatched_hash = ContentHash::new(content_hash(b"different version"))?;
    let Some(version_event) = events.get_mut(1) else {
        return Err(test_error("canonical version event missing from fixture"));
    };
    version_event.event = DomainEvent::DocumentTreeCaptured {
        artifact_id: ArtifactId::new(11),
        artifact_version_id: ArtifactVersionId::new(22),
        content_hash: mismatched_hash,
        root_id: StructureNodeId::new(1),
        nodes: Vec::new(),
    };
    let resolver = CodeIntelSecurityResolver::from_events(
        CodeIntelSecurityResolverParts {
            artifacts: fixture.artifacts,
            evidence: fixture.evidence,
            blobs: fixture.blobs,
        },
        &events,
    )?;
    assert_internal(
        resolver.authorizes(&fixture.symbol, &default_authorization()?),
        "canonical artifact version hash mismatch",
    );
    Ok(())
}

#[test]
fn delayed_stale_event_does_not_remove_newer_source() -> Result<(), Box<dyn Error>> {
    let fixture = fixture(FixtureMode::Complete)?;
    let old_hash = ContentHash::new(content_hash(b"old source"))?;
    let mut events = canonical_events(
        ArtifactId::new(10),
        ArtifactVersionId::new(20),
        fixture.blob_id,
        &old_hash,
    );
    events.extend(canonical_events(
        ArtifactId::new(11),
        ArtifactVersionId::new(22),
        fixture.blob_id,
        &fixture.content_hash,
    ));
    events.push(DomainEventEnvelope {
        id: EventId::new(5),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::SourceBecameStale {
            artifact_id: ArtifactId::new(10),
            source_path: SOURCE_PATH.to_string(),
            content_hash: old_hash.as_str().to_string(),
        },
    });
    let resolver = CodeIntelSecurityResolver::from_events(
        CodeIntelSecurityResolverParts {
            artifacts: fixture.artifacts,
            evidence: fixture.evidence,
            blobs: fixture.blobs,
        },
        &events,
    )?;
    assert!(resolver.authorizes(&fixture.symbol, &default_authorization()?)?);
    Ok(())
}
