use std::{collections::BTreeSet, path::Path};

use super::SessionStores;
use crate::session_search::retained::{PersistedGeneration, RetainedDocumentaryStore};
use anyhow::{Result, anyhow, bail};
use sillage_domain::{
    Artifact, BlobId, Card, Chunk, Evidence, EvidenceId, EvidenceKind, IndexStatus, LogicalTick,
    ParseStatus, SnapshotRef, SourceSpan, evidence_id_for, excerpt_for, representations_digest,
};
use sillage_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EvidenceRepository, FullTextIndex,
    IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk, ParsedArtifact,
    RepresentationKind, SourceSpan as ParsedSourceSpan,
};

pub(super) fn parsed_memory_estimate(parsed: &ParsedArtifact, path_bytes: usize) -> Result<u64> {
    let mut estimate = u64::try_from(path_bytes)?
        .saturating_mul((parsed.chunks.len() + parsed.cards.len()) as u64 + 1)
        .saturating_mul(4)
        .saturating_add(2048)
        .saturating_add(u64::try_from(parsed.chunks.len())?.saturating_mul(512))
        .saturating_add(u64::try_from(parsed.cards.len())?.saturating_mul(384))
        .saturating_add(u64::try_from(parsed.tree.nodes().len())?.saturating_mul(256));
    for chunk in &parsed.chunks {
        estimate = estimate.saturating_add(u64::try_from(chunk.text.capacity())?.saturating_mul(2));
        for representation in &chunk.representations {
            estimate = estimate.saturating_add(
                u64::try_from(representation.content.capacity())?.saturating_mul(2),
            );
        }
    }
    for card in &parsed.cards {
        estimate = estimate
            .saturating_add(u64::try_from(card.card.title.capacity())?.saturating_mul(2))
            .saturating_add(u64::try_from(card.card.body.capacity())?.saturating_mul(2));
    }
    for node in parsed.tree.nodes() {
        estimate = estimate
            .saturating_add(u64::try_from(node.parser_generation.capacity())?)
            .saturating_add(u64::try_from(node.schema_generation.capacity())?)
            .saturating_add(
                u64::try_from(node.section_path.capacity())?
                    .saturating_mul(std::mem::size_of::<String>() as u64),
            );
        if let Some(language) = &node.language {
            estimate = estimate.saturating_add(u64::try_from(language.capacity())?);
        }
        for part in &node.section_path {
            estimate = estimate.saturating_add(u64::try_from(part.capacity())?.saturating_mul(2));
        }
    }
    Ok(estimate)
}

pub(super) enum ParsedSourceStoreOutcome {
    Stored {
        blob_id: BlobId,
        generation: Option<PersistedGeneration>,
    },
    SecretRejected,
}

fn parsed_content_is_secret_clean(stores: &SessionStores, parsed: &ParsedArtifact) -> Result<bool> {
    for chunk in &parsed.chunks {
        if !parsed_text_is_secret_clean(stores, &chunk.text)? {
            return Ok(false);
        }
    }
    for card in &parsed.cards {
        if !parsed_text_is_secret_clean(stores, &card.card.title)?
            || !parsed_text_is_secret_clean(stores, &card.card.body)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parsed_text_is_secret_clean(stores: &SessionStores, text: &str) -> Result<bool> {
    let _admission = stores.memory.reserve(secret_scan_memory_estimate(text)?)?;
    Ok(sillage_governance::scan_secrets(text).is_clean())
}

fn secret_scan_memory_estimate(text: &str) -> Result<u64> {
    let lines = u64::try_from(text.lines().count())?;
    if lines == 0 {
        return Ok(0);
    }
    Ok(lines
        .saturating_mul(4)
        .max(4)
        .saturating_mul(std::mem::size_of::<sillage_governance::SecretFinding>() as u64)
        .saturating_add(64))
}

pub(super) fn store_parsed_source(
    stores: &SessionStores,
    title: &str,
    path: &Path,
    parsed: &ParsedArtifact,
    source_bytes: Vec<u8>,
    documentary: Option<&RetainedDocumentaryStore>,
) -> Result<ParsedSourceStoreOutcome> {
    if !parsed_content_is_secret_clean(stores, parsed)? {
        return Ok(ParsedSourceStoreOutcome::SecretRejected);
    }
    let generation = documentary
        .map(|store| store.persist(&source_bytes, &parsed.content_hash, &stores.memory))
        .transpose()?;
    index_source(stores, path, parsed)?;
    let blob_id = sillage_ports::BlobStore::put(&stores.blobs, source_bytes)?;
    let path_text = path
        .to_str()
        .ok_or_else(|| anyhow!("source path is not UTF-8"))?
        .to_owned();
    let records = store_chunks(stores, &path_text, parsed, blob_id)?;
    let card_ids = store_cards(stores, parsed)?;
    stores.artifacts.put(Artifact {
        id: parsed.artifact_id,
        title: title.to_owned(),
        chunk_ids: records.chunk_ids,
        card_ids,
        claim_ids: BTreeSet::new(),
        evidence_ids: records.evidence_ids,
        index_status: IndexStatus::Indexed,
        content_hash: Some(parsed.content_hash.clone()),
        parse_status: Some(to_domain_parse_status(&parsed.status)),
        security: sillage_domain::SecurityMetadata::default(),
    })?;
    Ok(ParsedSourceStoreOutcome::Stored {
        blob_id,
        generation,
    })
}

fn index_source(stores: &SessionStores, path: &Path, parsed: &ParsedArtifact) -> Result<()> {
    let _staging = stores
        .memory
        .reserve(parsed_memory_estimate(parsed, path.as_os_str().len())?)?;
    let path_text = path
        .to_str()
        .ok_or_else(|| anyhow!("source path is not UTF-8"))?;
    let filename = path.file_name().and_then(|name| name.to_str());
    let chunks = parsed
        .chunks
        .iter()
        .map(|chunk| IndexedChunk {
            artifact_id: parsed.artifact_id,
            chunk_id: chunk.chunk_id,
            text: chunk.text.clone(),
        })
        .collect();
    let lexical_chunks = parsed
        .chunks
        .iter()
        .map(|chunk| IndexedLexicalChunk {
            artifact_id: parsed.artifact_id,
            chunk_id: chunk.chunk_id,
            text: chunk.text.clone(),
            path: Some(path_text.to_owned()),
            filename: filename.map(str::to_owned),
            symbol: None,
        })
        .collect();
    let cards = parsed
        .cards
        .iter()
        .map(|card| IndexedCard {
            artifact_id: card.card.artifact_id,
            card_id: card.card.card_id,
            title: card.card.title.clone(),
            body: card.card.body.clone(),
        })
        .collect();
    let lexical_cards = parsed
        .cards
        .iter()
        .map(|card| IndexedLexicalCard {
            artifact_id: card.card.artifact_id,
            card_id: card.card.card_id,
            title: card.card.title.clone(),
            body: card.card.body.clone(),
            path: Some(path_text.to_owned()),
            filename: filename.map(str::to_owned),
            symbol: None,
        })
        .collect();
    stores
        .search_index
        .index_artifact_chunks(chunks, cards, lexical_chunks, lexical_cards)?;
    stores.search_index.commit_and_reload()?;
    Ok(())
}

struct IndexedChunkRecords {
    chunk_ids: BTreeSet<sillage_domain::ChunkId>,
    evidence_ids: BTreeSet<EvidenceId>,
}

fn store_chunks(
    stores: &SessionStores,
    path: &str,
    parsed: &ParsedArtifact,
    blob_id: BlobId,
) -> Result<IndexedChunkRecords> {
    let security = sillage_domain::SecurityMetadata::default();
    let mut chunk_ids = BTreeSet::new();
    let mut evidence_ids = BTreeSet::new();
    for (index, parsed_chunk) in parsed.chunks.iter().enumerate() {
        let order = u32::try_from(index)?;
        let span = to_domain_span(&parsed_chunk.source_span)?;
        let representations = to_domain_representations(&parsed_chunk.representations);
        let evidence_id = evidence_id_for(parsed.artifact_id, order);
        let excerpt = excerpt_for(&parsed_chunk.text);
        if excerpt_is_truncated(&parsed_chunk.text, &excerpt)? {
            stores.mark_excerpt_truncated(evidence_id)?;
        }
        let evidence = Evidence {
            id: evidence_id,
            artifact_id: parsed.artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: path.to_owned(),
                range: text_range(&span)?,
                snapshot: SnapshotRef::new(blob_id, parsed.content_hash.clone()),
            },
            excerpt,
            observed_at: LogicalTick::new(1),
            security: security.clone(),
        };
        stores.chunks.put(Chunk {
            id: parsed_chunk.chunk_id,
            artifact_id: parsed.artifact_id,
            node_id: parsed_chunk.node_id,
            source_span: span,
            representations_digest: representations_digest(&representations),
            representations,
            order,
            text: parsed_chunk.text.clone(),
        })?;
        stores.evidence.put(evidence)?;
        chunk_ids.insert(parsed_chunk.chunk_id);
        evidence_ids.insert(evidence_id);
    }
    Ok(IndexedChunkRecords {
        chunk_ids,
        evidence_ids,
    })
}

fn excerpt_is_truncated(source: &str, excerpt: &str) -> Result<bool> {
    let mut source_words = source.split_whitespace();
    for excerpt_word in excerpt.split_whitespace() {
        if source_words.next() != Some(excerpt_word) {
            bail!("canonical session excerpt is not a source prefix");
        }
    }
    Ok(source_words.next().is_some())
}

fn store_cards(
    stores: &SessionStores,
    parsed: &ParsedArtifact,
) -> Result<BTreeSet<sillage_domain::CardId>> {
    let mut card_ids = BTreeSet::new();
    for parsed_card in &parsed.cards {
        let card = &parsed_card.card;
        stores.cards.put(Card {
            id: card.card_id,
            artifact_id: card.artifact_id,
            node_id: parsed_card.node_id,
            source_span: card.source_span,
            title: card.title.clone(),
            body: card.body.clone(),
            security: match &card.security {
                Some(security) => security.clone(),
                None => Default::default(),
            },
        })?;
        card_ids.insert(card.card_id);
    }
    Ok(card_ids)
}

fn to_domain_parse_status(status: &sillage_ports::ParseStatus) -> ParseStatus {
    match status {
        sillage_ports::ParseStatus::Parsed => ParseStatus::Parsed,
        sillage_ports::ParseStatus::Unsupported => ParseStatus::Unsupported,
        sillage_ports::ParseStatus::Failed => ParseStatus::Failed,
        sillage_ports::ParseStatus::MetadataOnly => ParseStatus::MetadataOnly,
        sillage_ports::ParseStatus::NeedsOcr => ParseStatus::NeedsOcr,
        sillage_ports::ParseStatus::Quarantined => ParseStatus::Quarantined,
    }
}

fn to_domain_representations(
    representations: &[sillage_ports::ParsedRepresentation],
) -> Vec<sillage_domain::ParsedRepresentation> {
    representations
        .iter()
        .map(|representation| sillage_domain::ParsedRepresentation {
            kind: match representation.kind {
                RepresentationKind::Raw => sillage_domain::RepresentationKind::Raw,
                RepresentationKind::Retrieval => sillage_domain::RepresentationKind::Retrieval,
                RepresentationKind::Contextual => sillage_domain::RepresentationKind::Contextual,
                RepresentationKind::Summary => sillage_domain::RepresentationKind::Summary,
                RepresentationKind::Visual => sillage_domain::RepresentationKind::Visual,
            },
            content: representation.content.clone(),
        })
        .collect()
}

fn to_domain_span(span: &ParsedSourceSpan) -> Result<SourceSpan> {
    match span {
        ParsedSourceSpan::TextSpan {
            start_line,
            end_line,
        } => SourceSpan::text_span(*start_line, *end_line).map_err(anyhow::Error::new),
        _ => bail!("selected Markdown parser returned a non-text locator"),
    }
}

fn text_range(span: &SourceSpan) -> Result<sillage_domain::LineRange> {
    match span {
        SourceSpan::TextSpan {
            start_line,
            end_line,
        } => sillage_domain::LineRange::new(*start_line, *end_line).map_err(anyhow::Error::new),
        _ => bail!("selected Markdown evidence has a non-text locator"),
    }
}
