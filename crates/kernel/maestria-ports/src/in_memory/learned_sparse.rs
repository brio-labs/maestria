use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::execution::{Meter, validate_limit_u32};
use super::store::lock_map;
use crate::{
    BoundedSearch, LearnedSparseIndex, LearnedSparseProvider, PortError, ProviderDisclosure,
    RetentionPolicy, SparseDocument, SparseIdentity, SparseInputKind, SparseSearchHit,
    SparseSearchQuery, SparseTermContribution, SparseTermWeight, SparseVector,
};
use maestria_domain::ChunkId;

#[derive(Clone)]
pub struct InMemoryLearnedSparseProvider {
    identity: SparseIdentity,
    disclosure: ProviderDisclosure,
}

impl InMemoryLearnedSparseProvider {
    pub fn new(identity: SparseIdentity) -> Result<Self, PortError> {
        identity.validate()?;
        Ok(Self {
            identity,
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        })
    }
}

impl LearnedSparseProvider for InMemoryLearnedSparseProvider {
    fn disclosure(&self) -> Option<ProviderDisclosure> {
        Some(self.disclosure.clone())
    }

    fn identity(&self) -> Option<SparseIdentity> {
        Some(self.identity.clone())
    }

    fn encode(
        &self,
        text: &str,
        kind: SparseInputKind,
        identity: SparseIdentity,
    ) -> Result<SparseVector, PortError> {
        if identity != self.identity {
            return Err(PortError::InvalidInputContext {
                context: "sparse provider identity mismatch",
                source: "request identity differs from provider".to_string(),
            });
        }
        if text.trim().is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "sparse input text is empty",
                source: "text must contain a non-whitespace token".to_string(),
            });
        }
        let mut frequencies = BTreeMap::<u32, u32>::new();
        for token in tokenize(text) {
            let term_id = stable_term_id(&token, identity.fingerprint.vocabulary_size);
            frequencies
                .entry(term_id)
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
        }
        if frequencies.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "sparse input has no indexable terms",
                source: "tokenization produced no terms".to_string(),
            });
        }
        let kind_boost = match kind {
            SparseInputKind::Document => 1.0_f32,
            SparseInputKind::Query => 1.25_f32,
        };
        let mut weighted = frequencies
            .into_iter()
            .map(|(term_id, count)| {
                let weight = (count as f32).ln_1p() * kind_boost;
                SparseTermWeight::new(term_id, weight)
            })
            .collect::<Result<Vec<_>, _>>()?;
        weighted.retain(|term| term.weight() >= identity.fingerprint.pruning_threshold);
        weighted.sort_by(|left, right| {
            right
                .weight()
                .total_cmp(&left.weight())
                .then_with(|| left.term_id().cmp(&right.term_id()))
        });
        let max_terms = usize::try_from(identity.fingerprint.max_terms).map_err(|_| {
            PortError::InvalidInputContext {
                context: "sparse max_terms exceeds platform range",
                source: "max_terms does not fit this platform".to_string(),
            }
        })?;
        weighted.truncate(max_terms);
        SparseVector::new(identity, weighted)
    }
}

#[derive(Clone)]
pub struct InMemoryLearnedSparseIndex {
    identity: SparseIdentity,
    documents: Arc<Mutex<Vec<SparseDocument>>>,
}

impl InMemoryLearnedSparseIndex {
    pub fn new(identity: SparseIdentity) -> Result<Self, PortError> {
        identity.validate()?;
        Ok(Self {
            identity,
            documents: Arc::new(Mutex::new(Vec::new())),
        })
    }
    fn search_with_filter(
        &self,
        query: SparseSearchQuery,
        filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<SparseSearchHit>, PortError> {
        if query.vector.identity() != &self.identity {
            return Err(PortError::InvalidInputContext {
                context: "sparse query identity mismatch",
                source: "query identity differs from index".to_string(),
            });
        }
        validate_limit_u32(
            query.limit,
            query.execution_budget,
            "sparse search result limit",
        )?;
        let contribution_cap = usize::try_from(query.max_contributions).map_err(|_| {
            PortError::InvalidInputContext {
                context: "sparse contribution cap exceeds platform range",
                source: "max_contributions does not fit this platform".to_string(),
            }
        })?;
        let mut meter = Meter::new(query.execution_budget);
        if query.limit == 0 {
            return Ok(meter.complete(Vec::new()));
        }
        let guard = lock_map(&self.documents, "learned sparse index lock poisoned")?;
        let (hits, stopped) = collect_sparse_hits(
            guard.as_slice(),
            &query,
            filter,
            contribution_cap,
            &mut meter,
        )?;
        Ok(crate::learned_sparse::finish_sparse_search(
            meter,
            hits,
            maestria_domain::saturating_usize(u64::from(query.limit)),
            stopped,
        ))
    }
}

fn collect_sparse_hits(
    documents: &[SparseDocument],
    query: &SparseSearchQuery,
    filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    contribution_cap: usize,
    meter: &mut Meter,
) -> Result<
    (
        Vec<SparseSearchHit>,
        Option<maestria_domain::SearchExecutionResource>,
    ),
    PortError,
> {
    let mut hits = Vec::new();
    let mut stopped = None;
    for document in documents {
        if let Some(resource) = meter.candidate() {
            stopped = Some(resource);
            break;
        }
        if !filter(document.chunk_id)? {
            continue;
        }
        let bytes = maestria_domain::saturating_u64(
            document
                .vector
                .terms()
                .len()
                .saturating_add(query.vector.terms().len())
                .saturating_mul(std::mem::size_of::<SparseTermWeight>()),
        );
        if let Some(resource) = meter.bytes(bytes) {
            stopped = Some(resource);
            break;
        }
        let work = maestria_domain::saturating_u64(
            document
                .vector
                .terms()
                .len()
                .saturating_add(query.vector.terms().len()),
        );
        if let Some(resource) = meter.work(work) {
            stopped = Some(resource);
            break;
        }
        let contributions =
            crate::learned_sparse::dot_contributions(document.vector.terms(), query.vector.terms());
        if contributions.is_empty() {
            continue;
        }
        let score = contributions
            .iter()
            .map(|(_, value)| *value)
            .fold(0.0_f64, |total, value| total + value);
        if !score.is_finite() || score <= 0.0 {
            continue;
        }
        let mut trace = contributions
            .into_iter()
            .map(|(term_id, value)| SparseTermContribution {
                term_id,
                contribution_micros: crate::learned_sparse::fixed_micros(value),
            })
            .collect::<Vec<_>>();
        trace.sort_by(|left, right| {
            right
                .contribution_micros
                .cmp(&left.contribution_micros)
                .then_with(|| left.term_id.cmp(&right.term_id))
        });
        trace.truncate(contribution_cap);
        hits.push(SparseSearchHit {
            chunk_id: document.chunk_id,
            score_micros: crate::learned_sparse::fixed_micros(score),
            contributions: trace,
        });
    }
    Ok((hits, stopped))
}

impl LearnedSparseIndex for InMemoryLearnedSparseIndex {
    fn identity(&self) -> Option<SparseIdentity> {
        Some(self.identity.clone())
    }

    fn index_documents(&self, documents: Vec<SparseDocument>) -> Result<(), PortError> {
        if documents
            .iter()
            .any(|document| document.vector.identity() != &self.identity)
        {
            return Err(PortError::InvalidInputContext {
                context: "sparse document identity mismatch",
                source: "document identity differs from index".to_string(),
            });
        }
        let mut guard = lock_map(&self.documents, "learned sparse index lock poisoned")?;
        for document in documents {
            if let Some(position) = guard
                .iter()
                .position(|existing| existing.chunk_id == document.chunk_id)
            {
                guard[position] = document;
            } else {
                guard.push(document);
            }
        }
        guard.sort_by_key(|document| document.chunk_id);
        Ok(())
    }

    fn search(
        &self,
        query: SparseSearchQuery,
    ) -> Result<BoundedSearch<SparseSearchHit>, PortError> {
        self.search_with_filter(query, &|_| Ok(true))
    }

    fn search_filtered(
        &self,
        query: SparseSearchQuery,
        filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<SparseSearchHit>, PortError> {
        self.search_with_filter(query, filter)
    }

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError> {
        let mut guard = lock_map(&self.documents, "learned sparse index lock poisoned")?;
        guard.retain(|document| !chunk_ids.contains(&document.chunk_id));
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        let mut guard = lock_map(&self.documents, "learned sparse index lock poisoned")?;
        guard.clear();
        Ok(())
    }
}

fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
}

fn stable_term_id(token: &str, vocabulary_size: u32) -> u32 {
    let hash = token
        .bytes()
        .fold(2_166_136_261_u32, |value, byte| value ^ u32::from(byte));
    hash.wrapping_mul(16_777_619) % vocabulary_size
}
