use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use sillage_domain::{
    Evidence, EvidenceKind, SnapshotRef, excerpt_for, verify_snapshot_bytes, verify_text_snapshot,
};
use sillage_ports::{FileHandle, ParseContext, SourceSpan};
use std::path::PathBuf;

pub(super) fn verify_source_snapshot(
    ports: &CorePorts<'_>,
    evidence: &Evidence,
    artifact: &sillage_domain::Artifact,
) -> CoreResult<()> {
    match &evidence.kind {
        EvidenceKind::PdfSpan { snapshot, .. } | EvidenceKind::PdfRegion { snapshot, .. } => {
            verify_binary_snapshot(ports, evidence, artifact, snapshot, "PDF")
        }
        EvidenceKind::WebSnapshot { snapshot, .. } => {
            verify_web_snapshot(ports, evidence, artifact, snapshot)
        }
        EvidenceKind::DocxParagraphSpan {
            path,
            range,
            snapshot,
        } => verify_docx_snapshot(ports, evidence, artifact, path, range, snapshot),
        EvidenceKind::FileSpan {
            range, snapshot, ..
        } => verify_file_snapshot(ports, evidence, artifact, range, snapshot),
        _ => Ok(()),
    }
}

fn verify_binary_snapshot(
    ports: &CorePorts<'_>,
    evidence: &Evidence,
    artifact: &sillage_domain::Artifact,
    snapshot: &SnapshotRef,
    source_kind: &str,
) -> CoreResult<()> {
    verify_snapshot_binding(evidence, artifact, snapshot)?;
    let bytes = ports.blobs.get(snapshot.blob_id())?;
    verify_snapshot_bytes(snapshot, &bytes).map_err(|error| CoreError::InvalidEvidence {
        evidence_id: evidence.id.to_string(),
        reason: format!("{source_kind} snapshot verification failed: {error}"),
    })
}

fn verify_web_snapshot(
    ports: &CorePorts<'_>,
    evidence: &Evidence,
    artifact: &sillage_domain::Artifact,
    snapshot: &SnapshotRef,
) -> CoreResult<()> {
    verify_snapshot_binding(evidence, artifact, snapshot)?;
    let bytes = ports.blobs.get(snapshot.blob_id())?;
    verify_text_snapshot(snapshot, &bytes, None, &evidence.excerpt).map_err(|error| {
        CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!("web snapshot verification failed: {error}"),
        }
    })
}

fn verify_docx_snapshot(
    ports: &CorePorts<'_>,
    evidence: &Evidence,
    artifact: &sillage_domain::Artifact,
    path: &str,
    range: &sillage_domain::ParagraphRange,
    snapshot: &SnapshotRef,
) -> CoreResult<()> {
    let source_path = PathBuf::from(path);
    if !source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
    {
        return Err(CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: "DOCX paragraph evidence does not reference a .docx source".to_string(),
        });
    }
    verify_snapshot_binding(evidence, artifact, snapshot)?;
    let bytes = ports.blobs.get(snapshot.blob_id())?;
    verify_snapshot_bytes(snapshot, &bytes).map_err(|error| CoreError::InvalidEvidence {
        evidence_id: evidence.id.to_string(),
        reason: format!("DOCX snapshot verification failed: {error}"),
    })?;
    let parsed = ports
        .parser
        .parse(
            FileHandle {
                path: source_path,
                bytes,
            },
            ParseContext {
                artifact_id: evidence.artifact_id,
            },
        )
        .map_err(|error| CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!("DOCX snapshot parse failed: {error}"),
        })?;
    verify_docx_paragraph_excerpt(evidence, range, &parsed.chunks)
}

fn verify_docx_paragraph_excerpt(
    evidence: &Evidence,
    range: &sillage_domain::ParagraphRange,
    chunks: &[sillage_ports::ParsedChunk],
) -> CoreResult<()> {
    let start_paragraph =
        usize::try_from(range.start()).map_err(|error| CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!("DOCX paragraph start is invalid: {error}"),
        })?;
    let end_paragraph =
        usize::try_from(range.end()).map_err(|error| CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!("DOCX paragraph end is invalid: {error}"),
        })?;
    let mut matched_span = false;
    for chunk in chunks {
        if let SourceSpan::DocxParagraphSpan {
            start_paragraph: actual_start,
            end_paragraph: actual_end,
        } = &chunk.source_span
            && *actual_start == start_paragraph
            && *actual_end == end_paragraph
        {
            if matched_span {
                return Err(CoreError::InvalidEvidence {
                    evidence_id: evidence.id.to_string(),
                    reason: "DOCX paragraph span is ambiguous in its snapshot".to_string(),
                });
            }
            matched_span = true;
            if excerpt_for(&chunk.text).as_str() != evidence.excerpt.as_str() {
                return Err(CoreError::InvalidEvidence {
                    evidence_id: evidence.id.to_string(),
                    reason: "DOCX excerpt does not match its paragraph span".to_string(),
                });
            }
        }
    }
    if !matched_span {
        return Err(CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: "DOCX paragraph span is absent from its snapshot".to_string(),
        });
    }
    Ok(())
}

fn verify_file_snapshot(
    ports: &CorePorts<'_>,
    evidence: &Evidence,
    artifact: &sillage_domain::Artifact,
    range: &sillage_domain::LineRange,
    snapshot: &SnapshotRef,
) -> CoreResult<()> {
    verify_snapshot_binding(evidence, artifact, snapshot)?;
    let bytes = ports.blobs.get(snapshot.blob_id())?;
    verify_text_snapshot(snapshot, &bytes, Some(range), &evidence.excerpt).map_err(|error| {
        CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!("file snapshot verification failed: {error}"),
        }
    })
}

fn verify_snapshot_binding(
    evidence: &Evidence,
    artifact: &sillage_domain::Artifact,
    snapshot: &SnapshotRef,
) -> CoreResult<()> {
    if evidence.artifact_id != artifact.id {
        return Err(CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!(
                "evidence belongs to artifact {}, loaded owning artifact is {}",
                evidence.artifact_id, artifact.id
            ),
        });
    }
    if artifact.content_hash.as_ref() != Some(snapshot.content_hash()) {
        return Err(CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!(
                "snapshot hash does not match owning artifact: expected {:?}, got {}",
                artifact.content_hash,
                snapshot.content_hash().as_str()
            ),
        });
    }
    Ok(())
}
