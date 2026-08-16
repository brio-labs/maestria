use crate::{
    error::to_port_error,
    execution::{Meter, validate_limit},
    keys::chunk_key,
    scoring::{descending_score, score_to_u32},
    search_helpers::collect_bounded,
    tantivy_index::TantivyFullTextIndex,
};
use maestria_domain::{ArtifactId, ChunkId, SearchExecutionCompletion, SearchExecutionResource};
use maestria_ports::{BoundedSearch, IndexedChunk, PortError, SearchHit, SearchQuery};
use tantivy::schema::Value;
use tantivy::{
    TantivyDocument, Term,
    query::{AllQuery, BooleanQuery, QueryParser, TermSetQuery},
};

fn budget_usize(value: u64) -> usize {
    maestria_domain::saturating_usize(value)
}

type ScoredChunks = (
    Vec<(f32, u64, u64, IndexedChunk)>,
    Option<SearchExecutionResource>,
);

impl TantivyFullTextIndex {
    pub(crate) fn index_chunks_impl(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| PortError::DownstreamContext {
                context: "index chunks requires a writable full-text index",
                source: "full-text index is read-only".to_string(),
            })?;
        for chunk in chunks {
            writer.delete_term(Term::from_field_text(
                self.fields.key,
                &chunk_key(chunk.artifact_id, chunk.chunk_id),
            ));
            writer
                .add_document(self.chunk_document(&chunk))
                .map_err(to_port_error)?;
        }
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }

    pub(crate) fn delete_chunks_impl(
        &self,
        chunks: &[(maestria_domain::ArtifactId, maestria_domain::ChunkId)],
    ) -> Result<(), PortError> {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| PortError::DownstreamContext {
                context: "delete chunks requires a writable full-text index",
                source: "full-text index is read-only".to_string(),
            })?;
        for (artifact_id, chunk_id) in chunks {
            writer.delete_term(Term::from_field_text(
                self.fields.key,
                &chunk_key(*artifact_id, *chunk_id),
            ));
        }
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }

    pub(crate) fn clear_impl(&self) -> Result<(), PortError> {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| PortError::DownstreamContext {
                context: "clear requires a writable full-text index",
                source: "full-text index is read-only".to_string(),
            })?;
        writer.delete_all_documents().map_err(to_port_error)?;
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
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
        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.fields.text]);
        let parsed_query =
            parser
                .parse_query(trimmed)
                .map_err(|error| PortError::InvalidInputContext {
                    context: "invalid search query",
                    source: error.to_string(),
                })?;
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
        let (scored, stopped) = self.score_chunk_documents(
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
        let searcher = self.reader.searcher();
        let (allowed, authorization_stop) =
            self.allowed_chunk_keys(&searcher, filter, &mut meter)?;
        if allowed.is_empty() {
            if let Some(resource) = authorization_stop {
                return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Exhausted(resource)));
            }
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
        }
        if meter.usage.candidates >= query.execution_budget.max_candidates() {
            return Ok(meter.done(
                Vec::new(),
                SearchExecutionCompletion::Exhausted(SearchExecutionResource::Candidates),
            ));
        }
        let parser = QueryParser::for_index(&self.index, vec![self.fields.text]);
        let parsed_query =
            parser
                .parse_query(trimmed)
                .map_err(|error| PortError::InvalidInputContext {
                    context: "invalid search query",
                    source: error.to_string(),
                })?;
        let scoped_query = BooleanQuery::intersection(vec![
            parsed_query,
            Box::new(TermSetQuery::new(
                allowed
                    .into_iter()
                    .map(|key| Term::from_field_text(self.fields.key, &key)),
            )),
        ]);
        let remaining = query
            .execution_budget
            .max_candidates()
            .saturating_sub(meter.usage.candidates);
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
        let (scored, score_stop) = self.score_chunk_documents(
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
        let mut scored = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            if let Some(resource) = meter.candidate() {
                stopped = Some(resource);
                break;
            }
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            let bytes = document
                .get_first(self.fields.text)
                .and_then(|value| value.as_str())
                .map_or(0, |text| text.len() as u64);
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
        scored.sort_by(|a, b| {
            descending_score(a.0, b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        Ok((scored, stopped))
    }

    pub(crate) fn allowed_chunk_keys(
        &self,
        searcher: &tantivy::Searcher,
        filter: &dyn Fn(ChunkId, ArtifactId) -> Result<bool, PortError>,
        meter: &mut Meter,
    ) -> Result<(Vec<String>, Option<SearchExecutionResource>), PortError> {
        // The AllQuery walk collects every live document; the candidate
        // limit is one past the document count so a complete walk never
        // trips the collector's `docs.len() == candidate_limit` truncation
        // marker (which would mark the lane Exhausted(Candidates) and the
        // engine's adaptive loop BudgetExhausted on a full answer).
        let limit = budget_usize(searcher.num_docs());
        if limit == 0 {
            return Ok((Vec::new(), Some(SearchExecutionResource::Candidates)));
        }
        let collection = collect_bounded(
            searcher,
            &AllQuery,
            0,
            limit,
            limit.saturating_add(1),
            meter,
        )?;
        if let Some(resource) = collection.stopped {
            return Ok((Vec::new(), Some(resource)));
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
        Ok((allowed.into_iter().collect(), stopped))
    }
}

fn finish_results(
    meter: &mut Meter,
    count: usize,
    stopped: Option<SearchExecutionResource>,
) -> SearchExecutionCompletion {
    let mut stopped = stopped;
    for _ in 0..count {
        if let Some(resource) = meter.result() {
            stopped = Some(resource);
            break;
        }
    }
    stopped.map_or(
        SearchExecutionCompletion::Complete,
        SearchExecutionCompletion::Exhausted,
    )
}
