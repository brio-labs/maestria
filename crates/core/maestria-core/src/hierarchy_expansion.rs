use crate::error::CoreResult;
use crate::ports::CorePorts;
use crate::provenance::evidence_id_for;
use crate::types::{EvidencePack, SourceGroundedCardHit, SourceGroundedSearchHit};
use maestria_domain::{ArtifactId, DomainEvent, IndexStatus, StructureNode, StructureNodeId};
use maestria_ports::{DocumentTree, EventFilter};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Expands seed hits through their persisted document trees.
///
/// Parent, child, and sibling nodes are traversed breadth-first. The query
/// selects a shallow depth for precise lookups and the configured depth for
/// broader queries. Every projected candidate is rechecked against the same
/// security and source-snapshot gates as initial retrieval.
pub(super) fn expand(
    ports: &CorePorts<'_>,
    mut pack: EvidencePack,
    query: &str,
    limit: usize,
    config: &crate::types::GraphConfig,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<EvidencePack> {
    if limit == 0 || config.max_results == 0 {
        return Ok(pack);
    }
    let depth_limit = selected_depth(query, config.max_depth);
    if depth_limit == 0 {
        return Ok(pack);
    }

    let mut seed_nodes: BTreeMap<ArtifactId, BTreeSet<StructureNodeId>> = BTreeMap::new();
    for hit in &pack.cards {
        seed_nodes
            .entry(hit.artifact.id)
            .or_default()
            .insert(hit.card.node_id);
    }
    for hit in &pack.chunks {
        seed_nodes
            .entry(hit.artifact.id)
            .or_default()
            .insert(hit.chunk.node_id);
    }

    let mut added = 0usize;
    for (artifact_id, seeds) in seed_nodes {
        if added >= config.max_results {
            break;
        }
        let Some(tree) = latest_tree(ports, artifact_id)? else {
            continue;
        };
        let nodes: BTreeMap<_, _> = tree.nodes.iter().map(|node| (node.id, node)).collect();
        let mut queue: VecDeque<(StructureNodeId, u32, usize)> = seeds
            .into_iter()
            .filter_map(|node_id| {
                nodes
                    .get(&node_id)
                    .map(|_| (node_id, seed_score(&pack, artifact_id, node_id), 0))
            })
            .collect();
        let mut visited = BTreeSet::new();
        while let Some((node_id, seed_score, depth)) = queue.pop_front() {
            if depth >= depth_limit || added >= config.max_results {
                continue;
            }
            let Some(node) = nodes.get(&node_id) else {
                continue;
            };
            for neighbor in neighbors(node, &tree.nodes, node_id) {
                if !visited.insert(neighbor) {
                    continue;
                }
                let next_depth = depth + 1;
                let score = seed_score
                    / u32::try_from(next_depth + 1)
                        .map_or(u32::MAX, |value| value)
                        .max(1);
                let remaining = config.max_results.saturating_sub(added);
                let projected = project_node(
                    ports,
                    artifact_id,
                    neighbor,
                    score,
                    remaining,
                    &mut pack,
                    policy,
                )?;
                added = added.saturating_add(projected);
                if next_depth < depth_limit {
                    queue.push_back((neighbor, score, next_depth));
                }
                if added >= config.max_results {
                    break;
                }
            }
        }
    }
    Ok(pack)
}

fn selected_depth(query: &str, configured: usize) -> usize {
    let query = query.trim();
    if query.is_empty() || configured == 0 {
        return 0;
    }
    let terms = query.split_whitespace().count();
    let precise = (query.starts_with('"') && query.ends_with('"')) || terms <= 1;
    if precise {
        configured.min(1)
    } else {
        configured
    }
}

fn latest_tree(ports: &CorePorts<'_>, artifact_id: ArtifactId) -> CoreResult<Option<DocumentTree>> {
    let Some(expected_hash) = ports
        .artifacts
        .get(artifact_id)?
        .and_then(|artifact| artifact.content_hash)
    else {
        return Ok(None);
    };
    let events = ports.events.scan(EventFilter {
        artifact_id: Some(artifact_id),
    })?;
    let Some(event) = events
        .into_iter()
        .filter_map(|envelope| match envelope.event {
            DomainEvent::DocumentTreeCaptured {
                root_id,
                nodes,
                content_hash,
                ..
            } if content_hash.as_str() == expected_hash => {
                Some((envelope.sequence, root_id, nodes))
            }
            _ => None,
        })
        .max_by_key(|(sequence, _, _)| *sequence)
    else {
        return Ok(None);
    };
    Ok(DocumentTree::new(event.1, event.2).ok())
}

fn seed_score(pack: &EvidencePack, artifact_id: ArtifactId, node_id: StructureNodeId) -> u32 {
    pack.cards
        .iter()
        .filter(|hit| hit.artifact.id == artifact_id && hit.card.node_id == node_id)
        .map(|hit| hit.score)
        .chain(
            pack.chunks
                .iter()
                .filter(|hit| hit.artifact.id == artifact_id && hit.chunk.node_id == node_id)
                .map(|hit| hit.score),
        )
        .max()
        .map_or(0, |value| value)
}

fn neighbors(
    node: &StructureNode,
    all_nodes: &[StructureNode],
    node_id: StructureNodeId,
) -> Vec<StructureNodeId> {
    let mut result = Vec::new();
    if let Some(parent_id) = node.parent_id {
        result.push(parent_id);
    }
    result.extend(
        all_nodes
            .iter()
            .filter(|candidate| candidate.parent_id == Some(node_id))
            .map(|candidate| candidate.id),
    );
    if let Some(parent_id) = node.parent_id {
        result.extend(
            all_nodes
                .iter()
                .filter(|candidate| {
                    candidate.parent_id == Some(parent_id) && candidate.id != node_id
                })
                .map(|candidate| candidate.id),
        );
    }
    result.sort();
    result.dedup();
    result
}

fn project_node(
    ports: &CorePorts<'_>,
    artifact_id: ArtifactId,
    node_id: StructureNodeId,
    score: u32,
    remaining: usize,
    pack: &mut EvidencePack,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> CoreResult<usize> {
    let Some(artifact) = ports.artifacts.get(artifact_id)? else {
        return Ok(0);
    };
    if artifact.index_status != IndexStatus::Indexed
        || policy.evaluate(&artifact.security) != maestria_governance::RetrievalDecision::Allowed
    {
        return Ok(0);
    }
    let mut added = 0;
    let mut cards = ports.cards.list_for_artifact(artifact_id)?;
    cards.sort_by_key(|card| card.id);
    for card in cards.into_iter().filter(|card| card.node_id == node_id) {
        if added >= remaining {
            break;
        }
        if !maestria_governance::scan_secrets(&card.title).is_clean()
            || !maestria_governance::scan_secrets(&card.body).is_clean()
            || policy.evaluate(&card.security) != maestria_governance::RetrievalDecision::Allowed
            || pack.cards.iter().any(|hit| hit.card.id == card.id)
        {
            continue;
        }
        pack.cards.push(SourceGroundedCardHit {
            artifact: artifact.clone(),
            card,
            score,
            lexical_metadata: None,
        });
        added += 1;
    }

    let mut chunks = ports.chunks.list_for_artifact(artifact_id)?;
    chunks.sort_by_key(|chunk| (chunk.order, chunk.id));
    for chunk in chunks.into_iter().filter(|chunk| chunk.node_id == node_id) {
        if added >= remaining {
            break;
        }
        if pack.chunks.iter().any(|hit| hit.chunk.id == chunk.id)
            || !maestria_governance::scan_secrets(&chunk.text).is_clean()
        {
            continue;
        }
        let evidence_id = evidence_id_for(artifact_id, chunk.order);
        let Some(evidence) = ports.evidence.get(evidence_id)? else {
            continue;
        };
        if evidence.artifact_id != artifact_id
            || !maestria_governance::scan_secrets(&evidence.excerpt).is_clean()
            || policy.evaluate(&evidence.security)
                != maestria_governance::RetrievalDecision::Allowed
            || crate::retrieval::verify_source_snapshot(ports, &evidence).is_err()
        {
            continue;
        }
        if !pack.evidence_ids.contains(&evidence_id) {
            pack.evidence_ids.push(evidence_id);
        }
        pack.chunks.push(SourceGroundedSearchHit {
            artifact: artifact.clone(),
            chunk,
            evidence,
            score,
            lexical_metadata: None,
        });
        added += 1;
    }
    Ok(added)
}
#[cfg(test)]
mod tests {
    use super::selected_depth;

    #[test]
    fn precise_queries_use_shallow_context_and_broad_queries_use_configured_depth() {
        assert_eq!(selected_depth("\"exact phrase\"", 4), 1);
        assert_eq!(selected_depth("single", 4), 1);
        assert_eq!(selected_depth("broad query with context", 4), 4);
        assert_eq!(selected_depth("", 4), 0);
    }
}
