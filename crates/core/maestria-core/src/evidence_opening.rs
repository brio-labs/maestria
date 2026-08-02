use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use crate::types::{OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput};
use maestria_domain::{
    Evidence, EvidenceKind, IndexStatus, SnapshotRef, verify_snapshot_bytes, verify_text_snapshot,
};

pub(super) fn open_evidence<'a>(
    ports: &CorePorts<'a>,
    input: OpenEvidenceInput,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<OpenEvidenceOutput> {
    let evidence =
        ports
            .evidence
            .get(input.evidence_id)?
            .ok_or_else(|| CoreError::NotFoundEntity {
                kind: "evidence",
                id: input.evidence_id.to_string(),
            })?;
    if policy.evaluate(&evidence.security) != maestria_governance::RetrievalDecision::Allowed {
        return Err(CoreError::NotAvailable {
            kind: "evidence",
            reason: "not available under retrieval policy",
        });
    }
    if !maestria_governance::scan_secrets(&evidence.excerpt).is_clean() {
        return Err(CoreError::NotAvailable {
            kind: "evidence",
            reason: "contains secret material",
        });
    }
    let artifact =
        ports
            .artifacts
            .get(evidence.artifact_id)?
            .ok_or_else(|| CoreError::NotFoundEntity {
                kind: "artifact",
                id: evidence.artifact_id.to_string(),
            })?;
    if policy.evaluate(&artifact.security) != maestria_governance::RetrievalDecision::Allowed {
        return Err(CoreError::NotAvailable {
            kind: "artifact",
            reason: "not available under retrieval policy",
        });
    }
    verify_source_snapshot(ports, &evidence, &artifact)?;
    if artifact.index_status != IndexStatus::Indexed {
        return Err(CoreError::NotAvailable {
            kind: "artifact",
            reason: "not indexed",
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
        .ok_or_else(|| CoreError::NotFoundEntity {
            kind: "chunk",
            id: input.chunk_id.to_string(),
        })?;
    let evidence = ports
        .evidence
        .get(maestria_domain::evidence_id_for(
            chunk.artifact_id,
            chunk.order,
        ))?
        .ok_or_else(|| CoreError::NotFoundEntity {
            kind: "evidence for chunk",
            id: input.chunk_id.to_string(),
        })?;
    if evidence.artifact_id != chunk.artifact_id {
        return Err(CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!(
                "chunk evidence belongs to artifact {}, requested chunk belongs to artifact {}",
                evidence.artifact_id, chunk.artifact_id
            ),
        });
    }
    open_evidence(
        ports,
        OpenEvidenceInput {
            evidence_id: evidence.id,
        },
        policy,
    )
}

fn verify_source_snapshot(
    ports: &CorePorts<'_>,
    evidence: &Evidence,
    artifact: &maestria_domain::Artifact,
) -> CoreResult<()> {
    if let EvidenceKind::PdfSpan { snapshot, .. } | EvidenceKind::PdfRegion { snapshot, .. } =
        &evidence.kind
    {
        verify_snapshot_binding(evidence, artifact, snapshot)?;
        let bytes = ports.blobs.get(snapshot.blob_id())?;
        verify_snapshot_bytes(snapshot, &bytes).map_err(|error| CoreError::InvalidEvidence {
            evidence_id: evidence.id.to_string(),
            reason: format!("PDF snapshot verification failed: {error}"),
        })?;
        return Ok(());
    }
    if let EvidenceKind::WebSnapshot { snapshot, .. } = &evidence.kind {
        verify_snapshot_binding(evidence, artifact, snapshot)?;
        let bytes = ports.blobs.get(snapshot.blob_id())?;
        verify_text_snapshot(snapshot, &bytes, None, &evidence.excerpt).map_err(|error| {
            CoreError::InvalidEvidence {
                evidence_id: evidence.id.to_string(),
                reason: format!("web snapshot verification failed: {error}"),
            }
        })?;
        return Ok(());
    }
    if let EvidenceKind::FileSpan {
        range, snapshot, ..
    } = &evidence.kind
    {
        verify_snapshot_binding(evidence, artifact, snapshot)?;
        let bytes = ports.blobs.get(snapshot.blob_id())?;
        verify_text_snapshot(snapshot, &bytes, Some(range), &evidence.excerpt).map_err(
            |error| CoreError::InvalidEvidence {
                evidence_id: evidence.id.to_string(),
                reason: format!("file snapshot verification failed: {error}"),
            },
        )?;
    }
    Ok(())
}

fn verify_snapshot_binding(
    evidence: &Evidence,
    artifact: &maestria_domain::Artifact,
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
