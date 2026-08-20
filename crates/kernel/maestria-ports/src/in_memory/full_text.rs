use std::sync::{Arc, Mutex};

use super::execution::{Meter, validate_limit};
use super::store::lock_map;
use crate::{BoundedSearch, CardHit, IndexedCard, IndexedChunk, PortError, SearchHit, SearchQuery};

#[derive(Clone, Default)]
pub struct InMemoryFullTextIndex {
    chunks: Arc<Mutex<Vec<IndexedChunk>>>,
    cards: Arc<Mutex<Vec<IndexedCard>>>,
    lexical_chunks: Arc<Mutex<Vec<crate::IndexedLexicalChunk>>>,
    lexical_cards: Arc<Mutex<Vec<crate::IndexedLexicalCard>>>,
}

impl InMemoryFullTextIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::FullTextIndex for InMemoryFullTextIndex {
    fn supports_lexical_metadata(&self) -> bool {
        true
    }
    fn delete_chunks(
        &self,
        chunks: &[(maestria_domain::ArtifactId, maestria_domain::ChunkId)],
    ) -> Result<(), PortError> {
        let mut guard = lock_map(&self.chunks, "full-text chunk index lock poisoned")?;
        let mut lexical_guard =
            self.lexical_chunks
                .lock()
                .map_err(|_| PortError::InternalContext {
                    context: "full-text lexical chunk index lock poisoned",
                    source: "lexical chunk index mutex is poisoned".to_string(),
                })?;
        for (artifact_id, chunk_id) in chunks {
            guard.retain(|existing| {
                existing.artifact_id != *artifact_id || existing.chunk_id != *chunk_id
            });
            lexical_guard.retain(|existing| {
                existing.artifact_id != *artifact_id || existing.chunk_id != *chunk_id
            });
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        let mut guard = lock_map(&self.chunks, "full-text chunk index lock poisoned")?;
        let mut lexical_guard =
            self.lexical_chunks
                .lock()
                .map_err(|_| PortError::InternalContext {
                    context: "full-text lexical chunk index lock poisoned",
                    source: "lexical chunk index mutex is poisoned".to_string(),
                })?;
        guard.clear();
        lexical_guard.clear();
        Ok(())
    }

    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        let mut guard = lock_map(&self.chunks, "full-text chunk index lock poisoned")?;

        let mut lexical_guard =
            self.lexical_chunks
                .lock()
                .map_err(|_| PortError::InternalContext {
                    context: "full-text lexical chunk index lock poisoned",
                    source: "lexical chunk index mutex is poisoned".to_string(),
                })?;

        for chunk in &chunks {
            guard.retain(|existing| {
                existing.artifact_id != chunk.artifact_id || existing.chunk_id != chunk.chunk_id
            });
            lexical_guard.retain(|existing| {
                existing.artifact_id != chunk.artifact_id || existing.chunk_id != chunk.chunk_id
            });
            lexical_guard.push(crate::IndexedLexicalChunk {
                artifact_id: chunk.artifact_id,
                chunk_id: chunk.chunk_id,
                text: chunk.text.clone(),
                path: None,
                filename: None,
                symbol: None,
            });
        }
        guard.extend(chunks);
        Ok(())
    }
    fn search(&self, query: SearchQuery) -> Result<BoundedSearch<SearchHit>, PortError> {
        if query.q.trim().is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "empty chunk search query",
                source: "query must contain non-whitespace text".to_string(),
            });
        }
        self.search_filtered(query, &|_, _| Ok(true))
    }

    fn search_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(
            maestria_domain::ChunkId,
            maestria_domain::ArtifactId,
        ) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<SearchHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "empty filtered chunk search query",
                source: "query must contain non-whitespace text".to_string(),
            });
        }
        validate_limit(
            query.limit,
            query.execution_budget,
            "chunk search result limit",
        )?;
        let mut meter = Meter::new(query.execution_budget);
        if query.limit == 0 {
            return Ok(meter.complete(Vec::new()));
        }
        let guard = lock_map(&self.chunks, "full-text chunk index lock poisoned")?;
        let needle = trimmed.to_lowercase();
        let mut hits = Vec::new();
        let mut stopped = None;
        for chunk in guard.iter() {
            if let Some(resource) = meter.candidate() {
                stopped = Some(resource);
                break;
            }
            if !filter(chunk.chunk_id, chunk.artifact_id)? {
                continue;
            }
            let bytes = u64::try_from(chunk.text.len()).map_err(|error| {
                PortError::internal("full-text chunk byte accounting", error.to_string())
            })?;
            if let Some(resource) = meter.bytes(bytes) {
                stopped = Some(resource);
                break;
            }
            if let Some(resource) = meter.work(1) {
                stopped = Some(resource);
                break;
            }
            if !chunk.text.to_lowercase().contains(&needle) {
                continue;
            }
            hits.push(SearchHit {
                chunk: chunk.clone(),
                score: (chunk.text.len().min(u32::MAX as usize)) as u32,
            });
        }
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.chunk.artifact_id.cmp(&b.chunk.artifact_id))
                .then_with(|| a.chunk.chunk_id.cmp(&b.chunk.chunk_id))
        });
        let result_exhausted = hits.len() > query.limit;
        let selected = hits
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect::<Vec<_>>();
        if result_exhausted {
            stopped = Some(maestria_domain::SearchExecutionResource::Results);
        }
        for _ in 0..selected.len() {
            if let Some(resource) = meter.result() {
                stopped = Some(resource);
                break;
            }
        }
        if let Some(resource) = stopped {
            Ok(meter.exhausted(selected, resource))
        } else {
            Ok(meter.complete(selected))
        }
    }

    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError> {
        let mut guard = lock_map(&self.cards, "full-text card index lock poisoned")?;
        let mut lexical_guard =
            self.lexical_cards
                .lock()
                .map_err(|_| PortError::InternalContext {
                    context: "full-text lexical card index lock poisoned",
                    source: "lexical card index mutex is poisoned".to_string(),
                })?;
        for card in &cards {
            guard.retain(|c| c.artifact_id != card.artifact_id || c.card_id != card.card_id);
            lexical_guard
                .retain(|c| c.artifact_id != card.artifact_id || c.card_id != card.card_id);
            lexical_guard.push(crate::IndexedLexicalCard {
                artifact_id: card.artifact_id,
                card_id: card.card_id,
                title: card.title.clone(),
                body: card.body.clone(),
                path: None,
                filename: None,
                symbol: None,
            });
        }
        guard.extend(cards);
        Ok(())
    }

    fn search_cards(&self, query: SearchQuery) -> Result<BoundedSearch<CardHit>, PortError> {
        if query.q.trim().is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "empty card search query",
                source: "query must contain non-whitespace text".to_string(),
            });
        }
        self.search_cards_filtered(query, &|_, _| Ok(true))
    }

    fn search_cards_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(
            maestria_domain::CardId,
            maestria_domain::ArtifactId,
        ) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<CardHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "empty filtered card search query",
                source: "query must contain non-whitespace text".to_string(),
            });
        }
        validate_limit(
            query.limit,
            query.execution_budget,
            "card search result limit",
        )?;
        let mut meter = Meter::new(query.execution_budget);
        if query.limit == 0 {
            return Ok(meter.complete(Vec::new()));
        }
        let guard = lock_map(&self.cards, "full-text card index lock poisoned")?;
        let needle = trimmed.to_lowercase();
        let mut hits = Vec::new();
        let mut stopped = None;
        for card in guard.iter() {
            if let Some(resource) = meter.candidate() {
                stopped = Some(resource);
                break;
            }
            if !filter(card.card_id, card.artifact_id)? {
                continue;
            }
            let bytes = u64::try_from(card.title.len().saturating_add(card.body.len())).map_err(
                |error| PortError::InternalContext {
                    context: "full-text card byte accounting",
                    source: error.to_string(),
                },
            )?;
            if let Some(resource) = meter.bytes(bytes) {
                stopped = Some(resource);
                break;
            }
            if let Some(resource) = meter.work(1) {
                stopped = Some(resource);
                break;
            }
            if !(card.title.to_lowercase().contains(&needle)
                || card.body.to_lowercase().contains(&needle))
            {
                continue;
            }
            let score = ((card.title.len() + card.body.len()).min(u32::MAX as usize)) as u32;
            hits.push(CardHit {
                card: card.clone(),
                score,
            });
        }
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.card.artifact_id.cmp(&b.card.artifact_id))
                .then_with(|| a.card.card_id.cmp(&b.card.card_id))
        });
        let result_exhausted = hits.len() > query.limit;
        let selected = hits
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect::<Vec<_>>();
        if result_exhausted {
            stopped = Some(maestria_domain::SearchExecutionResource::Results);
        }
        for _ in 0..selected.len() {
            if let Some(resource) = meter.result() {
                stopped = Some(resource);
                break;
            }
        }
        if let Some(resource) = stopped {
            Ok(meter.exhausted(selected, resource))
        } else {
            Ok(meter.complete(selected))
        }
    }
    fn index_lexical_chunks(
        &self,
        chunks: Vec<crate::IndexedLexicalChunk>,
    ) -> Result<(), PortError> {
        super::lexical::index_lexical_chunks(&self.lexical_chunks, chunks)
    }

    fn index_lexical_cards(&self, cards: Vec<crate::IndexedLexicalCard>) -> Result<(), PortError> {
        super::lexical::index_lexical_cards(&self.lexical_cards, cards)
    }
}

#[cfg(test)]
#[path = "full_text_tests.rs"]
mod tests;
