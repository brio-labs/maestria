use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use crate::provenance::{content_hash, evidence_id_for};
use crate::types::{
    OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, SearchInput, SearchOutput,
};

use maestria_domain::{Evidence, EvidenceKind};
use maestria_ports::SearchQuery;

pub(super) fn search<'a>(ports: &CorePorts<'a>, input: SearchInput) -> CoreResult<SearchOutput> {
    let hits = ports.search_index.search(SearchQuery {
        q: input.query,
        limit: input.limit,
    })?;
    let mut results = Vec::with_capacity(hits.len());
    for hit in hits {
        let artifact =
            ports
                .artifacts
                .get(hit.chunk.artifact_id)?
                .ok_or_else(|| CoreError::NotFound {
                    message: format!("artifact {} for search hit", hit.chunk.artifact_id),
                })?;
        let chunk = ports
            .chunks
            .get(hit.chunk.chunk_id)?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("chunk {} for search hit", hit.chunk.chunk_id),
            })?;
        let evidence = ports
            .evidence
            .get(evidence_id_for(chunk.artifact_id, chunk.order))?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("evidence for search chunk {}", chunk.id),
            })?;
        verify_source_snapshot(ports, &evidence)?;
        results.push(crate::types::SourceGroundedSearchHit {
            artifact,
            chunk,
            evidence,
            score: hit.score,
        });
    }
    Ok(SearchOutput { hits: results })
}

pub(super) fn open_evidence<'a>(
    ports: &CorePorts<'a>,
    input: OpenEvidenceInput,
) -> CoreResult<OpenEvidenceOutput> {
    let evidence = ports
        .evidence
        .get(input.evidence_id)?
        .ok_or_else(|| CoreError::NotFound {
            message: format!("evidence {}", input.evidence_id),
        })?;
    verify_source_snapshot(ports, &evidence)?;
    let artifact =
        ports
            .artifacts
            .get(evidence.artifact_id)?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("artifact {} for evidence", evidence.artifact_id),
            })?;
    Ok(OpenEvidenceOutput { artifact, evidence })
}

pub(super) fn open_chunk_evidence<'a>(
    ports: &CorePorts<'a>,
    input: OpenChunkEvidenceInput,
) -> CoreResult<OpenEvidenceOutput> {
    let chunk = ports
        .chunks
        .get(input.chunk_id)?
        .ok_or_else(|| CoreError::NotFound {
            message: format!("chunk {}", input.chunk_id),
        })?;
    let evidence = ports
        .evidence
        .get(evidence_id_for(chunk.artifact_id, chunk.order))?
        .ok_or_else(|| CoreError::NotFound {
            message: format!("evidence for chunk {}", input.chunk_id),
        })?;
    open_evidence(
        ports,
        OpenEvidenceInput {
            evidence_id: evidence.id,
        },
    )
}
fn verify_source_snapshot(ports: &CorePorts<'_>, evidence: &Evidence) -> CoreResult<()> {
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
