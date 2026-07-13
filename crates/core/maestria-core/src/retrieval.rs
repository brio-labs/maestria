use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use crate::provenance::{content_hash, evidence_id_for};
use crate::types::{
    EvidencePack, OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, SearchInput,
    SearchOutput, SourceGroundedCardHit, SourceGroundedSearchHit,
};

use maestria_domain::{Evidence, EvidenceKind, IndexStatus};
use maestria_ports::{SearchQuery, VectorSearchQuery};

pub(super) fn search<'a>(
    ports: &CorePorts<'a>,
    input: SearchInput,
    vector_query: Option<VectorSearchQuery>,
) -> CoreResult<SearchOutput> {
    let cards = search_cards(ports, &input.query, input.limit)?;
    let (mut chunks, mut evidence_ids) =
        search_vector_chunks(ports, &input.query, input.limit, vector_query)?;
    let (full_text_chunks, _) = search_chunks(ports, &input.query, input.limit)?;
    for chunk in full_text_chunks {
        if chunks.len() >= input.limit {
            break;
        }
        let evidence_id = evidence_id_for(chunk.chunk.artifact_id, chunk.chunk.order);
        if evidence_ids.contains(&evidence_id) {
            continue;
        }
        evidence_ids.push(evidence_id);
        chunks.push(chunk);
    }

    Ok(SearchOutput {
        pack: EvidencePack {
            query: input.query,
            cards,
            chunks,
            evidence_ids,
        },
    })
}
fn search_cards(
    ports: &CorePorts<'_>,
    query: &str,
    limit: usize,
) -> CoreResult<Vec<SourceGroundedCardHit>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut offset = 0;
    let mut cards = Vec::with_capacity(limit);
    loop {
        let page = ports.search_index.search_cards(SearchQuery {
            q: query.to_string(),
            limit,
            offset,
        })?;
        if page.is_empty() {
            break;
        }
        for hit in &page {
            if cards.len() >= limit {
                break;
            }
            let Some(artifact) = ports.artifacts.get(hit.card.artifact_id)? else {
                continue;
            };
            if artifact.index_status != IndexStatus::Indexed {
                continue;
            }
            let Some(card) = ports.cards.get(hit.card.card_id)? else {
                continue;
            };
            if card.artifact_id != hit.card.artifact_id {
                continue;
            }
            cards.push(SourceGroundedCardHit {
                artifact,
                card,
                score: hit.score,
            });
        }
        if cards.len() >= limit || page.len() < limit {
            break;
        }
        offset = offset.saturating_add(page.len());
    }
    Ok(cards)
}

fn search_chunks(
    ports: &CorePorts<'_>,
    query: &str,
    limit: usize,
) -> CoreResult<(
    Vec<SourceGroundedSearchHit>,
    Vec<maestria_domain::EvidenceId>,
)> {
    if limit == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut offset = 0;
    let mut chunks = Vec::with_capacity(limit);
    let mut seen_evidence = std::collections::BTreeSet::new();
    let mut evidence_ids = Vec::with_capacity(limit);
    loop {
        let page = ports.search_index.search(SearchQuery {
            q: query.to_string(),
            limit,
            offset,
        })?;
        if page.is_empty() {
            break;
        }
        for hit in &page {
            if chunks.len() >= limit {
                break;
            }
            let Some(artifact) = ports.artifacts.get(hit.chunk.artifact_id)? else {
                continue;
            };
            if artifact.index_status != IndexStatus::Indexed {
                continue;
            }
            let Some(chunk) = ports.chunks.get(hit.chunk.chunk_id)? else {
                continue;
            };
            if chunk.artifact_id != hit.chunk.artifact_id || chunk.artifact_id != artifact.id {
                continue;
            }
            let Some(evidence) = ports
                .evidence
                .get(evidence_id_for(chunk.artifact_id, chunk.order))?
            else {
                continue;
            };
            verify_source_snapshot(ports, &evidence)?;
            if seen_evidence.insert(evidence.id) {
                evidence_ids.push(evidence.id);
            }
            chunks.push(SourceGroundedSearchHit {
                artifact,
                chunk,
                evidence,
                score: hit.score,
            });
        }
        if chunks.len() >= limit || page.len() < limit {
            break;
        }
        offset = offset.saturating_add(page.len());
    }
    Ok((chunks, evidence_ids))
}

fn search_vector_chunks(
    ports: &CorePorts<'_>,
    _query: &str,
    limit: usize,
    vector_query: Option<VectorSearchQuery>,
) -> CoreResult<(
    Vec<SourceGroundedSearchHit>,
    Vec<maestria_domain::EvidenceId>,
)> {
    if limit == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let Some(vector_query) = vector_query else {
        return Ok((Vec::new(), Vec::new()));
    };
    let Some(vector_index) = ports.vector_index else {
        return Ok((Vec::new(), Vec::new()));
    };
    let hits = match vector_index.search_similar(vector_query) {
        Ok(hits) => hits,
        Err(_) => return Ok((Vec::new(), Vec::new())),
    };
    let mut chunks = Vec::with_capacity(limit);
    let mut evidence_ids = Vec::with_capacity(limit);
    let mut seen_evidence = std::collections::BTreeSet::new();
    for hit in hits {
        if chunks.len() >= limit {
            break;
        }
        let Some(chunk) = ports.chunks.get(hit.chunk_id)? else {
            continue;
        };
        let Some(artifact) = ports.artifacts.get(chunk.artifact_id)? else {
            continue;
        };
        if artifact.index_status != IndexStatus::Indexed {
            continue;
        }
        let Some(evidence) = ports
            .evidence
            .get(evidence_id_for(chunk.artifact_id, chunk.order))?
        else {
            continue;
        };
        verify_source_snapshot(ports, &evidence)?;
        if !seen_evidence.insert(evidence.id) {
            continue;
        }
        evidence_ids.push(evidence.id);
        chunks.push(SourceGroundedSearchHit {
            artifact,
            chunk,
            evidence,
            score: vector_score(hit.score),
        });
    }
    Ok((chunks, evidence_ids))
}

fn vector_score(score: f32) -> u32 {
    if !score.is_finite() {
        return 0;
    }
    let normalized = ((score as f64 + 1.0) / 2.0).clamp(0.0, 1.0);
    (normalized * 1_000_000.0) as u32
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
