//! Cross-file symbol references over persisted relations.
//!
//! A `CodeQuery::References` query resolves a seed (symbols whose name or
//! qualified name contains the pattern, authorized like any symbol match)
//! and walks the persisted relation set once: inbound keeps edges whose
//! target is a seed and reports the source usage sites, outbound keeps
//! edges whose source is a seed and reports the target symbols. Records are
//! deduplicated per symbol and ordered with the same `symbol_order` as the
//! symbol scan; `limit` caps records and `relations` carries the evidence
//! edges of the returned records only.

use crate::query::symbol_order;
use crate::{
    CodeIntelError, CodeQuery, CodeRelationRecord, QueryResult, QuerySummary, ReferencesDirection,
    RepositoryCodeIndex, SymbolRecord,
};
use std::collections::{BTreeMap, BTreeSet};

/// Failure while parsing a [`ReferencesDirection`] from user input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencesDirectionParseError {
    input: String,
}

impl std::fmt::Display for ReferencesDirectionParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown references direction {:?}; expected inbound or outbound",
            self.input
        )
    }
}

impl std::error::Error for ReferencesDirectionParseError {}

impl std::str::FromStr for ReferencesDirection {
    type Err = ReferencesDirectionParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "inbound" => Ok(Self::Inbound),
            "outbound" => Ok(Self::Outbound),
            _ => Err(ReferencesDirectionParseError {
                input: value.to_string(),
            }),
        }
    }
}

impl RepositoryCodeIndex {
    /// Resolve cross-file symbol references for a `CodeQuery::References`
    /// query through the caller's authorization policy. Any other query
    /// kind is a typed error: relations exist only inside this index, so
    /// the symbol scan never executes them.
    pub fn references<E, F>(
        &self,
        query: CodeQuery,
        limit: usize,
        mut authorize: F,
    ) -> Result<QueryResult, E>
    where
        F: FnMut(&SymbolRecord) -> Result<bool, E>,
        E: From<CodeIntelError>,
    {
        let (pattern, direction) = match query {
            CodeQuery::References { pattern, direction } => (pattern, direction),
            _ => {
                return Err(CodeIntelError::UnsupportedQuery {
                    details: "references() accepts CodeQuery::References only".to_string(),
                }
                .into());
            }
        };

        // Seed scan: substring match on name or qualified name, with the
        // authorization callback firing per seed exactly like the symbol
        // query. Unauthorized seeds are invisible, never errors.
        let mut seed_ids: BTreeSet<&str> = BTreeSet::new();
        let mut scanned = 0usize;
        for symbol in &self.symbols {
            scanned = scanned.saturating_add(1);
            if !(symbol.name.contains(&pattern) || symbol.qualified_name.contains(&pattern)) {
                continue;
            }
            if !authorize(symbol)? {
                continue;
            }
            seed_ids.insert(symbol.record_id.as_str());
        }

        let symbol_by_id = self
            .symbols
            .iter()
            .map(|symbol| (symbol.record_id.as_str(), symbol))
            .collect::<BTreeMap<_, _>>();

        // Single pass over the relation set. The non-seed endpoint is the
        // usage site and must itself pass authorization; a symbol with
        // several relations to the seed contributes one record and one
        // relation per edge.
        let mut matched_records: BTreeMap<&str, &SymbolRecord> = BTreeMap::new();
        let mut matched_relations: Vec<(&str, &CodeRelationRecord)> = Vec::new();
        for relation in &self.relations {
            let (seed_id, usage_id) = match direction {
                ReferencesDirection::Inbound => (
                    relation.target_record_id.as_str(),
                    relation.source_record_id.as_str(),
                ),
                ReferencesDirection::Outbound => (
                    relation.source_record_id.as_str(),
                    relation.target_record_id.as_str(),
                ),
            };
            if !seed_ids.contains(seed_id) {
                continue;
            }
            let usage = match symbol_by_id.get(usage_id) {
                Some(symbol) => *symbol,
                None => {
                    return Err(CodeIntelError::Integrity {
                        context: "reference relation endpoint".to_string(),
                        details: usage_id.to_string(),
                    }
                    .into());
                }
            };
            if !authorize(usage)? {
                continue;
            }
            matched_records.insert(usage_id, usage);
            matched_relations.push((usage_id, relation));
        }

        let mut records: Vec<&SymbolRecord> = matched_records.into_values().collect();
        records.sort_by(|left, right| symbol_order(left, right));
        let matched = records.len();
        records.truncate(limit);
        let returned_ids: BTreeSet<&str> = records
            .iter()
            .map(|record| record.record_id.as_str())
            .collect();
        let relations: Vec<CodeRelationRecord> = matched_relations
            .into_iter()
            .filter(|(usage_id, _)| returned_ids.contains(usage_id))
            .map(|(_, relation)| relation.clone())
            .collect();
        let records: Vec<SymbolRecord> = records.into_iter().cloned().collect();

        Ok(QueryResult {
            summary: QuerySummary {
                query: CodeQuery::References { pattern, direction },
                matched,
                returned: records.len(),
                truncated: records.len() < matched,
                scanned,
                limit,
                regex_error: None,
            },
            records,
            relations,
        })
    }
}
