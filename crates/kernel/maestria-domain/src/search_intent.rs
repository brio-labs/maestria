use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SearchIntent {
    ExactLookup,
    FactualLocal,
    SemanticDiscovery,
    CompositionalConstraints,
    MultiHop,
    CorpusSynthesis,
    RepositoryCode,
    VisualDocument,
    TemporalMemory,
    CurrentWeb,
    ContradictionAudit,
}

impl SearchIntent {
    /// Classifies a query using deterministic lexical signals only.
    pub fn classify(query: &str) -> Self {
        let query = query.trim().to_ascii_lowercase();
        let has = |terms: &[&str]| {
            terms.iter().any(|term| {
                let is_token = term
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_');
                if is_token {
                    query
                        .split(|character: char| {
                            !character.is_ascii_alphanumeric() && character != '_'
                        })
                        .any(|token| token == *term || token.starts_with(term))
                } else {
                    query.contains(term)
                }
            })
        };
        if query.is_empty()
            || (query.starts_with('"') && query.ends_with('"'))
            || has(&["id:", "::", ".rs", "cargo.toml", "path:"])
            || (query.split_whitespace().count() == 1
                && query.contains('-')
                && query
                    .chars()
                    .any(|character| character.is_ascii_alphanumeric()))
        {
            Self::ExactLookup
        } else if has(&["contradict", "conflict", "disagree", "counterevidence"]) {
            Self::ContradictionAudit
        } else if has(&["latest", "today", "current", "web", "news", "http"]) {
            Self::CurrentWeb
        } else if has(&[
            "table", "chart", "figure", "image", "visual", "pdf", "formula", "equation", "diagram",
            "scan", "ocr",
        ]) {
            Self::VisualDocument
        } else if has(&[
            "rust",
            "cargo",
            "function",
            "struct",
            "trait",
            "module",
            "repository",
        ]) {
            Self::RepositoryCode
        } else if has(&["when", "before", "after", "history", "previous", "last"]) {
            Self::TemporalMemory
        } else if has(&["how does", "relationship", "connected", "multi-hop"]) {
            Self::MultiHop
        } else if has(&[
            "summarize",
            "summary",
            "overview",
            "across",
            "compare",
            "synthesis",
        ]) {
            Self::CorpusSynthesis
        } else if has(&["must", "without", "requires", "constraint"])
            || (query.contains(" and ") && query.contains(" where "))
        {
            Self::CompositionalConstraints
        } else if has(&["similar", "related", "discover", "explore"]) {
            Self::SemanticDiscovery
        } else {
            Self::FactualLocal
        }
    }
}
