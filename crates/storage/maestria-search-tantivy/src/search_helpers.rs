use maestria_ports::PortError;
use tantivy::{
    DocAddress, Searcher, TERMINATED,
    query::{EnableScoring, Query, Scorer},
};

use crate::tantivy_index::to_port_error;

/// Collects at most `candidate_limit` live matches.
///
/// Tantivy's `TopDocs` collector bounds the retained heap, not the scorer
/// traversal. This collector drives each scorer directly so the candidate
/// budget bounds the actual index walk as well as the returned page.
pub(super) fn collect_bounded(
    searcher: &Searcher,
    query: &dyn Query,
    offset: usize,
    limit: usize,
    candidate_limit: usize,
) -> Result<(Vec<(f32, DocAddress)>, bool), PortError> {
    let requested = offset.saturating_add(limit).min(candidate_limit);
    if requested == 0 {
        return Ok((Vec::new(), false));
    }
    let weight = query
        .weight(EnableScoring::enabled_from_searcher(searcher))
        .map_err(to_port_error)?;
    let mut docs = Vec::with_capacity(requested);
    let mut truncated = false;
    for (segment_ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
        let mut scorer = weight.scorer(segment_reader, 1.0).map_err(to_port_error)?;
        let alive = segment_reader.alive_bitset();
        let mut doc = scorer.doc();
        while doc != TERMINATED {
            if alive.is_none_or(|bitset| bitset.is_alive(doc)) {
                if docs.len() == candidate_limit {
                    truncated = true;
                    break;
                }
                docs.push((scorer.score(), DocAddress::new(segment_ord as u32, doc)));
            }
            doc = scorer.advance();
        }
        if truncated {
            break;
        }
    }
    docs.sort_by(|(left_score, left_address), (right_score, right_address)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| left_address.segment_ord.cmp(&right_address.segment_ord))
            .then_with(|| left_address.doc_id.cmp(&right_address.doc_id))
    });
    docs.truncate(requested);
    Ok((docs, truncated))
}
