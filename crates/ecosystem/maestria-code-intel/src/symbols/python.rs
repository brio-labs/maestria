//! Python call-target resolution for relation candidates.
//!
//! Exact match of a callee hint against qualified names first, then the
//! short name when it is unambiguous; `self.`/`cls.` receivers resolve by
//! the remainder. Ambiguity yields no edge — determinism beats recall.

use super::resolution::unique_symbol;
use crate::SymbolRecord;
use std::collections::BTreeMap;

/// Python call resolution: exact match of the callee hint against qualified
/// names first, then the short name when it is unambiguous. `self.`/`cls.`
/// receivers resolve by the remainder. Ambiguity yields no edge —
/// determinism beats recall.
pub(super) fn resolve_call<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    by_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    hint: &str,
) -> Option<&'a SymbolRecord> {
    let hint = hint.trim();
    // `this.` is the TypeScript analogue of Python's `self.` receiver.
    let hint = match ["self.", "cls.", "this."]
        .iter()
        .find_map(|prefix| hint.strip_prefix(prefix))
    {
        Some(stripped) => stripped,
        None => hint,
    };
    if let Some(target) = exact_qualified_match(by_qualified_name, hint) {
        return Some(target);
    }
    let short = match hint.rsplit('.').next() {
        Some(short) => short,
        None => hint,
    };
    unique_symbol(by_name.get(short)?.as_slice())
}

fn exact_qualified_match<'a>(
    by_qualified_name: &'a BTreeMap<String, Vec<&'a SymbolRecord>>,
    qualified: &str,
) -> Option<&'a SymbolRecord> {
    by_qualified_name
        .get(qualified)
        .and_then(|matches| unique_symbol(matches))
}
