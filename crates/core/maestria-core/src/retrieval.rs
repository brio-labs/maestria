use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use crate::provenance::evidence_id_for;
use crate::types::{
    OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, SearchInput, SearchOutput,
};

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
