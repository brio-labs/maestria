//! The generic in-memory lexical search lane (Rule 13/16: one shared
//! pipeline, typed per corpus). [`LexicalLane`] names everything a corpus
//! must provide — record identity, field extraction, metering, hit
//! construction, and ranking accessors — so index maintenance and budgeted
//! search are implemented once and reused by the chunk and card lanes
//! instead of being duplicated per corpus type.

use super::super::execution::{Meter, validate_limit};
use super::matching::{build_metadata, process_field_match, validate_and_prepare_query};
use crate::lexical::{LexicalHitMetadata, LexicalQuery, MatchMode};
use crate::{BoundedSearch, PortError};
use maestria_domain::{ArtifactId, SearchExecutionResource};
use std::sync::{Arc, Mutex};

/// One searchable corpus lane: identity, field matching, metering, hit
/// construction, and ranking for a single indexed record type.
pub(super) trait LexicalLane {
    type Id: Ord + Clone + Copy + std::fmt::Debug;
    type Field: std::fmt::Debug + Copy + PartialEq;
    type Record: Clone;
    type Hit;

    fn id(record: &Self::Record) -> Self::Id;
    fn artifact_id(record: &Self::Record) -> ArtifactId;
    fn is_id_field(field: &Self::Field) -> bool;
    fn id_key(record: &Self::Record) -> String;
    fn field_value<'a>(record: &'a Self::Record, field: &Self::Field) -> Option<&'a String>;
    fn field_len(record: &Self::Record, field: &Self::Field) -> usize;
    fn metered_bytes(record: &Self::Record) -> u64;
    fn build_hit(record: Self::Record, metadata: LexicalHitMetadata) -> Self::Hit;
    fn hit_score(hit: &Self::Hit) -> f32;
    fn hit_artifact_id(hit: &Self::Hit) -> ArtifactId;
    fn hit_item_id(hit: &Self::Hit) -> Self::Id;
    fn set_hit_rank(hit: &mut Self::Hit, rank: u32);
}

/// Replace-or-append indexing: an incoming record with the same (artifact,
/// item) identity as an existing one replaces it, preserving lane order.
pub(super) fn index_lane<R: LexicalLane>(
    store: &Arc<Mutex<Vec<R::Record>>>,
    records: Vec<R::Record>,
) -> Result<(), PortError> {
    let mut guard = store.lock().map_err(|_| PortError::InternalContext {
        context: "lexical index lock poisoned",
        source: "index mutex is poisoned".to_string(),
    })?;
    for record in &records {
        guard.retain(|existing| {
            R::artifact_id(existing) != R::artifact_id(record) || R::id(existing) != R::id(record)
        });
    }
    guard.extend(records);
    Ok(())
}

/// Budgeted lane search: validate and normalize the query, meter candidate,
/// byte, work, and result budgets, then page, rank, and annotate the hits.
pub(super) fn search_lane<R: LexicalLane>(
    store: &Arc<Mutex<Vec<R::Record>>>,
    query: LexicalQuery<R::Field>,
    filter: &dyn Fn(R::Id, ArtifactId) -> Result<bool, PortError>,
    empty_query_message: &'static str,
    fields_message: &'static str,
    limit_message: &'static str,
) -> Result<BoundedSearch<R::Hit>, PortError> {
    let needle = validate_and_prepare_query(&query.q, query.mode, empty_query_message)?;
    if query.fields.is_empty() {
        return Err(PortError::InvalidInputContext {
            context: fields_message,
            source: "at least one field is required".to_string(),
        });
    }
    validate_limit(query.limit, query.execution_budget, limit_message)?;
    let mut meter = Meter::new(query.execution_budget);
    if query.limit == 0 {
        return Ok(meter.complete(Vec::new()));
    }
    let guard = store.lock().map_err(|_| PortError::InternalContext {
        context: "lexical index lock poisoned",
        source: "index mutex is poisoned".to_string(),
    })?;
    let (hits, mut stopped) =
        collect_lane_hits::<R>(guard.as_slice(), &query, &needle, filter, &mut meter)?;
    let selected = page_and_rank_hits::<R>(hits, query.offset, query.limit);
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

fn collect_lane_hits<R: LexicalLane>(
    records: &[R::Record],
    query: &LexicalQuery<R::Field>,
    needle: &str,
    filter: &dyn Fn(R::Id, ArtifactId) -> Result<bool, PortError>,
    meter: &mut Meter,
) -> Result<(Vec<R::Hit>, Option<SearchExecutionResource>), PortError> {
    let mut hits = Vec::new();
    let mut stopped = None;
    for record in records {
        if let Some(resource) = meter.candidate() {
            stopped = Some(resource);
            break;
        }
        if !filter(R::id(record), R::artifact_id(record))? {
            continue;
        }
        if let Some(resource) = meter.bytes(R::metered_bytes(record)) {
            stopped = Some(resource);
            break;
        }
        let work =
            u64::try_from(query.fields.len()).map_or(u64::MAX, |fields| fields.saturating_add(1));
        if let Some(resource) = meter.work(work) {
            stopped = Some(resource);
            break;
        }
        let mut matched_field = None;
        let mut raw_score = 0.0;
        for field in &query.fields {
            if R::is_id_field(&field.field) {
                let key = R::id_key(record);
                let matches = match query.mode {
                    MatchMode::Contains => key.contains(needle),
                    MatchMode::Exact => key == needle,
                };
                if matches {
                    matched_field = Some("id".to_string());
                    raw_score += field.boost;
                }
                continue;
            }
            process_field_match(
                R::field_value(record, &field.field),
                R::field_len(record, &field.field),
                field,
                query.mode,
                needle,
                &mut matched_field,
                &mut raw_score,
            );
        }
        if let Some(metadata) = build_metadata(matched_field, query.mode, raw_score) {
            hits.push(R::build_hit(record.clone(), metadata));
        }
    }
    Ok((hits, stopped))
}

/// Deterministic paging and ranking: score descending, then artifact and
/// item identity ascending, with the rank set from the page position.
fn page_and_rank_hits<R: LexicalLane>(
    mut hits: Vec<R::Hit>,
    offset: usize,
    limit: usize,
) -> Vec<R::Hit> {
    hits.sort_by(|a, b| {
        let score_order = match R::hit_score(b).partial_cmp(&R::hit_score(a)) {
            Some(ordering) => ordering,
            None => std::cmp::Ordering::Equal,
        };
        score_order
            .then_with(|| R::hit_artifact_id(a).cmp(&R::hit_artifact_id(b)))
            .then_with(|| R::hit_item_id(a).cmp(&R::hit_item_id(b)))
    });
    hits.into_iter()
        .enumerate()
        .skip(offset)
        .take(limit)
        .map(|(i, mut hit)| {
            R::set_hit_rank(&mut hit, (i + 1) as u32);
            hit
        })
        .collect()
}
