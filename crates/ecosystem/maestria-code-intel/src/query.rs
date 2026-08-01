//! Query execution over in-memory symbol records.

use crate::{CodeQuery, QueryResult, QuerySummary, SymbolRecord};
use regex::Regex;

/// Apply a bounded query over extracted symbols.
pub(crate) fn execute_query<E, F>(
    symbols: &[SymbolRecord],
    query: CodeQuery,
    limit: usize,
    authorize: &mut F,
) -> Result<QueryResult, E>
where
    F: FnMut(&SymbolRecord) -> Result<bool, E>,
{
    let matcher = match &query {
        CodeQuery::All => QueryMatcher::All,
        CodeQuery::Symbol { pattern } => QueryMatcher::Contains {
            pattern,
            mode: MatchMode::Symbol,
        },
        CodeQuery::Path { pattern } => QueryMatcher::Contains {
            pattern,
            mode: MatchMode::Path,
        },
        CodeQuery::Regex { pattern } => match Regex::new(pattern) {
            Ok(regex) => QueryMatcher::Regex(regex),
            Err(error) => {
                return Ok(QueryResult {
                    summary: QuerySummary {
                        query,
                        matched: 0,
                        returned: 0,
                        truncated: false,
                        scanned: 0,
                        limit,
                        regex_error: Some(error.to_string()),
                    },
                    records: Vec::new(),
                });
            }
        },
    };

    let mut matched = 0;
    let mut scanned = 0;
    let mut scan_exhausted = false;
    let mut selected: Vec<&SymbolRecord> = Vec::with_capacity(limit);
    for symbol in symbols {
        if scanned >= limit {
            scan_exhausted = true;
            break;
        }
        scanned = scanned.saturating_add(1);
        if !matcher.matches(symbol) {
            continue;
        }
        if !authorize(symbol)? {
            continue;
        }
        matched += 1;
        selected.push(symbol);
        selected.sort_by(|left, right| symbol_order(left, right));
        if selected.len() > limit {
            selected.pop();
        }
    }
    let records: Vec<SymbolRecord> = selected.into_iter().cloned().collect();

    Ok(QueryResult {
        summary: QuerySummary {
            query,
            matched,
            returned: records.len(),
            truncated: scan_exhausted || records.len() < matched,
            scanned,
            limit,
            regex_error: None,
        },
        records,
    })
}

fn symbol_order(left: &SymbolRecord, right: &SymbolRecord) -> std::cmp::Ordering {
    (
        left.provenance.file_path.as_str(),
        left.provenance.source_range.start_line(),
        left.qualified_name.as_str(),
    )
        .cmp(&(
            right.provenance.file_path.as_str(),
            right.provenance.source_range.start_line(),
            right.qualified_name.as_str(),
        ))
}

enum MatchMode {
    Symbol,
    Path,
}

enum QueryMatcher<'a> {
    All,
    Contains { pattern: &'a str, mode: MatchMode },
    Regex(Regex),
}

impl<'a> QueryMatcher<'a> {
    fn matches(&self, symbol: &SymbolRecord) -> bool {
        match self {
            Self::All => true,
            Self::Contains { pattern, mode } => match mode {
                MatchMode::Symbol => {
                    symbol.name.contains(pattern) || symbol.qualified_name.contains(pattern)
                }
                MatchMode::Path => symbol.provenance.file_path.contains(pattern),
            },
            Self::Regex(regex) => {
                regex.is_match(&symbol.name)
                    || regex.is_match(&symbol.qualified_name)
                    || regex.is_match(&symbol.provenance.file_path)
                    || symbol
                        .signature
                        .as_deref()
                        .is_some_and(|signature| regex.is_match(signature))
                    || symbol.imports.iter().any(|import| regex.is_match(import))
            }
        }
    }
}
