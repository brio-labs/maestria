use std::sync::{Arc, Mutex};

use crate::lexical::{CardField, ChunkField, LexicalCardHit, LexicalChunkHit, LexicalQuery};
use crate::{CardHit, IndexedCard, IndexedChunk, PortError, SearchHit, SearchQuery};

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
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        let mut guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;

        let mut lexical_guard = self
            .lexical_chunks
            .lock()
            .map_err(|_| PortError::Internal {
                message: "lexical index lock poisoned".to_string(),
            })?;

        for chunk in &chunks {
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

    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError> {
        let guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        let needle = query.q.to_lowercase();
        let mut hits = guard
            .iter()
            .filter(|chunk| chunk.text.to_lowercase().contains(&needle))
            .map(|chunk| SearchHit {
                chunk: chunk.clone(),
                score: (chunk.text.len().min(u32::MAX as usize)) as u32,
            })
            .collect::<Vec<_>>();

        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.chunk.artifact_id.cmp(&b.chunk.artifact_id))
                .then_with(|| a.chunk.chunk_id.cmp(&b.chunk.chunk_id))
        });
        Ok(hits
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect())
    }

    fn search_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(maestria_domain::ChunkId, maestria_domain::ArtifactId) -> bool,
    ) -> Result<Vec<SearchHit>, PortError> {
        let guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        let needle = query.q.to_lowercase();
        let mut hits = guard
            .iter()
            .filter(|chunk| chunk.text.to_lowercase().contains(&needle))
            .filter(|chunk| filter(chunk.chunk_id, chunk.artifact_id))
            .map(|chunk| SearchHit {
                chunk: chunk.clone(),
                score: (chunk.text.len().min(u32::MAX as usize)) as u32,
            })
            .collect::<Vec<_>>();

        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.chunk.artifact_id.cmp(&b.chunk.artifact_id))
                .then_with(|| a.chunk.chunk_id.cmp(&b.chunk.chunk_id))
        });
        Ok(hits
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect())
    }

    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError> {
        let mut guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        let mut lexical_guard = self.lexical_cards.lock().map_err(|_| PortError::Internal {
            message: "lexical index lock poisoned".to_string(),
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

    fn search_cards(&self, query: SearchQuery) -> Result<Vec<CardHit>, PortError> {
        let guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        let needle = query.q.to_lowercase();
        let mut hits: Vec<CardHit> = guard
            .iter()
            .filter(|card| {
                card.title.to_lowercase().contains(&needle)
                    || card.body.to_lowercase().contains(&needle)
            })
            .map(|card| {
                let score = ((card.title.len() + card.body.len()).min(u32::MAX as usize)) as u32;
                CardHit {
                    card: card.clone(),
                    score,
                }
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.card.artifact_id.cmp(&b.card.artifact_id))
                .then_with(|| a.card.card_id.cmp(&b.card.card_id))
        });
        Ok(hits
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect())
    }

    fn search_cards_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(maestria_domain::CardId, maestria_domain::ArtifactId) -> bool,
    ) -> Result<Vec<CardHit>, PortError> {
        let guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        let needle = query.q.to_lowercase();
        let mut hits: Vec<CardHit> = guard
            .iter()
            .filter(|card| {
                card.title.to_lowercase().contains(&needle)
                    || card.body.to_lowercase().contains(&needle)
            })
            .filter(|card| filter(card.card_id, card.artifact_id))
            .map(|card| {
                let score = ((card.title.len() + card.body.len()).min(u32::MAX as usize)) as u32;
                CardHit {
                    card: card.clone(),
                    score,
                }
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.card.artifact_id.cmp(&b.card.artifact_id))
                .then_with(|| a.card.card_id.cmp(&b.card.card_id))
        });
        Ok(hits
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect())
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

    fn search_lexical(
        &self,
        query: LexicalQuery<ChunkField>,
    ) -> Result<Vec<LexicalChunkHit>, PortError> {
        super::lexical::search_lexical(&self.lexical_chunks, query)
    }

    fn search_cards_lexical(
        &self,
        query: LexicalQuery<CardField>,
    ) -> Result<Vec<LexicalCardHit>, PortError> {
        super::lexical::search_cards_lexical(&self.lexical_cards, query)
    }

    fn search_lexical_filtered(
        &self,
        query: LexicalQuery<ChunkField>,
        filter: &dyn Fn(maestria_domain::ChunkId, maestria_domain::ArtifactId) -> bool,
    ) -> Result<Vec<LexicalChunkHit>, PortError> {
        super::lexical::search_lexical_filtered(&self.lexical_chunks, query, filter)
    }

    fn search_cards_lexical_filtered(
        &self,
        query: LexicalQuery<CardField>,
        filter: &dyn Fn(maestria_domain::CardId, maestria_domain::ArtifactId) -> bool,
    ) -> Result<Vec<LexicalCardHit>, PortError> {
        super::lexical::search_cards_lexical_filtered(&self.lexical_cards, query, filter)
    }
}
