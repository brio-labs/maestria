//! Query execution over in-memory symbol records.

use crate::{CodeQuery, QueryResult, QuerySummary, SymbolRecord};
use regex::Regex;
use std::collections::BTreeSet;

/// Apply a bounded query over extracted symbols. `changed_files` carries the
/// changed file set for `CodeQuery::Changed` (computed by the caller from the
/// persisted delta or live git state); when absent, a `Changed` query matches
/// nothing rather than fabricating matches.
pub(crate) fn execute_query<E, F>(
    symbols: &[SymbolRecord],
    query: CodeQuery,
    limit: usize,
    changed_files: Option<&BTreeSet<String>>,
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
        CodeQuery::Doc { pattern } => QueryMatcher::Doc { pattern },
        CodeQuery::Markers { marker_kind } => QueryMatcher::Markers { kind: *marker_kind },
        CodeQuery::Changed { .. } => QueryMatcher::ChangedFiles {
            files: changed_files,
        },
    };

    let mut matched = 0;
    let mut scanned = 0usize;
    let mut selected: Vec<&SymbolRecord> = Vec::with_capacity(limit);
    // Scan the full in-memory index: matching is substring work only, and
    // the authorization callback fires solely for pattern matches, so a
    // scan budget would hide symbols from later files for no cost saving.
    // `limit` caps the returned records, not the scan.
    for symbol in symbols {
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
            truncated: records.len() < matched,
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
    Doc { pattern: &'a str },
    Markers { kind: crate::MarkerQueryKind },
    ChangedFiles { files: Option<&'a BTreeSet<String>> },
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
            Self::Doc { pattern } => symbol
                .doc_comment
                .as_deref()
                .is_some_and(|doc_comment| doc_comment.contains(pattern)),
            Self::Markers { kind } => symbol.has_marker(*kind),
            Self::ChangedFiles { files } => {
                files.is_some_and(|files| files.contains(&symbol.provenance.file_path))
            }
        }
    }
}
