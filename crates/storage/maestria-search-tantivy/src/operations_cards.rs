use crate::{
    error::to_port_error,
    execution::{Meter, budget_usize, finish_results, validate_limit},
    keys::card_key,
    scoring::{descending_score, score_to_u32},
    search_helpers::{collect_bounded, parse_query, scope_by_keys},
    tantivy_index::TantivyFullTextIndex,
};
use maestria_domain::{ArtifactId, CardId, SearchExecutionCompletion, SearchExecutionResource};
use maestria_ports::{BoundedSearch, CardHit, IndexedCard, PortError, SearchQuery};
use std::collections::BTreeSet;
use tantivy::schema::Value;
use tantivy::{TantivyDocument, Term, query::AllQuery};

type ScoredCards = (
    Vec<(f32, u64, u64, maestria_ports::IndexedCard)>,
    Option<SearchExecutionResource>,
);

impl TantivyFullTextIndex {
    pub(crate) fn index_cards_impl(&self, cards: Vec<IndexedCard>) -> Result<(), PortError> {
        self.with_writer(
            "index cards requires a writable full-text index",
            |writer| {
                for card in cards {
                    writer.delete_term(Term::from_field_text(
                        self.fields.card_key,
                        &card_key(card.artifact_id, card.card_id),
                    ));
                    writer
                        .add_document(self.card_document(&card))
                        .map_err(to_port_error)?;
                }
                Ok(())
            },
        )
    }

    pub(crate) fn search_cards_impl(
        &self,
        query: SearchQuery,
    ) -> Result<BoundedSearch<CardHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "empty card search query",
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
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
        }
        let searcher = self.reader.searcher();
        let fields = vec![self.fields.card_title, self.fields.card_body];
        let parsed_query = parse_query(&self.index, fields, trimmed)?;
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
        let (scored, stopped) = self.score_card_documents(
            &searcher,
            collection.docs,
            collection.truncated,
            &mut meter,
        )?;
        let selected = scored
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|(score, _, _, card)| CardHit {
                card,
                score: score_to_u32(score),
            })
            .collect::<Vec<_>>();
        let completion = finish_results(&mut meter, selected.len(), stopped);
        Ok(meter.done(selected, completion))
    }

    pub(crate) fn search_cards_filtered_impl(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(CardId, ArtifactId) -> Result<bool, PortError>,
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
            "filtered card search result limit",
        )?;
        let mut meter = Meter::new(query.execution_budget);
        if query.limit == 0 {
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
        }
        let searcher = self.reader.searcher();
        let (allowed, authorization_stop) =
            self.allowed_card_keys(&searcher, filter, &mut meter)?;
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
        let fields = vec![self.fields.card_title, self.fields.card_body];
        let parsed_query = parse_query(&self.index, fields, trimmed)?;
        let scoped_query = scope_by_keys(parsed_query, self.fields.card_key, allowed);
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
        let (scored, score_stop) = self.score_card_documents(
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
            .map(|(score, _, _, card)| CardHit {
                card,
                score: score_to_u32(score),
            })
            .collect::<Vec<_>>();
        let completion = finish_results(&mut meter, selected.len(), stopped);
        Ok(meter.done(selected, completion))
    }

    fn score_card_documents(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: Vec<(f32, tantivy::DocAddress)>,
        truncated: bool,
        meter: &mut Meter,
    ) -> Result<ScoredCards, PortError> {
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
                .get_first(self.fields.card_title)
                .and_then(|value| value.as_str())
                .map_or(0, |title| title.len() as u64)
                .saturating_add(
                    document
                        .get_first(self.fields.card_body)
                        .and_then(|value| value.as_str())
                        .map_or(0, |body| body.len() as u64),
                );
            if let Some(resource) = meter.bytes(bytes) {
                stopped = Some(resource);
                break;
            }
            if let Some(resource) = meter.work(1) {
                stopped = Some(resource);
                break;
            }
            let card = self.read_card(&document)?;
            scored.push((score, card.artifact_id.value(), card.card_id.value(), card));
        }
        scored.sort_by(|a, b| {
            descending_score(a.0, b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        Ok((scored, stopped))
    }

    pub(crate) fn allowed_card_keys(
        &self,
        searcher: &tantivy::Searcher,
        filter: &dyn Fn(CardId, ArtifactId) -> Result<bool, PortError>,
        meter: &mut Meter,
    ) -> Result<(BTreeSet<String>, Option<SearchExecutionResource>), PortError> {
        // Same one-past-the-end candidate limit as the chunk walk: a full
        // AllQuery collection must never carry the truncation marker.
        let limit = budget_usize(searcher.num_docs());
        if limit == 0 {
            return Ok((BTreeSet::new(), Some(SearchExecutionResource::Candidates)));
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
            return Ok((BTreeSet::new(), Some(resource)));
        }
        let mut allowed = std::collections::BTreeSet::new();
        let mut stopped = None;
        for (_, address) in collection.docs {
            let (artifact_id, card_id) = self.read_card_identity_at(searcher, address)?;
            if let Some(resource) = meter.bytes(crate::documents::INDEXED_IDENTITY_BYTES) {
                stopped = Some(resource);
                break;
            }
            if !filter(card_id, artifact_id)? {
                continue;
            }
            if let Some(resource) = meter.work(1) {
                stopped = Some(resource);
                break;
            }
            allowed.insert(card_key(artifact_id, card_id));
        }
        if stopped.is_none() && collection.truncated {
            stopped = Some(SearchExecutionResource::Candidates);
        }
        Ok((allowed, stopped))
    }
}
