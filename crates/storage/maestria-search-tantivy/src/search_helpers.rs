use maestria_domain::SearchExecutionResource;
use maestria_ports::PortError;
use std::collections::BTreeSet;
use tantivy::{
    DocAddress, Index, Searcher, TERMINATED, Term,
    query::{BooleanQuery, EnableScoring, Query, QueryParser, Scorer, TermSetQuery},
    schema::Field,
};

use crate::error::to_port_error;
use crate::execution::Meter;

pub(super) struct BoundedCollection {
    pub(super) docs: Vec<(f32, DocAddress)>,
    pub(super) truncated: bool,
    pub(super) stopped: Option<SearchExecutionResource>,
}

/// Collects at most `candidate_limit` live matches.
///
/// Tantivy's `TopDocs` collector bounds the retained heap, not the scorer
/// traversal. This collector drives each scorer directly and meters every
/// scorer step so work budgets bound actual index traversal as well as the
/// candidate budget bounds the returned page.
pub(super) fn collect_bounded(
    searcher: &Searcher,
    query: &dyn Query,
    offset: usize,
    limit: usize,
    candidate_limit: usize,
    meter: &mut Meter,
) -> Result<BoundedCollection, PortError> {
    let requested = offset.saturating_add(limit).min(candidate_limit);
    if requested == 0 {
        return Ok(BoundedCollection {
            docs: Vec::new(),
            truncated: false,
            stopped: None,
        });
    }
    let weight = query
        .weight(EnableScoring::enabled_from_searcher(searcher))
        .map_err(to_port_error)?;
    let mut docs = Vec::new();
    let mut truncated = false;
    let mut stopped = None;
    'segments: for (segment_ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
        let mut scorer = weight.scorer(segment_reader, 1.0).map_err(to_port_error)?;
        let alive = segment_reader.alive_bitset();
        let mut doc = scorer.doc();
        while doc != TERMINATED {
            if let Some(resource) = meter.work(1) {
                stopped = Some(resource);
                break 'segments;
            }
            if alive.is_none_or(|bitset| bitset.is_alive(doc)) {
                if docs.len() == candidate_limit {
                    truncated = true;
                    break 'segments;
                }
                docs.push((scorer.score(), DocAddress::new(segment_ord as u32, doc)));
            }
            doc = scorer.advance();
        }
    }
    docs.sort_by(|(left_score, left_address), (right_score, right_address)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| left_address.segment_ord.cmp(&right_address.segment_ord))
            .then_with(|| left_address.doc_id.cmp(&right_address.doc_id))
    });
    docs.truncate(requested);
    Ok(BoundedCollection {
        docs,
        truncated,
        stopped,
    })
}
pub(crate) fn parse_query(
    index: &Index,
    fields: Vec<Field>,
    text: &str,
) -> Result<Box<dyn Query>, PortError> {
    let parser = QueryParser::for_index(index, fields);
    parser
        .parse_query(text)
        .map_err(|error| PortError::invalid_input("invalid search query", error.to_string()))
}

pub(crate) fn scope_by_keys(
    parsed: Box<dyn Query>,
    key_field: Field,
    keys: BTreeSet<String>,
) -> BooleanQuery {
    BooleanQuery::intersection(vec![
        parsed,
        Box::new(TermSetQuery::new(
            keys.into_iter()
                .map(|key| Term::from_field_text(key_field, &key)),
        )),
    ])
}
