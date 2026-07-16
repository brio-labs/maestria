use tantivy::{DocAddress, Searcher, collector::TopDocs, query::Query};

use super::to_port_error;

pub(super) fn collect_tie_complete(
    searcher: &Searcher,
    query: &dyn Query,
    offset: usize,
    limit: usize,
) -> Result<Vec<(f32, DocAddress)>, super::PortError> {
    let total_docs = searcher.num_docs() as usize;
    let safe_limit = limit.min(total_docs).max(1);
    let requested = offset.saturating_add(limit).min(total_docs);
    if requested == 0 {
        return Ok(Vec::new());
    }
    let mut documents = searcher
        .search(query, &TopDocs::with_limit(requested).order_by_score())
        .map_err(to_port_error)?;
    if documents.len() < requested {
        return Ok(documents);
    }
    let boundary = documents.last().map(|(score, _)| *score);
    let Some(boundary) = boundary else {
        return Ok(documents);
    };
    let mut next_offset = requested;
    loop {
        let page = searcher
            .search(
                query,
                &TopDocs::with_limit(safe_limit)
                    .and_offset(next_offset)
                    .order_by_score(),
            )
            .map_err(to_port_error)?;
        if page.is_empty() {
            break;
        }
        let mut found_boundary = false;
        let mut found_lower = false;
        for (score, address) in page {
            if score == boundary {
                documents.push((score, address));
                found_boundary = true;
            } else if score < boundary {
                found_lower = true;
            }
        }
        if found_lower || !found_boundary {
            break;
        }
        next_offset = next_offset.saturating_add(safe_limit);
    }
    Ok(documents)
}
