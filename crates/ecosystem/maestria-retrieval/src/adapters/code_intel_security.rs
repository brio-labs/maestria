use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use maestria_code_intel::SymbolRecord;
use maestria_domain::{
    Artifact, ArtifactId, ArtifactVersionId, ContentHash, DomainEvent, DomainEventEnvelope,
    Evidence, EvidenceKind, IndexStatus, SecurityMetadata, TrustLabel, TrustZone,
};
use maestria_governance::{RetrievalAuthorizationContext, RetrievalDecision, scan_secrets};
use maestria_ports::{ArtifactRepository, BlobStore, EvidenceRepository};

use super::SourceSnapshotVerifier;
use crate::types::RetrievalError;

/// Canonical persistence dependencies used to authorize repository-code records.
pub struct CodeIntelSecurityResolverParts {
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
}

/// Resolves repository-code provenance against current artifact and evidence truth.
#[derive(Clone)]
pub struct CodeIntelSecurityResolver {
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    verifier: SourceSnapshotVerifier,
    sources: Arc<BTreeMap<PathBuf, CanonicalCodeSource>>,
}

#[derive(Debug, Clone)]
enum CanonicalCodeSource {
    Ready {
        artifact_id: ArtifactId,
        artifact_version: ArtifactVersionId,
        content_hash: ContentHash,
    },
    MissingVersion {
        artifact_id: ArtifactId,
        content_hash: ContentHash,
    },
    VersionHashMismatch {
        artifact_id: ArtifactId,
        content_hash: ContentHash,
        version_hash: ContentHash,
    },
}

/// Proof that a symbol is bound to current, policy-authorized artifact evidence.
pub struct AuthorizedCodeBinding {
    pub(super) symbol: SymbolRecord,
    pub(super) artifact_version: ArtifactVersionId,
    pub(super) evidence: Evidence,
    pub(super) security: SecurityMetadata,
}

impl CodeIntelSecurityResolver {
    /// Builds a deterministic source catalog from append-only domain history.
    pub fn from_events(
        parts: CodeIntelSecurityResolverParts,
        events: &[DomainEventEnvelope],
    ) -> Result<Self, RetrievalError> {
        let mut active_sources = BTreeMap::<PathBuf, (ArtifactId, ContentHash)>::new();
        let mut versions = BTreeMap::<ArtifactId, (ArtifactVersionId, ContentHash)>::new();

        for envelope in events {
            match &envelope.event {
                DomainEvent::ParserStarted {
                    artifact_id,
                    source_path,
                    content_hash,
                    ..
                } => {
                    active_sources.insert(
                        PathBuf::from(source_path),
                        (*artifact_id, content_hash.clone()),
                    );
                }
                DomainEvent::DocumentTreeCaptured {
                    artifact_id,
                    artifact_version_id,
                    content_hash,
                    ..
                } => {
                    versions.insert(*artifact_id, (*artifact_version_id, content_hash.clone()));
                }
                DomainEvent::SourceBecameStale {
                    artifact_id,
                    source_path,
                    content_hash,
                } => {
                    let path = Path::new(source_path);
                    if active_sources
                        .get(path)
                        .is_some_and(|(active_id, active_hash)| {
                            active_id == artifact_id && active_hash == content_hash
                        })
                    {
                        active_sources.remove(path);
                    }
                }
                _ => {}
            }
        }

        let mut sources = BTreeMap::new();
        for (path, (artifact_id, content_hash)) in active_sources {
            let source = match versions.get(&artifact_id) {
                None => CanonicalCodeSource::MissingVersion {
                    artifact_id,
                    content_hash,
                },
                Some((_, version_hash)) if version_hash != &content_hash => {
                    CanonicalCodeSource::VersionHashMismatch {
                        artifact_id,
                        content_hash,
                        version_hash: version_hash.clone(),
                    }
                }
                Some((artifact_version, _)) => CanonicalCodeSource::Ready {
                    artifact_id,
                    artifact_version: *artifact_version,
                    content_hash,
                },
            };
            sources.insert(path, source);
        }

        Ok(Self {
            artifacts: parts.artifacts,
            evidence: parts.evidence,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            sources: Arc::new(sources),
        })
    }

    /// Returns whether canonical artifact evidence authorizes this record.
    pub fn authorizes(
        &self,
        symbol: &SymbolRecord,
        authorization: &RetrievalAuthorizationContext,
    ) -> Result<bool, RetrievalError> {
        self.resolve(symbol, authorization)
            .map(|binding| binding.is_some())
    }

    pub(super) fn resolve(
        &self,
        symbol: &SymbolRecord,
        authorization: &RetrievalAuthorizationContext,
    ) -> Result<Option<AuthorizedCodeBinding>, RetrievalError> {
        if symbol_contains_secret(symbol) {
            return Ok(None);
        }
        let expected_path =
            Path::new(&symbol.provenance.repository_root).join(&symbol.provenance.file_path);
        let (artifact_id, artifact_version, content_hash) =
            self.resolve_source(&expected_path, symbol)?;
        let Some(artifact) = self.resolve_artifact(artifact_id, content_hash, authorization)?
        else {
            return Ok(None);
        };
        self.resolve_evidence(
            symbol,
            &expected_path,
            content_hash,
            artifact_version,
            &artifact,
            authorization,
        )
    }

    fn resolve_source<'a>(
        &'a self,
        expected_path: &Path,
        symbol: &SymbolRecord,
    ) -> Result<(ArtifactId, ArtifactVersionId, &'a ContentHash), RetrievalError> {
        let Some(source) = self.sources.get(expected_path) else {
            return Err(RetrievalError::Internal(format!(
                "canonical repository source binding is missing for {}",
                expected_path.display()
            )));
        };
        let (artifact_id, artifact_version, content_hash) = match source {
            CanonicalCodeSource::Ready {
                artifact_id,
                artifact_version,
                content_hash,
            } => (*artifact_id, *artifact_version, content_hash),
            CanonicalCodeSource::MissingVersion {
                artifact_id,
                content_hash,
            } => {
                return Err(RetrievalError::Internal(format!(
                    "canonical artifact version is missing for active repository source {} (artifact {}, hash {})",
                    expected_path.display(),
                    artifact_id,
                    content_hash.as_str()
                )));
            }
            CanonicalCodeSource::VersionHashMismatch {
                artifact_id,
                content_hash,
                version_hash,
            } => {
                return Err(RetrievalError::Internal(format!(
                    "canonical artifact version hash mismatch for active repository source {} (artifact {}, source {}, version {})",
                    expected_path.display(),
                    artifact_id,
                    content_hash.as_str(),
                    version_hash.as_str()
                )));
            }
        };
        if content_hash.as_str() != symbol.provenance.content_hash {
            return Err(RetrievalError::Internal(format!(
                "repository code content hash mismatch for {}",
                symbol.provenance.file_path
            )));
        }
        Ok((artifact_id, artifact_version, content_hash))
    }

    fn resolve_artifact(
        &self,
        artifact_id: ArtifactId,
        content_hash: &ContentHash,
        authorization: &RetrievalAuthorizationContext,
    ) -> Result<Option<Artifact>, RetrievalError> {
        let Some(artifact) = self.artifacts.get(artifact_id).map_err(port_error)? else {
            return Err(RetrievalError::Internal(format!(
                "canonical repository artifact {artifact_id} is missing"
            )));
        };
        if artifact.index_status != IndexStatus::Indexed
            || artifact.content_hash.as_ref() != Some(content_hash)
        {
            return Err(RetrievalError::Internal(format!(
                "canonical repository artifact {} is stale or mismatched",
                artifact.id
            )));
        }
        if authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed {
            return Ok(None);
        }
        Ok(Some(artifact))
    }

    fn resolve_evidence(
        &self,
        symbol: &SymbolRecord,
        expected_path: &Path,
        content_hash: &ContentHash,
        artifact_version: ArtifactVersionId,
        artifact: &Artifact,
        authorization: &RetrievalAuthorizationContext,
    ) -> Result<Option<AuthorizedCodeBinding>, RetrievalError> {
        let mut authorized_binding = None;
        let mut has_canonical_binding = false;
        for evidence_id in &artifact.evidence_ids {
            let Some(evidence) = self.evidence.get(*evidence_id).map_err(port_error)? else {
                return Err(RetrievalError::Internal(format!(
                    "canonical repository evidence {evidence_id} is missing"
                )));
            };
            if evidence.artifact_id != artifact.id {
                return Err(RetrievalError::Internal(format!(
                    "repository code evidence {} owner mismatch: expected artifact {}, found {}",
                    evidence.id, artifact.id, evidence.artifact_id
                )));
            }
            if !evidence_binds_symbol(&evidence, symbol, expected_path, content_hash) {
                continue;
            }
            has_canonical_binding = true;
            let security = artifact.security.taint_from(&evidence.security);
            if authorization.evaluate(&evidence.security) != RetrievalDecision::Allowed
                || authorization.evaluate(&security) != RetrievalDecision::Allowed
                || !scan_secrets(&evidence.excerpt).is_clean()
            {
                continue;
            }
            self.verifier.verify(&evidence, artifact)?;
            if authorized_binding.is_some() {
                return Err(RetrievalError::Internal(format!(
                    "repository code symbol {} has ambiguous canonical evidence",
                    symbol.record_id
                )));
            }
            authorized_binding = Some(AuthorizedCodeBinding {
                symbol: symbol.clone(),
                artifact_version,
                evidence,
                security,
            });
        }
        if !has_canonical_binding {
            return Err(RetrievalError::Internal(format!(
                "canonical repository evidence is missing for symbol {}",
                symbol.record_id
            )));
        }
        Ok(authorized_binding)
    }
}

fn symbol_contains_secret(symbol: &SymbolRecord) -> bool {
    !scan_secrets(&symbol.name).is_clean()
        || !scan_secrets(&symbol.qualified_name).is_clean()
        || !scan_secrets(&symbol.package).is_clean()
        || !scan_secrets(&symbol.target).is_clean()
}

fn evidence_binds_symbol(
    evidence: &Evidence,
    symbol: &SymbolRecord,
    expected_path: &Path,
    content_hash: &ContentHash,
) -> bool {
    let EvidenceKind::FileSpan {
        path,
        range,
        snapshot,
    } = &evidence.kind
    else {
        return false;
    };
    Path::new(path) == expected_path
        && snapshot.content_hash() == content_hash
        && range.start() <= symbol.provenance.source_range.start_line()
        && range.end() >= symbol.provenance.source_range.end_line()
}

/// Maps a symbol's security metadata to the evidence trust label.
pub fn trust_label(security: &SecurityMetadata) -> TrustLabel {
    match security.trust_zone {
        TrustZone::System | TrustZone::Verified => TrustLabel::Verified,
        TrustZone::Untrusted | TrustZone::Quarantined => TrustLabel::Unverified,
    }
}

fn port_error(error: maestria_ports::PortError) -> RetrievalError {
    RetrievalError::Internal(error.to_string())
}

#[cfg(test)]
#[path = "code_intel_security_tests.rs"]
mod tests;
