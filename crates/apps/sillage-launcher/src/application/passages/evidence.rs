use serde::Deserialize;

use super::{
    MAX_EXCERPT_BYTES, MAX_HASH_BYTES, MAX_PASSAGES, MAX_PATH_BYTES, MAX_PATH_RESULTS, Passage,
    PassageLocation, PassageSearchResult, PathResult, SearchCoverage, SearchMetadata, safe_status,
};

pub(super) fn parse_search(output: &[u8], query: &str) -> Option<PassageSearchResult> {
    let response: SearchEnvelope = serde_json::from_slice(output).ok()?;
    if response.kind != "search" || response.data.query != query {
        return None;
    }
    Some(parse_search_data(response.data))
}

fn parse_search_data(data: SearchData) -> PassageSearchResult {
    let paths = parse_paths(data.path_results);
    let metadata = SearchMetadata {
        coverage: data.coverage.map(|coverage| SearchCoverage {
            percent_covered: coverage.percent_covered,
            distinct_sources: coverage.distinct_sources,
            distinct_documents: coverage.distinct_documents,
        }),
        index_generation: data.index_generation,
        status: data.status.and_then(safe_status),
    };
    let passages = parse_passages(data.evidence);
    PassageSearchResult {
        passages,
        paths,
        metadata,
    }
}

fn parse_paths(results: Vec<SearchPathResult>) -> Vec<PathResult> {
    let mut seen_paths = Vec::new();
    results
        .into_iter()
        .filter_map(|result| {
            let path = result.path;
            if path.is_empty()
                || path.len() > MAX_PATH_BYTES
                || path.as_bytes().contains(&0)
                || !std::path::Path::new(&path).is_absolute()
                || seen_paths.contains(&path)
            {
                return None;
            }
            seen_paths.push(path.clone());
            Some(PathResult { path })
        })
        .take(MAX_PATH_RESULTS)
        .collect()
}

fn parse_passages(evidence: Vec<SearchEvidence>) -> Vec<Passage> {
    let mut seen = Vec::with_capacity(MAX_PASSAGES);
    evidence
        .into_iter()
        .filter_map(|evidence| {
            let preview = evidence.preview?;
            if preview.excerpt.is_empty() || preview.excerpt.len() > MAX_EXCERPT_BYTES {
                return None;
            }
            let location = match preview.location {
                PreviewLocation::File {
                    path,
                    start_line,
                    end_line,
                    content_hash,
                } if !path.is_empty()
                    && path.len() <= MAX_PATH_BYTES
                    && !content_hash.is_empty()
                    && content_hash.len() <= MAX_HASH_BYTES =>
                {
                    PassageLocation::File {
                        path,
                        start_line,
                        end_line,
                        content_hash,
                    }
                }
                PreviewLocation::DocxParagraph {
                    path,
                    start_paragraph,
                    end_paragraph,
                    content_hash,
                } if !path.is_empty()
                    && path.len() <= MAX_PATH_BYTES
                    && !content_hash.is_empty()
                    && content_hash.len() <= MAX_HASH_BYTES =>
                {
                    PassageLocation::DocxParagraph {
                        path,
                        start_paragraph,
                        end_paragraph,
                        content_hash,
                    }
                }
                PreviewLocation::Pdf {
                    snapshot_id,
                    page_start,
                    page_end,
                    path: None,
                } => PassageLocation::Pdf {
                    snapshot_id,
                    page_start,
                    page_end,
                },
                PreviewLocation::PdfRegion {
                    snapshot_id,
                    page,
                    x,
                    y,
                    width,
                    height,
                    path: None,
                } => PassageLocation::PdfRegion {
                    snapshot_id,
                    page,
                    x,
                    y,
                    width,
                    height,
                },
                _ => return None,
            };
            if seen.contains(&evidence.evidence_id) {
                return None;
            }
            seen.push(evidence.evidence_id);
            Some(Passage {
                evidence_id: evidence.evidence_id,
                artifact_version: evidence.artifact_version,
                excerpt: preview.excerpt,
                truncated: preview.truncated,
                location,
            })
        })
        .take(MAX_PASSAGES)
        .collect()
}

#[derive(Deserialize)]
struct SearchEnvelope {
    #[serde(rename = "type")]
    kind: String,
    data: SearchData,
}

#[derive(Deserialize)]
struct SearchData {
    query: String,
    #[serde(default)]
    path_results: Vec<SearchPathResult>,
    evidence: Vec<SearchEvidence>,
    #[serde(default)]
    coverage: Option<SearchCoverageResponse>,
    #[serde(default)]
    index_generation: Option<u64>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize)]
struct SearchPathResult {
    path: String,
}

#[derive(Deserialize)]
struct SearchCoverageResponse {
    percent_covered: u8,
    distinct_sources: usize,
    distinct_documents: usize,
}

#[derive(Deserialize)]
struct SearchEvidence {
    evidence_id: u64,
    artifact_version: u64,
    preview: Option<SearchPreview>,
}

#[derive(Deserialize)]
struct SearchPreview {
    excerpt: String,
    truncated: bool,
    location: PreviewLocation,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PreviewLocation {
    File {
        path: String,
        start_line: u32,
        end_line: u32,
        content_hash: String,
    },
    DocxParagraph {
        path: String,
        start_paragraph: u32,
        end_paragraph: u32,
        content_hash: String,
    },
    Pdf {
        snapshot_id: u64,
        page_start: u32,
        page_end: u32,
        #[serde(default)]
        path: Option<String>,
    },
    PdfRegion {
        snapshot_id: u64,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        #[serde(default)]
        path: Option<String>,
    },
    #[serde(other)]
    Unsupported,
}
