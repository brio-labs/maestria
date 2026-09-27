use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use crate::types::{OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput};
use maestria_domain::IndexStatus;

#[path = "evidence_source_verification.rs"]
mod source_verification;

pub(super) fn open_evidence<'a>(
    ports: &CorePorts<'a>,
    input: OpenEvidenceInput,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
) -> CoreResult<OpenEvidenceOutput> {
    let evidence =
        ports
            .evidence
            .get(input.evidence_id)?
            .ok_or_else(|| CoreError::NotFoundEntity {
                kind: "evidence",
                id: input.evidence_id.to_string(),
            })?;
    if authorization.evaluate(&evidence.security) != maestria_governance::RetrievalDecision::Allowed
    {
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
    if authorization.evaluate(&artifact.security) != maestria_governance::RetrievalDecision::Allowed
    {
        return Err(CoreError::NotAvailable {
            kind: "artifact",
            reason: "not available under retrieval policy",
        });
    }
    source_verification::verify_source_snapshot(ports, &evidence, &artifact)?;
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
    authorization: &maestria_governance::RetrievalAuthorizationContext,
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
        authorization,
    )
}
