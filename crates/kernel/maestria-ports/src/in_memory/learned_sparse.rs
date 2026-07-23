use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::{
    ChunkId, LearnedSparseIndex, LearnedSparseProvider, PortError, ProviderDisclosure,
    RetentionPolicy, SparseDocument, SparseIdentity, SparseInputKind, SparseSearchHit,
    SparseSearchQuery, SparseTermContribution, SparseTermWeight, SparseVector,
};

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
            return Err(PortError::InvalidInput {
                message: "sparse request identity does not match provider".to_string(),
            });
        }
        if text.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "sparse input text must not be empty".to_string(),
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
            return Err(PortError::InvalidInput {
                message: "sparse input produced no indexable terms".to_string(),
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
            PortError::InvalidInput {
                message: "sparse max_terms does not fit this platform".to_string(),
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
        filter: &dyn Fn(ChunkId) -> bool,
    ) -> Result<Vec<SparseSearchHit>, PortError> {
        if query.vector.identity() != &self.identity {
            return Err(PortError::InvalidInput {
                message: "sparse query identity does not match index".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        let contribution_cap =
            usize::try_from(query.max_contributions).map_err(|_| PortError::InvalidInput {
                message: "sparse contribution cap does not fit this platform".to_string(),
            })?;
        let guard = self.documents.lock().map_err(|_| PortError::Internal {
            message: "learned sparse index lock poisoned".to_string(),
        })?;
        let mut hits = Vec::new();
        for document in guard.iter() {
            if !filter(document.chunk_id) {
                continue;
            }
            let contributions = dot_contributions(document.vector.terms(), query.vector.terms());
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
                    contribution_micros: fixed_micros(value),
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
                score_micros: fixed_micros(score),
                contributions: trace,
            });
        }
        hits.sort_by(|left, right| {
            right
                .score_micros
                .cmp(&left.score_micros)
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });
        let mut limit = usize::MAX;
        if let Ok(v) = usize::try_from(query.limit) {
            limit = v;
        }
        hits.truncate(limit);
        Ok(hits)
    }
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
            return Err(PortError::InvalidInput {
                message: "sparse document identity does not match index".to_string(),
            });
        }
        let mut guard = self.documents.lock().map_err(|_| PortError::Internal {
            message: "learned sparse index lock poisoned".to_string(),
        })?;
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

    fn search(&self, query: SparseSearchQuery) -> Result<Vec<SparseSearchHit>, PortError> {
        self.search_with_filter(query, &|_| true)
    }

    fn search_filtered(
        &self,
        query: SparseSearchQuery,
        filter: &dyn Fn(ChunkId) -> bool,
    ) -> Result<Vec<SparseSearchHit>, PortError> {
        self.search_with_filter(query, filter)
    }

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError> {
        let mut guard = self.documents.lock().map_err(|_| PortError::Internal {
            message: "learned sparse index lock poisoned".to_string(),
        })?;
        guard.retain(|document| !chunk_ids.contains(&document.chunk_id));
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        let mut guard = self.documents.lock().map_err(|_| PortError::Internal {
            message: "learned sparse index lock poisoned".to_string(),
        })?;
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

fn dot_contributions(document: &[SparseTermWeight], query: &[SparseTermWeight]) -> Vec<(u32, f64)> {
    let mut left = 0_usize;
    let mut right = 0_usize;
    let mut contributions = Vec::new();
    while left < document.len() && right < query.len() {
        let document_term = document[left];
        let query_term = query[right];
        match document_term.term_id().cmp(&query_term.term_id()) {
            std::cmp::Ordering::Less => left += 1,
            std::cmp::Ordering::Greater => right += 1,
            std::cmp::Ordering::Equal => {
                contributions.push((
                    document_term.term_id(),
                    f64::from(document_term.weight()) * f64::from(query_term.weight()),
                ));
                left += 1;
                right += 1;
            }
        }
    }
    contributions
}

fn fixed_micros(value: f64) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value * 1_000_000.0).round().min(f64::from(u32::MAX)) as u32
}
