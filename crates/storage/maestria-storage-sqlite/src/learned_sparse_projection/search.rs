use std::cmp::Ordering;

use maestria_domain::{
    SearchExecution, SearchExecutionBudget, SearchExecutionCompletion, SearchExecutionResource,
    SearchExecutionUsage,
};
use maestria_ports::{
    BoundedSearch, PortError, SparseIdentity, SparseSearchHit, SparseSearchQuery,
};

use super::{lifecycle, search_storage, storage};
use crate::SqliteStore;

struct Meter {
    budget: SearchExecutionBudget,
    usage: SearchExecutionUsage,
}

impl Meter {
    fn new(budget: SearchExecutionBudget) -> Self {
        Self {
            budget,
            usage: SearchExecutionUsage::default(),
        }
    }

    fn candidate(&mut self) -> Option<SearchExecutionResource> {
        if self.usage.candidates >= self.budget.max_candidates() {
            return Some(SearchExecutionResource::Candidates);
        }
        self.usage.candidates = self.usage.candidates.saturating_add(1);
        None
    }

    fn work(&mut self, units: u64) -> Option<SearchExecutionResource> {
        if units
            > self
                .budget
                .max_work_units()
                .saturating_sub(self.usage.work_units)
        {
            return Some(SearchExecutionResource::WorkUnits);
        }
        self.usage.work_units = self.usage.work_units.saturating_add(units);
        None
    }

    fn bytes(&mut self, bytes: u64) -> Option<SearchExecutionResource> {
        let Some(limit) = self.budget.max_bytes_read() else {
            self.usage.bytes_read = self.usage.bytes_read.saturating_add(bytes);
            return None;
        };
        if bytes > limit.get().saturating_sub(self.usage.bytes_read) {
            return Some(SearchExecutionResource::BytesRead);
        }
        self.usage.bytes_read = self.usage.bytes_read.saturating_add(bytes);
        None
    }

    fn result(&mut self) -> Option<SearchExecutionResource> {
        if self.usage.results >= self.budget.max_results() {
            return Some(SearchExecutionResource::Results);
        }
        self.usage.results = self.usage.results.saturating_add(1);
        None
    }

    fn complete<T>(self, hits: Vec<T>) -> BoundedSearch<T> {
        BoundedSearch::new(
            hits,
            SearchExecution::new(self.budget, self.usage, SearchExecutionCompletion::Complete),
        )
    }

    fn exhausted<T>(self, hits: Vec<T>, resource: SearchExecutionResource) -> BoundedSearch<T> {
        BoundedSearch::new(
            hits,
            SearchExecution::new(
                self.budget,
                self.usage,
                SearchExecutionCompletion::Exhausted(resource),
            ),
        )
    }
}
struct SearchVisitor<'a> {
    query: SparseSearchQuery,
    filter: &'a dyn Fn(maestria_domain::ChunkId) -> Result<bool, PortError>,
    contribution_cap: usize,
    meter: Meter,
    hits: Vec<SparseSearchHit>,
    stopped: Option<SearchExecutionResource>,
}

impl SearchVisitor<'_> {
    fn finish(mut self) -> Result<BoundedSearch<SparseSearchHit>, PortError> {
        self.hits.sort_by(|left, right| {
            right
                .score_micros
                .cmp(&left.score_micros)
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });
        let limit =
            usize::try_from(self.query.limit).map_err(|_| PortError::InvalidInputContext {
                context: "sparse result limit",
                source: "result limit exceeds platform range".to_string(),
            })?;
        self.hits.truncate(limit);
        for _ in &self.hits {
            if let Some(resource) = self.meter.result() {
                self.stopped = Some(resource);
                break;
            }
        }
        Ok(match self.stopped {
            Some(resource) => self.meter.exhausted(self.hits, resource),
            None => self.meter.complete(self.hits),
        })
    }
}

impl search_storage::DocumentVisitor for SearchVisitor<'_> {
    fn before_load(
        &mut self,
        document: search_storage::DocumentMetadata,
    ) -> Result<search_storage::DocumentLoadDecision, PortError> {
        if let Some(resource) = self.meter.candidate() {
            self.stopped = Some(resource);
            return Ok(search_storage::DocumentLoadDecision::Stop);
        }
        if !(self.filter)(document.chunk_id)? {
            return Ok(search_storage::DocumentLoadDecision::Skip);
        }
        if let Some(resource) = self.meter.bytes(document.encoded_bytes) {
            self.stopped = Some(resource);
            return Ok(search_storage::DocumentLoadDecision::Stop);
        }
        Ok(search_storage::DocumentLoadDecision::Load)
    }

    fn after_load(
        &mut self,
        document: storage::StoredDocument,
    ) -> Result<search_storage::DocumentVisit, PortError> {
        let work = u64::try_from(
            document
                .vector
                .terms()
                .len()
                .saturating_add(self.query.vector.terms().len()),
        )
        .map_err(|_| PortError::InvalidInputContext {
            context: "sparse search work",
            source: "term count exceeds platform range".to_string(),
        })?;
        if let Some(resource) = self.meter.work(work) {
            self.stopped = Some(resource);
            return Ok(search_storage::DocumentVisit::Stop);
        }
        if let Some(hit) = score_document(&self.query, &document, self.contribution_cap)? {
            self.hits.push(hit);
        }
        Ok(search_storage::DocumentVisit::Continue)
    }
}

pub(super) fn execute(
    store: &SqliteStore,
    identity: &SparseIdentity,
    query: SparseSearchQuery,
    filter: &dyn Fn(maestria_domain::ChunkId) -> Result<bool, PortError>,
) -> Result<BoundedSearch<SparseSearchHit>, PortError> {
    validate_query(identity, &query)?;
    let lifecycle = lifecycle::read(store, identity)?;
    if !matches!(
        lifecycle,
        maestria_domain::IndexLifecycle::Shadow | maestria_domain::IndexLifecycle::Active
    ) {
        return Err(PortError::Conflict {
            message: "sparse projection is not searchable in its current lifecycle".to_string(),
        });
    }
    let contribution_cap =
        usize::try_from(query.max_contributions).map_err(|_| PortError::InvalidInputContext {
            context: "sparse contribution cap",
            source: "contribution cap exceeds platform range".to_string(),
        })?;
    let execution_budget = query.execution_budget;
    let max_candidates = execution_budget.max_candidates();
    let mut visitor = SearchVisitor {
        query,
        filter,
        contribution_cap,
        meter: Meter::new(execution_budget),
        hits: Vec::new(),
        stopped: None,
    };
    search_storage::visit_documents(store, identity, max_candidates, &mut visitor)?;
    visitor.finish()
}

fn validate_query(identity: &SparseIdentity, query: &SparseSearchQuery) -> Result<(), PortError> {
    if query.vector.identity() != identity {
        return Err(PortError::InvalidInputContext {
            context: "sparse query identity mismatch",
            source: "query identity differs from projection identity".to_string(),
        });
    }
    if u64::from(query.limit) != query.execution_budget.max_results() {
        return Err(PortError::InvalidInputContext {
            context: "sparse search result limit",
            source: "query limit and execution budget max_results must agree".to_string(),
        });
    }
    if query.limit == 0 {
        return Err(PortError::InvalidInputContext {
            context: "sparse search result limit",
            source: "result limit must be positive".to_string(),
        });
    }
    if query.max_contributions == 0 {
        return Err(PortError::InvalidInputContext {
            context: "sparse contribution cap",
            source: "contribution cap must be positive".to_string(),
        });
    }
    Ok(())
}

fn score_document(
    query: &SparseSearchQuery,
    document: &storage::StoredDocument,
    contribution_cap: usize,
) -> Result<Option<SparseSearchHit>, PortError> {
    let contributions = dot_contributions(document, query);
    if contributions.is_empty() {
        return Ok(None);
    }
    let score = contributions
        .iter()
        .map(|(_, value)| *value)
        .fold(0.0_f64, |total, value| total + value);
    if !score.is_finite() || score <= 0.0 {
        return Ok(None);
    }
    let mut trace = contributions
        .into_iter()
        .map(|(term_id, value)| {
            let scaled = value * 1_000_000.0;
            let contribution_micros = if scaled >= f64::from(u32::MAX) {
                u32::MAX
            } else {
                scaled.round() as u32
            };
            (term_id, contribution_micros)
        })
        .map(
            |(term_id, contribution_micros)| maestria_ports::SparseTermContribution {
                term_id,
                contribution_micros,
            },
        )
        .collect::<Vec<_>>();
    trace.sort_by(|left, right| {
        right
            .contribution_micros
            .cmp(&left.contribution_micros)
            .then_with(|| left.term_id.cmp(&right.term_id))
    });
    trace.truncate(contribution_cap);
    let score_micros = if score * 1_000_000.0 >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        (score * 1_000_000.0).round() as u32
    };
    Ok(Some(SparseSearchHit {
        chunk_id: document.chunk_id,
        score_micros,
        contributions: trace,
    }))
}

fn dot_contributions(
    document: &storage::StoredDocument,
    query: &SparseSearchQuery,
) -> Vec<(u32, f64)> {
    let mut contributions = Vec::new();
    let mut document_index = 0;
    let mut query_index = 0;
    let document_terms = document.vector.terms();
    let query_terms = query.vector.terms();
    while document_index < document_terms.len() && query_index < query_terms.len() {
        match document_terms[document_index]
            .term_id()
            .cmp(&query_terms[query_index].term_id())
        {
            Ordering::Less => document_index += 1,
            Ordering::Greater => query_index += 1,
            Ordering::Equal => {
                contributions.push((
                    document_terms[document_index].term_id(),
                    f64::from(document_terms[document_index].weight())
                        * f64::from(query_terms[query_index].weight()),
                ));
                document_index += 1;
                query_index += 1;
            }
        }
    }
    contributions
}
