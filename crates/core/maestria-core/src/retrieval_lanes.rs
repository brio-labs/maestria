use crate::error::{CoreError, CoreResult};
use crate::lexical_helpers::{
    CardCandidate, ChunkCandidate, card_candidate_allowed, card_lexical_query,
    chunk_candidate_allowed, chunk_lexical_query, lexical_score, prepare_lexical_query,
    snapshot_id_from_evidence, vector_score,
};
use crate::ports::CorePorts;
use crate::provenance::{content_hash, content_is_safe, evidence_id_for};
use crate::types::{
    OpenChunkEvidenceInput, OpenEvidenceInput, OpenEvidenceOutput, SourceGroundedCardHit,
    SourceGroundedSearchHit,
};
use maestria_domain::{Evidence, EvidenceKind, IndexStatus};
use maestria_ports::{CardHit, SearchHit, SearchQuery, VectorSearchQuery};

pub(super) fn search_cards(
    ports: &CorePorts<'_>,
    query: &str,
    limit: usize,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<Vec<SourceGroundedCardHit>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut offset = 0;
    let mut cards = Vec::with_capacity(limit);
    let filter = |card_id, artifact_id| card_candidate_allowed(ports, card_id, artifact_id, policy);

    let (mode, q) = prepare_lexical_query(query);
    loop {
        let page = if ports.search_index.supports_lexical_metadata() {
            ports
                .search_index
                .search_cards_lexical_filtered(
                    card_lexical_query(&q, limit, offset, mode),
                    &filter,
                )?
                .into_iter()
                .map(|hit| CardCandidate {
                    hit: CardHit {
                        card: maestria_ports::IndexedCard {
                            artifact_id: hit.card.artifact_id,
                            card_id: hit.card.card_id,
                            title: hit.card.title,
                            body: hit.card.body,
                        },
                        score: lexical_score(hit.metadata.raw_score),
                    },
                    lexical_metadata: Some(hit.metadata),
                })
                .collect::<Vec<_>>()
        } else {
            ports
                .search_index
                .search_cards_filtered(
                    SearchQuery {
                        q: query.to_string(),
                        limit,
                        offset,
                    },
                    &filter,
                )?
                .into_iter()
                .map(|hit| CardCandidate {
                    hit,
                    lexical_metadata: None,
                })
                .collect()
        };
        if page.is_empty() {
            break;
        }
        for hit in &page {
            if cards.len() >= limit {
                break;
            }
            let Some(artifact) = ports.artifacts.get(hit.hit.card.artifact_id)? else {
                continue;
            };
            if artifact.index_status != IndexStatus::Indexed {
                continue;
            }
            let Some(card) = ports.cards.get(hit.hit.card.card_id)? else {
                continue;
            };
            if card.artifact_id != hit.hit.card.artifact_id {
                continue;
            }
            cards.push(SourceGroundedCardHit {
                artifact,
                card,
                score: hit.hit.score,
                lexical_metadata: hit.lexical_metadata.clone(),
            });
        }
        if cards.len() >= limit || page.len() < limit {
            break;
        }
        offset = offset.saturating_add(page.len());
    }
    Ok(cards)
}

pub(super) fn search_chunks(
    ports: &CorePorts<'_>,
    query: &str,
    limit: usize,
    policy: &maestria_governance::RetrievalSecurityPolicy,
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
    let filter =
        |chunk_id, artifact_id| chunk_candidate_allowed(ports, chunk_id, artifact_id, policy);
    let (mode, q) = prepare_lexical_query(query);
    loop {
        let page: Vec<ChunkCandidate> = if ports.search_index.supports_lexical_metadata() {
            ports
                .search_index
                .search_lexical_filtered(chunk_lexical_query(&q, limit, offset, mode), &filter)?
                .into_iter()
                .map(|hit| ChunkCandidate {
                    hit: SearchHit {
                        chunk: maestria_ports::IndexedChunk {
                            artifact_id: hit.chunk.artifact_id,
                            chunk_id: hit.chunk.chunk_id,
                            text: hit.chunk.text,
                        },
                        score: lexical_score(hit.metadata.raw_score),
                    },
                    lexical_metadata: Some(hit.metadata),
                })
                .collect()
        } else {
            ports
                .search_index
                .search_filtered(
                    SearchQuery {
                        q: query.to_string(),
                        limit,
                        offset,
                    },
                    &filter,
                )?
                .into_iter()
                .map(|hit| ChunkCandidate {
                    hit,
                    lexical_metadata: None,
                })
                .collect()
        };
        if page.is_empty() {
            break;
        }
        for hit in &page {
            if chunks.len() >= limit {
                break;
            }
            let Some(artifact) = ports.artifacts.get(hit.hit.chunk.artifact_id)? else {
                continue;
            };
            if artifact.index_status != IndexStatus::Indexed {
                continue;
            }
            let Some(chunk) = ports.chunks.get(hit.hit.chunk.chunk_id)? else {
                continue;
            };
            if chunk.artifact_id != hit.hit.chunk.artifact_id || chunk.artifact_id != artifact.id {
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
            let lexical_metadata = hit.lexical_metadata.clone().map(|mut metadata| {
                metadata.snapshot_id = snapshot_id_from_evidence(&evidence);
                metadata
            });
            chunks.push(SourceGroundedSearchHit {
                artifact,
                chunk,
                evidence,
                score: hit.hit.score,
                lexical_metadata,
            });
        }
        if chunks.len() >= limit || page.len() < limit {
            break;
        }
        offset = offset.saturating_add(page.len());
    }
    Ok((chunks, evidence_ids))
}

pub(super) fn search_vector_chunks(
    ports: &CorePorts<'_>,
    _query: &str,
    limit: usize,
    vector_query: Option<VectorSearchQuery>,
    policy: &maestria_governance::RetrievalSecurityPolicy,
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

    let filter = |chunk_id: maestria_domain::ChunkId| -> bool {
        let Ok(Some(chunk)) = ports.chunks.get(chunk_id) else {
            return false;
        };
        if !maestria_governance::scan_secrets(&chunk.text).is_clean() {
            return false;
        }
        let Ok(Some(artifact)) = ports.artifacts.get(chunk.artifact_id) else {
            return false;
        };
        if policy.evaluate(&artifact.security) != maestria_governance::RetrievalDecision::Allowed {
            return false;
        }
        let Ok(Some(evidence)) = ports
            .evidence
            .get(evidence_id_for(chunk.artifact_id, chunk.order))
        else {
            return false;
        };
        policy.evaluate(&evidence.security) == maestria_governance::RetrievalDecision::Allowed
            && content_is_safe(&chunk.text)
            && content_is_safe(&evidence.excerpt)
    };

    let hits = vector_index.search_similar_filtered(vector_query, &filter)?;
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
            lexical_metadata: None,
        });
    }
    Ok((chunks, evidence_ids))
}

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
        .get(evidence_id_for(chunk.artifact_id, chunk.order))?
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
pub(super) fn verify_source_snapshot(ports: &CorePorts<'_>, evidence: &Evidence) -> CoreResult<()> {
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
