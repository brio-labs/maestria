use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use crate::provenance::content_hash;
use crate::types::{OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput};
use maestria_domain::{Evidence, EvidenceKind, IndexStatus};

pub(super) fn open_evidence<'a>(
    ports: &CorePorts<'a>,
    input: OpenEvidenceInput,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<OpenEvidenceOutput> {
    let evidence = ports
        .evidence
        .get(input.evidence_id)?
        .ok_or_else(|| CoreError::NotFound {
            message: format!("evidence {}", input.evidence_id),
        })?;
    if policy.evaluate(&evidence.security) != maestria_governance::RetrievalDecision::Allowed {
        return Err(CoreError::NotFound {
            message: "evidence is not available under retrieval policy".to_string(),
        });
    }
    if !maestria_governance::scan_secrets(&evidence.excerpt).is_clean() {
        return Err(CoreError::NotFound {
            message: "evidence is not available because it contains secret material".to_string(),
        });
    }
    let artifact =
        ports
            .artifacts
            .get(evidence.artifact_id)?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("artifact {} for evidence", evidence.artifact_id),
            })?;
    if policy.evaluate(&artifact.security) != maestria_governance::RetrievalDecision::Allowed {
        return Err(CoreError::NotFound {
            message: "artifact is not available under retrieval policy".to_string(),
        });
    }
    verify_source_snapshot(ports, &evidence)?;
    if artifact.index_status != IndexStatus::Indexed {
        return Err(CoreError::NotFound {
            message: format!(
                "artifact {} not indexed (status {:?})",
                artifact.id, artifact.index_status
            ),
        });
    }
    Ok(OpenEvidenceOutput { artifact, evidence })
}

pub(super) fn open_chunk_evidence<'a>(
    ports: &CorePorts<'a>,
    input: OpenChunkEvidenceInput,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<OpenEvidenceOutput> {
    let chunk = ports
        .chunks
        .get(input.chunk_id)?
        .ok_or_else(|| CoreError::NotFound {
            message: format!("chunk {}", input.chunk_id),
        })?;
    let evidence = ports
        .evidence
        .get(maestria_domain::evidence_id_for(
            chunk.artifact_id,
            chunk.order,
        ))?
        .ok_or_else(|| CoreError::NotFound {
            message: format!("evidence for chunk {}", input.chunk_id),
        })?;
    open_evidence(
        ports,
        OpenEvidenceInput {
            evidence_id: evidence.id,
        },
        policy,
    )
}

fn verify_source_snapshot(ports: &CorePorts<'_>, evidence: &Evidence) -> CoreResult<()> {
    if let EvidenceKind::PdfSpan { blob, .. } | EvidenceKind::PdfRegion { blob, .. } =
        &evidence.kind
    {
        let bytes = ports.blobs.get(*blob)?;
        if bytes.is_empty() {
            return Err(CoreError::InvalidInput {
                message: format!("evidence {} PDF snapshot is empty", evidence.id),
            });
        }
        return Ok(());
    }
    if let EvidenceKind::WebSnapshot {
        snapshot,
        content_hash: expected_hash,
        ..
    } = &evidence.kind
    {
        let bytes = ports.blobs.get(*snapshot)?;
        let actual_hash = content_hash(&bytes);
        if &actual_hash != expected_hash {
            return Err(CoreError::InvalidInput {
                message: format!(
                    "evidence {} web snapshot hash mismatch: expected {expected_hash}, got {actual_hash}",
                    evidence.id
                ),
            });
        }
        let source = String::from_utf8_lossy(&bytes);
        if !evidence.excerpt.is_empty() && !source.contains(&evidence.excerpt) {
            return Err(CoreError::InvalidInput {
                message: format!(
                    "evidence {} excerpt is absent from its web snapshot",
                    evidence.id
                ),
            });
        }
        return Ok(());
    }

    let Evidence {
        kind:
            EvidenceKind::FileSpan {
                range,
                content_hash: expected_hash,
                snapshot: Some(snapshot),
                ..
            },
        excerpt,
        ..
    } = evidence
    else {
        return Ok(());
    };

    let bytes = ports.blobs.get(*snapshot)?;
    let actual_hash = content_hash(&bytes);
    if &actual_hash != expected_hash {
        return Err(CoreError::InvalidInput {
            message: format!(
                "evidence {} snapshot hash mismatch: expected {expected_hash}, got {actual_hash}",
                evidence.id
            ),
        });
    }

    let source = String::from_utf8_lossy(&bytes);
    let line_count = source.lines().count().max(1);
    if range.start == 0 || range.end < range.start || range.end > line_count {
        return Err(CoreError::InvalidInput {
            message: format!("evidence {} has an invalid source line range", evidence.id),
        });
    }
    let compact_source = source.split_whitespace().collect::<Vec<_>>().join(" ");
    if !excerpt.is_empty() && !compact_source.contains(excerpt) {
        return Err(CoreError::InvalidInput {
            message: format!(
                "evidence {} excerpt is absent from its source snapshot",
                evidence.id
            ),
        });
    }
    Ok(())
}
