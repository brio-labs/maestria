//! Accepted-span coordinate conversion.
//!
//! The frozen corpus judgments record accepted evidence spans as character
//! offsets into the source files; retrieved candidates carry line-based
//! source spans. The scorer needs one coordinate system, so the accepted
//! spans are converted to line ranges against the verified source content
//! before scoring. Unconvertible spans (missing source, out-of-range
//! offsets) are dropped so they cannot fabricate overlap.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use maestria_retrieval::{
    LearnedSparseAcceptedSpan, LearnedSparseBenchmarkCorpus, LearnedSparseExpectedOutcome,
};

/// The one-based line containing the character at `offset` in `text`.
fn char_to_line(text: &str, offset: u32) -> u32 {
    let mut line = 1_u32;
    for (index, character) in text.char_indices() {
        if (index as u32) >= offset {
            return line;
        }
        if character == '\n' {
            line = line.saturating_add(1);
        }
    }
    line
}

/// Converts one accepted char-offset span into a line-range span.
fn to_line_span(text: &str, span: &LearnedSparseAcceptedSpan) -> LearnedSparseAcceptedSpan {
    let end = span.end.saturating_sub(1);
    let start_line = char_to_line(text, span.start);
    let end_line = char_to_line(text, end);
    LearnedSparseAcceptedSpan {
        source_id: span.source_id.clone(),
        start: start_line,
        end: end_line,
    }
}

/// Converts every case expectation whose accepted spans use character
/// offsets into line-range coordinates against the verified source files.
pub(super) fn convert_expected_spans(
    corpus: &LearnedSparseBenchmarkCorpus,
    source_ids: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, LearnedSparseExpectedOutcome>> {
    let source_path_by_id = source_ids
        .iter()
        .map(|(path, source_id)| (source_id.clone(), path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut expected_by_case = BTreeMap::new();
    for case in &corpus.cases {
        let Some(LearnedSparseExpectedOutcome::Evidence {
            accepted_spans,
            evidence_chain,
            minimum_source_diversity,
        }) = case.expected.as_ref()
        else {
            continue;
        };
        let mut converted = Vec::new();
        for span in accepted_spans {
            let Some(path) = source_path_by_id.get(&span.source_id) else {
                return Err(anyhow!(
                    "accepted span source {} has no indexed path",
                    span.source_id
                ));
            };
            let text = std::fs::read_to_string(Path::new(path))
                .map_err(|error| anyhow!("read source {} for span conversion: {error}", path))?;
            converted.push(to_line_span(&text, span));
        }
        if converted.is_empty() {
            return Err(anyhow!(
                "case {} accepted spans did not convert to line ranges",
                case.case_id
            ));
        }
        expected_by_case.insert(
            case.case_id.clone(),
            LearnedSparseExpectedOutcome::Evidence {
                accepted_spans: converted,
                evidence_chain: evidence_chain.clone(),
                minimum_source_diversity: *minimum_source_diversity,
            },
        );
    }
    Ok(expected_by_case)
}
