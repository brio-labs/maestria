use crate::security::SecurityMetadata;
use std::collections::{BTreeSet, btree_map::Entry};

use crate::types::*;

use crate::search::StructureNode;

impl KernelState {
    /// First-time commit from fresh detection.
    /// On retry or resume the pending artifact entry is absent, so this is skipped.
    pub(super) fn process_parser_pending_artifacts(
        &mut self,
        input: &ParserResult,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut generated = Vec::new();
        if let Some(pending) = self.pending_artifacts.remove(&input.artifact_id) {
            if let Entry::Vacant(entry) = self.artifacts.entry(input.artifact_id) {
                let mut artifact = Artifact::with_title(input.artifact_id, pending.title.clone());
                artifact.security = SecurityMetadata::default();
                entry.insert(artifact);
                let register_event = self.emit_event(DomainEvent::ArtifactRegistered {
                    artifact_id: input.artifact_id,
                    title: pending.title,
                    security: SecurityMetadata::default(),
                });
                generated.push(register_event);
            }
            if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                artifact.content_hash = Some(pending.content_hash.clone());
                artifact.index_status = IndexStatus::Pending;
            }
            let pending_event = self.emit_event(DomainEvent::PendingIndex {
                artifact_id: input.artifact_id,
                content_hash: pending.content_hash,
            });
            generated.push(pending_event);
        }
        Ok(generated)
    }

    /// Resume/recovery path using the durable ParserStarted marker.
    pub(super) fn process_parser_pending_parsers(
        &mut self,
        input: &ParserResult,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut generated = Vec::new();
        if let Some(parser) = self.pending_parsers.get(&input.artifact_id).cloned() {
            if let Entry::Vacant(entry) = self.artifacts.entry(input.artifact_id) {
                let mut artifact = Artifact::with_title(input.artifact_id, parser.title.clone());
                artifact.security = SecurityMetadata::default();
                entry.insert(artifact);
                let register_event = self.emit_event(DomainEvent::ArtifactRegistered {
                    artifact_id: input.artifact_id,
                    title: parser.title.clone(),
                    security: SecurityMetadata::default(),
                });
                generated.push(register_event);
            } else if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id)
                && artifact.title.is_empty()
                && !parser.title.is_empty()
            {
                artifact.title = parser.title.clone();
            }
            if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                let needs_pending = artifact.index_status != IndexStatus::Pending
                    || artifact.content_hash.as_deref() != Some(&parser.content_hash);
                if needs_pending {
                    artifact.content_hash = Some(parser.content_hash.clone());
                    artifact.index_status = IndexStatus::Pending;
                    let pending_event = self.emit_event(DomainEvent::PendingIndex {
                        artifact_id: input.artifact_id,
                        content_hash: parser.content_hash,
                    });
                    generated.push(pending_event);
                }
            }
        }
        Ok(generated)
    }
    pub(super) fn register_parser_records(
        &mut self,
        input: &ParserResult,
        generated: &mut Vec<DomainEventEnvelope>,
    ) -> Result<(u32, u32), DomainError> {
        let mut new_chunks = 0u32;
        let mut new_cards = 0u32;
        if input.status == crate::provenance::ParseStatus::Parsed {
            for chunk in &input.chunks {
                if !input.tree_nodes.iter().any(|node| node.id == chunk.node_id)
                    && chunk.node_id != StructureNodeId::new(chunk.chunk_id.value())
                {
                    return Err(DomainError::InternalInvariantViolation {
                        detail: "parser chunk references a missing document tree node",
                    });
                }
                if let Some(existing) = self.chunks.get(&chunk.chunk_id) {
                    if existing.artifact_id != chunk.artifact_id
                        || existing.node_id != chunk.node_id
                        || existing.source_span != chunk.source_span
                        || existing.representations != chunk.representations
                        || existing.order != chunk.order
                        || existing.text != chunk.text
                    {
                        return Err(DomainError::DuplicateId {
                            kind: "chunk",
                            id: chunk.chunk_id.value(),
                        });
                    }
                } else {
                    generated.push(self.handle_register_chunk(chunk.clone())?);
                    self.pending_full_text.insert(chunk.chunk_id);
                    new_chunks += 1;
                }
                self.chunk_nodes.insert(chunk.chunk_id, chunk.node_id);
            }
            for card in &input.cards {
                if !input.tree_nodes.iter().any(|node| node.id == card.node_id)
                    && card.node_id != StructureNodeId::new(card.card_id.value())
                {
                    return Err(DomainError::InternalInvariantViolation {
                        detail: "parser card references a missing document tree node",
                    });
                }
                if let Some(existing) = self.cards.get(&card.card_id) {
                    if existing.artifact_id != card.artifact_id
                        || existing.node_id != card.node_id
                        || existing.source_span != card.source_span
                        || existing.title != card.title
                        || existing.body != card.body
                    {
                        return Err(DomainError::DuplicateId {
                            kind: "card",
                            id: card.card_id.value(),
                        });
                    }
                } else {
                    generated.push(self.handle_create_card(card.clone())?);
                    new_cards += 1;
                }
            }
        } else if !input.chunks.is_empty() || !input.cards.is_empty() {
            return Err(DomainError::InternalInvariantViolation {
                detail: "non-Parsed ParserResult must not contain chunks or cards",
            });
        }
        Ok((new_chunks, new_cards))
    }

    pub(super) fn capture_parser_tree(
        &mut self,
        input: &ParserResult,
        generated: &mut Vec<DomainEventEnvelope>,
    ) -> Result<(), DomainError> {
        if let Some(tree_root_id) = input.tree_root_id {
            let node_ids: BTreeSet<_> = input.tree_nodes.iter().map(|node| node.id).collect();
            let structurally_valid = node_ids.len() == input.tree_nodes.len()
                && input
                    .tree_nodes
                    .iter()
                    .filter(|node| node.parent_id.is_none())
                    .count()
                    == 1
                && input
                    .tree_nodes
                    .iter()
                    .any(|node| node.id == tree_root_id && node.parent_id.is_none())
                && input.tree_nodes.iter().all(|node| {
                    node.parent_id
                        .is_none_or(|parent| node_ids.contains(&parent))
                        && node
                            .sibling_id
                            .is_none_or(|sibling| node_ids.contains(&sibling))
                })
                && !has_link_cycles(&input.tree_nodes, |node| node.parent_id)
                && !has_link_cycles(&input.tree_nodes, |node| node.sibling_id);
            if !structurally_valid {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "parser document tree failed structural validation",
                });
            }
            let tree_changed = self.artifact_versions.get(&input.artifact_id)
                != Some(&input.artifact_version_id)
                || self.artifact_content_hashes.get(&input.artifact_id)
                    != Some(&input.content_hash)
                || !self.document_trees.contains_key(&input.artifact_id);
            if tree_changed {
                let tree_event = self.emit_event(DomainEvent::DocumentTreeCaptured {
                    artifact_id: input.artifact_id,
                    artifact_version_id: input.artifact_version_id,
                    content_hash: input.content_hash.clone(),
                    root_id: tree_root_id,
                    nodes: input.tree_nodes.clone(),
                });
                self.artifact_versions
                    .insert(input.artifact_id, input.artifact_version_id);
                self.artifact_content_hashes
                    .insert(input.artifact_id, input.content_hash.clone());
                self.document_trees
                    .insert(input.artifact_id, (tree_root_id, input.tree_nodes.clone()));
                generated.push(tree_event);
            }
        } else if !input.tree_nodes.is_empty() {
            return Err(DomainError::InternalInvariantViolation {
                detail: "ParserResult with no tree_root_id must have empty tree_nodes",
            });
        }
        Ok(())
    }
}

fn has_link_cycles(
    nodes: &[StructureNode],
    next: fn(&StructureNode) -> Option<StructureNodeId>,
) -> bool {
    for node in nodes {
        let mut current = node.id;
        let mut visited = BTreeSet::new();
        while let Some(candidate) = nodes.iter().find(|candidate| candidate.id == current) {
            if !visited.insert(current) {
                return true;
            }
            let Some(next_id) = next(candidate) else {
                break;
            };
            current = next_id;
        }
    }
    false
}
