use crate::{
    error::to_port_error,
    execution::{Meter, budget_usize, finish_results, validate_limit},
    keys::chunk_key,
    memory_document::chunk_delete_key_len,
    scoring::{descending_score, score_to_u32},
    search_helpers::{collect_bounded, parse_query, scope_by_keys},
    tantivy_index::TantivyFullTextIndex,
};
use sillage_domain::{ArtifactId, ChunkId, SearchExecutionCompletion, SearchExecutionResource};
use sillage_ports::{BoundedSearch, IndexedChunk, PortError, SearchHit, SearchQuery};
use std::collections::BTreeSet;
use tantivy::schema::Value;
use tantivy::{TantivyDocument, Term, query::Query};
type ScoredChunks = (
    Vec<(f32, u64, u64, IndexedChunk)>,
    Option<SearchExecutionResource>,
    Option<crate::RamMemoryReservation>,
);
impl TantivyFullTextIndex {
    pub(crate) fn index_chunks_impl(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        self.index_artifact_chunks_impl(chunks, Vec::new(), Vec::new(), Vec::new())
    }

    pub(crate) fn delete_chunks_impl(
        &self,
        chunks: &[(sillage_domain::ArtifactId, sillage_domain::ChunkId)],
    ) -> Result<(), PortError> {
        self.with_writer(
            "delete chunks requires a writable full-text index",
            |writer| {
                self.reserve_writer_batch(
                    &[],
                    chunks.iter().map(|(artifact_id, chunk_id)| {
                        chunk_delete_key_len(*artifact_id, *chunk_id)
                    }),
                )?;
                for (artifact_id, chunk_id) in chunks {
                    self.mark_dirty();
                    writer.delete_term(Term::from_field_text(
                        self.fields.key,
                        &chunk_key(*artifact_id, *chunk_id),
                    ));
                }
                Ok(())
            },
        )
    }

    pub(crate) fn clear_impl(&self) -> Result<(), PortError> {
        self.with_writer("clear requires a writable full-text index", |writer| {
            self.reserve_writer_batch(&[], std::iter::once(0))?;
            self.mark_dirty();
            writer.delete_all_documents().map_err(to_port_error)?;
            Ok(())
        })
    }

    pub(crate) fn search_chunks_impl(
        &self,
        query: SearchQuery,
    ) -> Result<BoundedSearch<SearchHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "empty chunk search query",
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
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
        }
        let _reader_guard = self.reader_guard()?;
        let searcher = self.reader.searcher();
        let _query_memory = self.reserve_query_work(&query, searcher.num_docs())?;
        let parsed_query = parse_query(&self.index, vec![self.fields.text], trimmed)?;
        let collection = collect_bounded(
            &searcher,
            &parsed_query,
            query.offset,
            query.limit,
            budget_usize(query.execution_budget.max_candidates()),
            &mut meter,
        )?;
        if let Some(resource) = collection.stopped {
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Exhausted(resource)));
        }
        let (scored, stopped, _output_memory) = self.score_chunk_documents(
            &searcher,
            collection.docs,
            collection.truncated,
            &mut meter,
        )?;
        let selected = scored
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|(score, _, _, chunk)| SearchHit {
                chunk,
                score: score_to_u32(score),
            })
            .collect::<Vec<_>>();
        let completion = finish_results(&mut meter, selected.len(), stopped);
        Ok(meter.done(selected, completion))
    }

    pub(crate) fn search_chunks_filtered_impl(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(ChunkId, ArtifactId) -> Result<bool, PortError>,
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
            "filtered chunk search result limit",
        )?;
        let mut meter = Meter::new(query.execution_budget);
        if query.limit == 0 {
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
        }
        let _reader_guard = self.reader_guard()?;
        let searcher = self.reader.searcher();
        let _query_memory = self.reserve_query_work(&query, searcher.num_docs())?;
        let parsed_query = parse_query(&self.index, vec![self.fields.text], trimmed)?;
        let (allowed, authorization_stop) =
            self.allowed_chunk_keys(&searcher, parsed_query.as_ref(), filter, &mut meter)?;
        if allowed.is_empty() {
            if let Some(resource) = authorization_stop {
                return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Exhausted(resource)));
            }
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
        }
        if meter.usage().candidates >= query.execution_budget.max_candidates() {
            return Ok(meter.done(
                Vec::new(),
                SearchExecutionCompletion::Exhausted(SearchExecutionResource::Candidates),
            ));
        }
        let scoped_query = scope_by_keys(parsed_query, self.fields.key, allowed);
        let remaining = query
            .execution_budget
            .max_candidates()
            .saturating_sub(meter.usage().candidates);
        let collection = collect_bounded(
            &searcher,
            &scoped_query,
            query.offset,
            query.limit,
            budget_usize(remaining),
            &mut meter,
        )?;
        if let Some(resource) = collection.stopped {
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Exhausted(resource)));
        }
        let (scored, score_stop, _output_memory) = self.score_chunk_documents(
            &searcher,
            collection.docs,
            collection.truncated,
            &mut meter,
        )?;
        let stopped = authorization_stop.or(score_stop);
        let selected = scored
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|(score, _, _, chunk)| SearchHit {
                chunk,
                score: score_to_u32(score),
            })
            .collect::<Vec<_>>();
        let completion = finish_results(&mut meter, selected.len(), stopped);
        Ok(meter.done(selected, completion))
    }

    fn score_chunk_documents(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: Vec<(f32, tantivy::DocAddress)>,
        truncated: bool,
        meter: &mut Meter,
    ) -> Result<ScoredChunks, PortError> {
        let mut stopped = truncated.then_some(SearchExecutionResource::Candidates);
        let output_memory = self.reserve_scored_output(top_docs.len())?;
        let mut scored = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            if let Some(resource) = meter.candidate() {
                stopped = Some(resource);
                break;
            }
            let _decode_memory = self.reserve_reader_document()?;
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            let bytes = match document
                .get_first(self.fields.text)
                .and_then(|value| value.as_str())
            {
                Some(text) => text.len() as u64,
                None => 0,
            };
            if let Some(resource) = meter.bytes(bytes) {
                stopped = Some(resource);
                break;
            }
            if let Some(resource) = meter.work(1) {
                stopped = Some(resource);
                break;
            }
            let chunk = self.read_chunk(&document)?;
            scored.push((
                score,
                chunk.artifact_id.value(),
                chunk.chunk_id.value(),
                chunk,
            ));
        }
        scored.sort_unstable_by(|a, b| {
            descending_score(a.0, b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        Ok((scored, stopped, output_memory))
    }

    pub(crate) fn allowed_chunk_keys(
        &self,
        searcher: &tantivy::Searcher,
        query: &dyn Query,
        filter: &dyn Fn(ChunkId, ArtifactId) -> Result<bool, PortError>,
        meter: &mut Meter,
    ) -> Result<(BTreeSet<String>, Option<SearchExecutionResource>), PortError> {
        // Only text-matched documents need authorization. Walking AllQuery
        // before evaluating a rare phrase makes every interaction O(corpus)
        // and can exhaust the budget before its matching document is seen.
        let limit = budget_usize(searcher.num_docs());
        if limit == 0 {
            return Ok((BTreeSet::new(), Some(SearchExecutionResource::Candidates)));
        }
        let collection =
            collect_bounded(searcher, query, 0, limit, limit.saturating_add(1), meter)?;
        if let Some(resource) = collection.stopped {
            return Ok((BTreeSet::new(), Some(resource)));
        }
        let mut allowed = std::collections::BTreeSet::new();
        let mut stopped = None;
        for (_, address) in collection.docs {
            let (artifact_id, chunk_id) = self.read_chunk_identity_at(searcher, address)?;
            if let Some(resource) = meter.bytes(crate::documents::INDEXED_IDENTITY_BYTES) {
                stopped = Some(resource);
                break;
            }
            if !filter(chunk_id, artifact_id)? {
                continue;
            }
            if let Some(resource) = meter.work(1) {
                stopped = Some(resource);
                break;
            }
            allowed.insert(chunk_key(artifact_id, chunk_id));
        }
        if stopped.is_none() && collection.truncated {
            stopped = Some(SearchExecutionResource::Candidates);
        }
        Ok((allowed, stopped))
    }
}
