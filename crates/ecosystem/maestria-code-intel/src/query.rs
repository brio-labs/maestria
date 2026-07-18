//! Query execution over in-memory symbol records.

use crate::{CodeQuery, QueryResult, QuerySummary, SymbolRecord};
use regex::Regex;
pub(crate) const MAX_QUERY_LIMIT: usize = 1_000;

/// Apply a bounded query over extracted symbols.
pub(crate) fn execute_query(
    symbols: &[SymbolRecord],
    query: CodeQuery,
    limit: usize,
) -> QueryResult {
    let limit = limit.min(MAX_QUERY_LIMIT);
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
                return QueryResult {
                    summary: QuerySummary {
                        query,
                        matched: 0,
                        returned: 0,
                        truncated: false,
                        limit,
                        regex_error: Some(error.to_string()),
                    },
                    records: Vec::new(),
                };
            }
        },
    };

    let mut matched = 0;
    let mut selected: Vec<&SymbolRecord> = Vec::with_capacity(limit);
    for symbol in symbols.iter().filter(|symbol| matcher.matches(symbol)) {
        matched += 1;
        if limit == 0 {
            continue;
        }
        selected.push(symbol);
        selected.sort_by(|left, right| symbol_order(left, right));
        if selected.len() > limit {
            selected.pop();
        }
    }

    let records: Vec<SymbolRecord> = selected.into_iter().cloned().collect();

    QueryResult {
        summary: QuerySummary {
            query,
            matched,
            returned: records.len(),
            truncated: records.len() < matched,
            limit,
            regex_error: None,
        },
        records,
    }
}

fn symbol_order(left: &SymbolRecord, right: &SymbolRecord) -> std::cmp::Ordering {
    (
        left.provenance.file_path.as_str(),
        left.provenance.source_range.start_line,
        left.qualified_name.as_str(),
    )
        .cmp(&(
            right.provenance.file_path.as_str(),
            right.provenance.source_range.start_line,
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
