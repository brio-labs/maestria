use crate::error::CoreResult;
use crate::ports::CorePorts;
use crate::provenance::evidence_id_for;
use crate::types::{EvidencePack, SourceGroundedCardHit, SourceGroundedSearchHit};
use maestria_domain::{IndexStatus, Relation, RelationEndpoint};
use std::collections::{BTreeSet, VecDeque};

pub(super) fn expand_graph(
    ports: &CorePorts<'_>,
    mut pack: EvidencePack,
    limit: usize,
    config: &crate::types::GraphConfig,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<EvidencePack> {
    let Some(graph) = ports.graph_index else {
        return Ok(pack);
    };
    let mut queue = VecDeque::new();
    let mut visited = BTreeSet::new();
    for hit in &pack.cards {
        let endpoint = RelationEndpoint::Card(hit.card.id);
        visited.insert(endpoint);
        queue.push_back((endpoint, hit.score, 0));
    }
    for hit in &pack.chunks {
        let endpoint = RelationEndpoint::Artifact(hit.chunk.artifact_id);
        visited.insert(endpoint);
        queue.push_back((endpoint, hit.score, 0));
    }

    let mut graph_cards = Vec::new();
    let mut graph_chunks = Vec::new();
    let mut total_added = 0;
    while let Some((endpoint, score, depth)) = queue.pop_front() {
        if depth >= config.max_depth || total_added >= config.max_results {
            continue;
        }
        for relation in grounded_relations(ports, graph, endpoint, policy)? {
            if total_added >= config.max_results {
                break;
            }
            let neighbor = if relation.source == endpoint {
                relation.target
            } else {
                relation.source
            };
            if !visited.insert(neighbor) {
                continue;
            }
            let next_score = fused_graph_score(score, relation.confidence_milli, depth + 1);
            total_added += 1;
            if project_graph_neighbor(
                ports,
                neighbor,
                next_score,
                &mut graph_cards,
                &mut graph_chunks,
                policy,
            )? || !matches!(
                neighbor,
                RelationEndpoint::Card(_) | RelationEndpoint::Artifact(_)
            ) {
                queue.push_back((neighbor, next_score, depth + 1));
            }
        }
    }

    pack.cards.extend(graph_cards);
    pack.cards.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.card.id.cmp(&b.card.id))
    });
    pack.cards.truncate(limit);
    for hit in graph_chunks {
        let evidence_id = evidence_id_for(hit.chunk.artifact_id, hit.chunk.order);
        if !pack.evidence_ids.contains(&evidence_id) {
            pack.evidence_ids.push(evidence_id);
        }
        if !pack
            .chunks
            .iter()
            .any(|existing| existing.chunk.id == hit.chunk.id)
        {
            pack.chunks.push(hit);
        }
    }
    pack.chunks.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.chunk.id.cmp(&b.chunk.id))
    });
    pack.chunks.truncate(limit);
    Ok(pack)
}

fn grounded_relations(
    ports: &CorePorts<'_>,
    graph: &dyn maestria_ports::GraphIndex,
    endpoint: RelationEndpoint,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<Vec<Relation>> {
    let mut grounded = Vec::new();
    for relation in graph.get_relations_for(endpoint)? {
        if policy.evaluate(&relation.security) != maestria_governance::RetrievalDecision::Allowed {
            continue;
        }
        let Some(evidence_id) = relation.evidence_id else {
            continue;
        };
        let Some(evidence) = ports.evidence.get(evidence_id)? else {
            continue;
        };
        if !maestria_governance::scan_secrets(&evidence.excerpt).is_clean() {
            continue;
        }
        if policy.evaluate(&evidence.security) == maestria_governance::RetrievalDecision::Allowed
            && crate::retrieval::verify_source_snapshot(ports, &evidence).is_ok()
        {
            grounded.push(relation);
        }
    }
    grounded.sort_by_key(|relation| relation.id);
    Ok(grounded)
}
fn project_graph_neighbor(
    ports: &CorePorts<'_>,
    neighbor: RelationEndpoint,
    score: u32,
    graph_cards: &mut Vec<SourceGroundedCardHit>,
    graph_chunks: &mut Vec<SourceGroundedSearchHit>,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<bool> {
    match neighbor {
        RelationEndpoint::Card(card_id) => {
            let Some(card) = ports.cards.get(card_id)? else {
                return Ok(false);
            };
            if !maestria_governance::scan_secrets(&card.title).is_clean()
                || !maestria_governance::scan_secrets(&card.body).is_clean()
            {
                return Ok(false);
            }
            if policy.evaluate(&card.security) != maestria_governance::RetrievalDecision::Allowed {
                return Ok(false);
            }
            let Some(artifact) = ports.artifacts.get(card.artifact_id)? else {
                return Ok(false);
            };
            if artifact.index_status != IndexStatus::Indexed
                || policy.evaluate(&artifact.security)
                    != maestria_governance::RetrievalDecision::Allowed
            {
                return Ok(false);
            }
            graph_cards.push(SourceGroundedCardHit {
                artifact,
                card,
                score,
                lexical_metadata: None,
            });
            Ok(true)
        }
        RelationEndpoint::Artifact(artifact_id) => {
            let Some(artifact) = ports.artifacts.get(artifact_id)? else {
                return Ok(false);
            };
            if artifact.index_status != IndexStatus::Indexed
                || policy.evaluate(&artifact.security)
                    != maestria_governance::RetrievalDecision::Allowed
            {
                return Ok(false);
            }
            let mut chunks = ports.chunks.list_for_artifact(artifact_id)?;
            chunks.sort_by_key(|chunk| chunk.order);
            let Some(chunk) = chunks.into_iter().next() else {
                return Ok(false);
            };
            if !maestria_governance::scan_secrets(&chunk.text).is_clean() {
                return Ok(false);
            }
            let evidence_id = evidence_id_for(artifact_id, chunk.order);
            let Some(evidence) = ports.evidence.get(evidence_id)? else {
                return Ok(false);
            };
            if !maestria_governance::scan_secrets(&evidence.excerpt).is_clean() {
                return Ok(false);
            }
            if policy.evaluate(&evidence.security)
                != maestria_governance::RetrievalDecision::Allowed
                || crate::retrieval::verify_source_snapshot(ports, &evidence).is_err()
            {
                return Ok(false);
            }
            graph_chunks.push(SourceGroundedSearchHit {
                artifact,
                chunk,
                evidence,
                score,
                lexical_metadata: None,
            });
            Ok(true)
        }
        RelationEndpoint::Claim(_) | RelationEndpoint::Task(_) | RelationEndpoint::Memory(_) => {
            Ok(true)
        }
    }
}

fn fused_graph_score(seed_score: u32, confidence_milli: u16, depth: usize) -> u32 {
    let confidence = u64::from(confidence_milli.min(1000));
    let depth = u64::try_from(depth).map_or(u64::MAX, |value| value.max(1));
    let depth_decay = 1000_u64 / depth;
    let score = u64::from(seed_score)
        .saturating_mul(confidence)
        .saturating_mul(depth_decay)
        / 1_000_000;
    score.min(u64::from(u32::MAX)) as u32
}
