use std::{collections::BTreeSet, sync::Arc};

use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};
use async_trait::async_trait;
use maestria_domain::{
    ArtifactVersionId, ContentRange, Evidence, EvidenceCandidate, EvidenceKind, EvidenceSpan,
    FreshnessStatus, IndexGenerationId, RetrievalReason, RetrievalScoreSet, SourceLocation,
    SourceSpan, StructureNodeId, TrustLabel,
};
use maestria_ports::BlobStore;

pub(super) fn port_error(error: maestria_ports::PortError) -> RetrievalError {
    RetrievalError::Internal(error.to_string())
}

pub(super) fn one_based_rank(rank: usize) -> u32 {
    match u32::try_from(rank.saturating_add(1)) {
        Ok(rank) => rank,
        Err(_) => u32::MAX,
    }
}

pub(super) fn generation_mismatch(
    expected: IndexGenerationId,
    actual: IndexGenerationId,
) -> RetrievalError {
    RetrievalError::Internal(format!(
        "retriever generation mismatch: expected {expected}, found {actual}"
    ))
}

/// Filters retriever results to the latest known artifact version per source.
pub struct CurrentVersionFilter {
    inner: Arc<dyn CandidateRetriever>,
    active_versions: Arc<BTreeSet<ArtifactVersionId>>,
}

impl CurrentVersionFilter {
    pub fn new(
        inner: Arc<dyn CandidateRetriever>,
        active_versions: BTreeSet<ArtifactVersionId>,
    ) -> Self {
        Self {
            inner,
            active_versions: Arc::new(active_versions),
        }
    }
}

#[async_trait]
impl CandidateRetriever for CurrentVersionFilter {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.inner.descriptor()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        let mut batch = self.inner.retrieve(request).await?;
        if !self.active_versions.is_empty() {
            batch
                .candidates
                .retain(|candidate| self.active_versions.contains(&candidate.artifact_version));
        }
        Ok(batch)
    }
}

/// Verifies immutable source snapshots before a candidate crosses retrieval.
pub struct SourceSnapshotVerifier {
    blobs: Arc<dyn BlobStore + Send + Sync>,
}

impl SourceSnapshotVerifier {
    pub fn new(blobs: Arc<dyn BlobStore + Send + Sync>) -> Self {
        Self { blobs }
    }

    pub fn verify(&self, evidence: &Evidence) -> Result<(), RetrievalError> {
        let (snapshot, expected_hash, excerpt, file_range) = match &evidence.kind {
            EvidenceKind::WebSnapshot {
                snapshot,
                content_hash,
                ..
            } => (
                Some(*snapshot),
                Some(content_hash),
                evidence.excerpt.as_str(),
                None,
            ),
            EvidenceKind::FileSpan {
                snapshot: Some(snapshot),
                content_hash,
                range,
                ..
            } => (
                Some(*snapshot),
                Some(content_hash),
                evidence.excerpt.as_str(),
                Some(*range),
            ),
            EvidenceKind::PdfSpan { blob, .. } | EvidenceKind::PdfRegion { blob, .. } => {
                (Some(*blob), None, "", None)
            }
            _ => (None, None, "", None),
        };
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        let bytes = self.blobs.get(snapshot).map_err(port_error)?;
        if bytes.is_empty() {
            return Err(RetrievalError::Internal(format!(
                "evidence {} source snapshot is empty",
                evidence.id
            )));
        }
        let actual_hash = maestria_domain::content_hash(&bytes);
        if expected_hash.is_some_and(|expected| expected != &actual_hash) {
            return Err(RetrievalError::Internal(format!(
                "evidence {} source snapshot hash mismatch",
                evidence.id
            )));
        }
        let source = String::from_utf8_lossy(&bytes);
        if let Some(range) = file_range {
            verify_file_range(source.as_ref(), range, evidence.id, &evidence.excerpt)?;
        } else if !excerpt.is_empty() && !contains_compact_excerpt(&source, excerpt) {
            return Err(RetrievalError::Internal(format!(
                "evidence {} excerpt is absent from its source snapshot",
                evidence.id
            )));
        }
        Ok(())
    }
}

fn verify_file_range(
    source: &str,
    range: ContentRange,
    evidence_id: maestria_domain::EvidenceId,
    excerpt: &str,
) -> Result<(), RetrievalError> {
    let lines = source.lines().collect::<Vec<_>>();
    if range.start == 0 || range.start > range.end || range.end > lines.len() {
        return Err(RetrievalError::Internal(format!(
            "evidence {evidence_id} file range is outside its source snapshot"
        )));
    }
    let selected = lines[range.start - 1..range.end].join("\n");
    if !excerpt.is_empty() && !contains_compact_excerpt(&selected, excerpt) {
        return Err(RetrievalError::Internal(format!(
            "evidence {evidence_id} excerpt does not match its source range"
        )));
    }
    Ok(())
}

fn contains_compact_excerpt(source: &str, excerpt: &str) -> bool {
    let needle: Vec<_> = excerpt.split_whitespace().collect();
    if needle.is_empty() {
        return true;
    }
    let mut remaining = source.split_whitespace();
    loop {
        let mut candidate = remaining.clone();
        if needle
            .iter()
            .all(|expected| candidate.next() == Some(*expected))
        {
            return true;
        }
        if remaining.next().is_none() {
            return false;
        }
    }
}

pub(super) fn candidate_from_records(
    artifact_id: maestria_domain::ArtifactId,
    source_span: &SourceSpan,
    evidence: &Evidence,
    node_id: StructureNodeId,
    scores: RetrievalScoreSet,
    reasons: Vec<RetrievalReason>,
) -> Result<EvidenceCandidate, RetrievalError> {
    let (location, range) = evidence_location(evidence, source_span)?;
    let source_span = EvidenceSpan::new(Some(node_id), location, range)
        .map_err(|error| RetrievalError::Internal(error.to_string()))?;
    Ok(EvidenceCandidate {
        evidence_id: evidence.id,
        artifact_version: ArtifactVersionId::new(artifact_id.value()),
        source_span,
        scores,
        trust: TrustLabel::Unverified,
        freshness: FreshnessStatus::Unknown,
        duplicate_cluster: None,
        reasons,
        coverage_keys: Vec::new(),
    })
}

fn evidence_location(
    evidence: &Evidence,
    source_span: &SourceSpan,
) -> Result<(SourceLocation, ContentRange), RetrievalError> {
    match &evidence.kind {
        EvidenceKind::FileSpan { path, range, .. } => {
            let (start_line, end_line) = match source_span {
                SourceSpan::TextSpan {
                    start_line,
                    end_line,
                } => (*start_line as u32, *end_line as u32),
                SourceSpan::PdfSpan { .. } => {
                    return Err(RetrievalError::Internal(
                        "file evidence has a PDF source span".to_string(),
                    ));
                }
                SourceSpan::PdfRegion { .. } => {
                    return Err(RetrievalError::Internal(
                        "file evidence has a PDF region source span".to_string(),
                    ));
                }
            };
            Ok((
                SourceLocation::File {
                    path: path.clone(),
                    start_line,
                    end_line,
                },
                *range,
            ))
        }
        EvidenceKind::PdfSpan {
            page_start,
            page_end,
            ..
        } => Ok((
            SourceLocation::Page {
                page_start: *page_start,
                page_end: *page_end,
            },
            ContentRange { start: 0, end: 1 },
        )),
        EvidenceKind::PdfRegion {
            page,
            x,
            y,
            width,
            height,
            ..
        } => Ok((
            SourceLocation::Region {
                page: *page,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            },
            ContentRange { start: 0, end: 1 },
        )),
        EvidenceKind::WebSnapshot { url, .. } => Ok((
            SourceLocation::Symbol {
                path: url.clone(),
                qualified_name: "web_snapshot".to_string(),
            },
            ContentRange { start: 0, end: 1 },
        )),
        _ => Ok((
            SourceLocation::Symbol {
                path: format!("evidence:{}", evidence.id),
                qualified_name: "evidence".to_string(),
            },
            ContentRange { start: 0, end: 1 },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_ports::{BlobStore, InMemoryBlobStore};

    fn file_evidence(
        range: ContentRange,
        excerpt: &str,
    ) -> Result<(SourceSnapshotVerifier, Evidence), Box<dyn std::error::Error>> {
        let source = b"alpha line\nbeta line\n";
        let blobs = Arc::new(InMemoryBlobStore::new());
        let snapshot = blobs.put(source.to_vec())?;
        let evidence = Evidence {
            id: maestria_domain::EvidenceId::new(1),
            artifact_id: maestria_domain::ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "notes.md".to_string(),
                range,
                content_hash: maestria_domain::content_hash(source),
                snapshot: Some(snapshot),
            },
            excerpt: excerpt.to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: Default::default(),
        };
        Ok((SourceSnapshotVerifier::new(blobs), evidence))
    }

    #[test]
    fn file_snapshot_range_must_bound_its_excerpt() -> Result<(), Box<dyn std::error::Error>> {
        let (verifier, valid) = file_evidence(ContentRange { start: 1, end: 1 }, "alpha line")?;
        verifier.verify(&valid)?;

        let (verifier, out_of_bounds) =
            file_evidence(ContentRange { start: 1, end: 3 }, "alpha line")?;
        assert!(verifier.verify(&out_of_bounds).is_err());

        let (verifier, wrong_range) =
            file_evidence(ContentRange { start: 2, end: 2 }, "alpha line")?;
        assert!(verifier.verify(&wrong_range).is_err());
        Ok(())
    }
}
