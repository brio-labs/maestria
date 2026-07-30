use crate::execution::Meter;
use crate::lexical_helpers::{page_card_hits, page_chunk_hits, score_card, score_chunk};
use crate::tantivy_index::{TantivyFullTextIndex, to_port_error};
use maestria_domain::{SearchExecutionCompletion, SearchExecutionResource};
use maestria_ports::{
    BoundedSearch, CardField, ChunkField, HitReason, IndexedLexicalCard, IndexedLexicalChunk,
    LexicalCardHit, LexicalChunkHit, LexicalQuery, PortError,
};
use tantivy::TantivyDocument;

pub(crate) type LexicalChunkScores = Vec<(f32, u64, u64, IndexedLexicalChunk, HitReason)>;
pub(crate) type LexicalCardScores = Vec<(f32, u64, u64, IndexedLexicalCard, HitReason)>;
pub(crate) type ScoredLexicalChunks = (LexicalChunkScores, Option<SearchExecutionResource>);
pub(crate) type ScoredLexicalCards = (LexicalCardScores, Option<SearchExecutionResource>);

impl TantivyFullTextIndex {
    pub(crate) fn finish_chunk_search(
        &self,
        scored: LexicalChunkScores,
        query: &LexicalQuery<ChunkField>,
        mut meter: Meter,
        mut stopped: Option<SearchExecutionResource>,
    ) -> BoundedSearch<LexicalChunkHit> {
        let result_exhausted =
            maestria_domain::saturating_u64(scored.len()) > query.execution_budget.max_results();
        let selected = page_chunk_hits(scored, query);
        for _ in 0..selected.len() {
            if let Some(resource) = meter.result() {
                stopped = Some(resource);
                break;
            }
        }
        let completion = if result_exhausted {
            SearchExecutionCompletion::Exhausted(SearchExecutionResource::Results)
        } else if let Some(resource) = stopped {
            SearchExecutionCompletion::Exhausted(resource)
        } else {
            SearchExecutionCompletion::Complete
        };
        meter.done(selected, completion)
    }

    pub(crate) fn finish_card_search(
        &self,
        scored: LexicalCardScores,
        query: &LexicalQuery<CardField>,
        mut meter: Meter,
        mut stopped: Option<SearchExecutionResource>,
    ) -> BoundedSearch<LexicalCardHit> {
        let result_exhausted =
            maestria_domain::saturating_u64(scored.len()) > query.execution_budget.max_results();
        let selected = page_card_hits(scored, query);
        for _ in 0..selected.len() {
            if let Some(resource) = meter.result() {
                stopped = Some(resource);
                break;
            }
        }
        let completion = if result_exhausted {
            SearchExecutionCompletion::Exhausted(SearchExecutionResource::Results)
        } else if let Some(resource) = stopped {
            SearchExecutionCompletion::Exhausted(resource)
        } else {
            SearchExecutionCompletion::Complete
        };
        meter.done(selected, completion)
    }

    pub(crate) fn score_lexical_chunks(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: Vec<(f32, tantivy::DocAddress)>,
        truncated: bool,
        query: &LexicalQuery<ChunkField>,
        needle: &str,
        meter: &mut Meter,
    ) -> Result<ScoredLexicalChunks, PortError> {
        let mut stopped = truncated.then_some(SearchExecutionResource::Candidates);
        let mut scored = Vec::new();
        for (_, address) in top_docs {
            if let Some(resource) = meter.candidate() {
                stopped = Some(resource);
                break;
            }
            if let Some(resource) =
                meter.work(maestria_domain::saturating_u64(query.fields.len()).saturating_add(1))
            {
                stopped = Some(resource);
                break;
            }
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            if document.get_first(self.fields.chunk_id).is_none() {
                continue;
            }
            let chunk = self.read_lexical_chunk(&document)?;
            let bytes = maestria_domain::saturating_u64(
                chunk
                    .text
                    .len()
                    .saturating_add(chunk.path.as_ref().map_or(0, String::len))
                    .saturating_add(chunk.filename.as_ref().map_or(0, String::len))
                    .saturating_add(chunk.symbol.as_ref().map_or(0, String::len)),
            );
            if let Some(resource) = meter.bytes(bytes) {
                stopped = Some(resource);
                break;
            }
            if let Some((raw_score, reason)) = score_chunk(&chunk, query, needle) {
                scored.push((
                    raw_score,
                    chunk.artifact_id.value(),
                    chunk.chunk_id.value(),
                    chunk,
                    reason,
                ));
            }
        }
        Ok((scored, stopped))
    }

    pub(crate) fn score_lexical_cards(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: Vec<(f32, tantivy::DocAddress)>,
        truncated: bool,
        query: &LexicalQuery<CardField>,
        needle: &str,
        meter: &mut Meter,
    ) -> Result<ScoredLexicalCards, PortError> {
        let mut stopped = truncated.then_some(SearchExecutionResource::Candidates);
        let mut scored = Vec::new();
        for (_, address) in top_docs {
            if let Some(resource) = meter.candidate() {
                stopped = Some(resource);
                break;
            }
            if let Some(resource) =
                meter.work(maestria_domain::saturating_u64(query.fields.len()).saturating_add(1))
            {
                stopped = Some(resource);
                break;
            }
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            if document.get_first(self.fields.card_id).is_none() {
                continue;
            }
            let card = self.read_lexical_card(&document)?;
            let bytes = maestria_domain::saturating_u64(
                card.title
                    .len()
                    .saturating_add(card.body.len())
                    .saturating_add(card.path.as_ref().map_or(0, String::len))
                    .saturating_add(card.filename.as_ref().map_or(0, String::len))
                    .saturating_add(card.symbol.as_ref().map_or(0, String::len)),
            );
            if let Some(resource) = meter.bytes(bytes) {
                stopped = Some(resource);
                break;
            }
            if let Some((raw_score, reason)) = score_card(&card, query, needle) {
                scored.push((
                    raw_score,
                    card.artifact_id.value(),
                    card.card_id.value(),
                    card,
                    reason,
                ));
            }
        }
        Ok((scored, stopped))
    }
}
