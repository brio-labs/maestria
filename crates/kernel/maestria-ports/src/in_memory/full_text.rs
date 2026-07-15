use std::sync::{Arc, Mutex};

use crate::{CardHit, IndexedCard, IndexedChunk, PortError, SearchHit, SearchQuery};

#[derive(Clone, Default)]
pub struct InMemoryFullTextIndex {
    chunks: Arc<Mutex<Vec<IndexedChunk>>>,
    cards: Arc<Mutex<Vec<IndexedCard>>>,
}

impl InMemoryFullTextIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::FullTextIndex for InMemoryFullTextIndex {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        let mut guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
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
        filter: &dyn Fn(crate::ChunkId, maestria_domain::ArtifactId) -> bool,
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
        for card in &cards {
            guard.retain(|c| c.artifact_id != card.artifact_id || c.card_id != card.card_id);
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
        filter: &dyn Fn(crate::CardId, maestria_domain::ArtifactId) -> bool,
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
}
