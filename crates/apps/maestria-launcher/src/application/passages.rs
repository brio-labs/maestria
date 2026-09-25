use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::time::timeout;

use crate::settings::SearchServiceConfig;

const MAX_PASSAGES: usize = 8;
const MAX_PATH_RESULTS: usize = 100;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const SEARCH_TIMEOUT: Duration = Duration::from_millis(750);
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PATH_BYTES: usize = 4096;
const MAX_HASH_BYTES: usize = 128;
const MAX_EXCERPT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub(super) struct Passage {
    pub(super) evidence_id: u64,
    pub(super) artifact_version: u64,
    pub(super) excerpt: String,
    pub(super) truncated: bool,
    pub(super) location: PassageLocation,
}

#[derive(Debug, Clone)]
pub(super) struct PathResult {
    pub(super) path: String,
}

#[derive(Debug, Clone)]
pub(super) enum PassageLocation {
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
    },
    PdfRegion {
        snapshot_id: u64,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

impl PassageLocation {
    pub(super) fn document_group(&self) -> String {
        match self {
            Self::File { path, .. } => path.clone(),
            Self::DocxParagraph { path, .. } => path.clone(),
            Self::Pdf { snapshot_id, .. } | Self::PdfRegion { snapshot_id, .. } => {
                format!("PDF snapshot {snapshot_id}")
            }
        }
    }

    pub(super) fn is_file(&self) -> bool {
        matches!(self, Self::File { .. })
    }

    pub(super) fn has_path(&self) -> bool {
        matches!(self, Self::File { .. } | Self::DocxParagraph { .. })
    }

    pub(super) fn citation(&self) -> String {
        match self {
            Self::File {
                path,
                start_line,
                end_line,
                ..
            } => format!("{}:{}", path, number_range(*start_line, *end_line)),
            Self::DocxParagraph {
                path,
                start_paragraph,
                end_paragraph,
                ..
            } => format!(
                "{} · paragraph {}",
                path,
                number_range(*start_paragraph, *end_paragraph)
            ),
            Self::Pdf {
                snapshot_id,
                page_start,
                page_end,
            } => format!(
                "PDF snapshot {snapshot_id} · page {}",
                number_range(*page_start, *page_end)
            ),
            Self::PdfRegion {
                snapshot_id,
                page,
                x,
                y,
                width,
                height,
            } => format!(
                "PDF snapshot {snapshot_id} · page {page} · region {x},{y} {width}×{height}"
            ),
        }
    }
}

fn number_range(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}–{end}")
    }
}

pub(super) struct PassageSearchResult {
    pub(super) passages: Vec<Passage>,
    pub(super) paths: Vec<PathResult>,
    pub(super) metadata: SearchMetadata,
}

#[derive(Debug, Default, Clone)]
pub(super) struct SearchMetadata {
    coverage: Option<SearchCoverage>,
    index_generation: Option<u64>,
    status: Option<String>,
}

#[derive(Debug, Clone)]
struct SearchCoverage {
    percent_covered: u8,
    distinct_sources: usize,
    distinct_documents: usize,
}

impl SearchMetadata {
    pub(super) fn summary(&self) -> String {
        let mut details = Vec::new();
        if let Some(status) = &self.status {
            details.push(format!("status {status}"));
        }
        if let Some(generation) = self.index_generation {
            details.push(format!("index generation {generation}"));
        }
        if let Some(coverage) = &self.coverage {
            details.push(format!(
                "{}% coverage · {} documents · {} sources",
                coverage.percent_covered, coverage.distinct_documents, coverage.distinct_sources
            ));
        }
        if details.is_empty() {
            "Coverage and index status unavailable".to_string()
        } else {
            details.join(" · ")
        }
    }
}

pub(super) async fn search(
    config: SearchServiceConfig,
    query: &str,
) -> Option<PassageSearchResult> {
    if query.is_empty() || query.len() > 4096 || query.as_bytes().contains(&0) {
        return None;
    }
    let mut command = base_command(&config, "interactive-search");
    command
        .arg("--limit")
        .arg(MAX_PASSAGES.to_string())
        .arg(query);
    let output = run_command(command, SEARCH_TIMEOUT).await?;
    let response: SearchEnvelope = serde_json::from_slice(&output).ok()?;
    if response.kind != "search" || response.data.query != query {
        return None;
    }
    let mut seen_paths = Vec::new();
    let paths = response
        .data
        .path_results
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
        .collect();

    let metadata = SearchMetadata {
        coverage: response.data.coverage.map(|coverage| SearchCoverage {
            percent_covered: coverage.percent_covered,
            distinct_sources: coverage.distinct_sources,
            distinct_documents: coverage.distinct_documents,
        }),
        index_generation: response.data.index_generation,
        status: response.data.status.and_then(safe_status),
    };
    let mut seen = Vec::with_capacity(MAX_PASSAGES);
    let passages = response
        .data
        .evidence
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
        .collect();
    Some(PassageSearchResult {
        passages,
        paths,
        metadata,
    })
}

fn safe_status(status: String) -> Option<String> {
    let status = status.trim();
    (!status.is_empty()
        && status.len() <= 64
        && status
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || " _-".contains(character)))
    .then(|| status.to_string())
}

pub(super) async fn reopen(
    config: SearchServiceConfig,
    passage: &Passage,
) -> Result<ReopenedPassage, ReopenError> {
    let mut command = base_command(&config, "open-evidence");
    command
        .arg("--evidence-id")
        .arg(passage.evidence_id.to_string());
    let output = run_command(command, OPEN_TIMEOUT)
        .await
        .ok_or(ReopenError::Unavailable)?;
    let response: EvidenceEnvelope =
        serde_json::from_slice(&output).map_err(|_| ReopenError::Unavailable)?;
    if response.kind != "evidence" || response.data.evidence_id != passage.evidence_id {
        return Err(ReopenError::Changed);
    }
    verify_opened_location(
        &passage.location,
        response.data.source,
        response.data.excerpt,
    )
}

pub(super) struct ReopenedPassage {
    pub(super) excerpt: String,
    pub(super) citation: String,
    pub(super) path: Option<std::path::PathBuf>,
    pub(super) pdf_page: Option<u32>,
    pub(super) fallback: String,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ReopenError {
    Unavailable,
    Changed,
}

fn verify_opened_location(
    preview: &PassageLocation,
    opened: EvidenceSource,
    excerpt: String,
) -> Result<ReopenedPassage, ReopenError> {
    match (preview, opened) {
        (
            PassageLocation::File {
                path: preview_path,
                start_line: preview_start,
                end_line: preview_end,
                content_hash: preview_hash,
            },
            EvidenceSource::File {
                path,
                start_line,
                end_line,
                content_hash,
            },
        ) if preview_path == &path
            && preview_start == &start_line
            && preview_end == &end_line
            && preview_hash == &content_hash =>
        {
            Ok(ReopenedPassage {
                excerpt,
                citation: format!("{}:{}", path, number_range(start_line, end_line)),
                path: Some(path.into()),
                pdf_page: None,
                fallback: format!(
                    "Default viewer may not jump to line {start_line}; passage shown here."
                ),
            })
        }
        (
            PassageLocation::DocxParagraph {
                path: preview_path,
                start_paragraph: preview_start,
                end_paragraph: preview_end,
                content_hash: preview_hash,
            },
            EvidenceSource::DocxParagraph {
                path,
                start_paragraph,
                end_paragraph,
                content_hash,
            },
        ) if preview_path == &path
            && preview_start == &start_paragraph
            && preview_end == &end_paragraph
            && preview_hash == &content_hash =>
        {
            Ok(ReopenedPassage {
                excerpt,
                citation: format!(
                    "{} · paragraph {}",
                    path,
                    number_range(start_paragraph, end_paragraph)
                ),
                path: Some(path.into()),
                pdf_page: None,
                fallback: format!(
                    "Default viewer may not jump to paragraph {start_paragraph}; passage shown here."
                ),
            })
        }
        (
            PassageLocation::Pdf {
                snapshot_id: preview_snapshot,
                page_start: preview_start,
                page_end: preview_end,
            },
            EvidenceSource::Pdf {
                snapshot_id,
                page_start,
                page_end,
                path,
            },
        ) if preview_snapshot == &snapshot_id
            && preview_start == &page_start
            && preview_end == &page_end =>
        {
            let citation_path = path.as_deref().map_or_else(
                || format!("PDF snapshot {snapshot_id}"),
                |path| path.to_owned(),
            );
            let path = path.map(std::path::PathBuf::from);
            Ok(ReopenedPassage {
                excerpt,
                citation: format!(
                    "{} · page {}",
                    citation_path,
                    number_range(page_start, page_end)
                ),
                path,
                pdf_page: Some(page_start),
                fallback: format!(
                    "Default PDF viewer may not jump to page {page_start}; passage shown here."
                ),
            })
        }
        (
            PassageLocation::PdfRegion {
                snapshot_id: preview_snapshot,
                page: preview_page,
                x: preview_x,
                y: preview_y,
                width: preview_width,
                height: preview_height,
            },
            EvidenceSource::PdfRegion {
                snapshot_id,
                page,
                x,
                y,
                width,
                height,
                path,
            },
        ) if preview_snapshot == &snapshot_id
            && preview_page == &page
            && preview_x == &x
            && preview_y == &y
            && preview_width == &width
            && preview_height == &height =>
        {
            let citation_path = path.as_deref().map_or_else(
                || format!("PDF snapshot {snapshot_id}"),
                |path| path.to_owned(),
            );
            let path = path.map(std::path::PathBuf::from);
            Ok(ReopenedPassage {
                excerpt,
                citation: format!(
                    "{} · page {page} · region {x},{y} {width}×{height}",
                    citation_path
                ),
                path,
                pdf_page: Some(page),
                fallback: format!(
                    "Default PDF viewer may not jump to page {page} or region; passage shown here."
                ),
            })
        }
        _ => Err(ReopenError::Changed),
    }
}

fn base_command(config: &SearchServiceConfig, subcommand: &str) -> Command {
    let mut command = Command::new("maestria-search");
    command
        .arg(subcommand)
        .arg("--socket-path")
        .arg(&config.socket_path)
        .arg("--consumer-realm")
        .arg(&config.consumer_realm)
        .arg("--credential-file")
        .arg(&config.credential_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

async fn run_command(mut command: Command, duration: Duration) -> Option<Vec<u8>> {
    let mut child: Child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let mut stdout = stdout.take((MAX_OUTPUT_BYTES + 1) as u64);
    let mut output = Vec::with_capacity(8192);
    let result = timeout(duration, async {
        if stdout.read_to_end(&mut output).await.is_err() || output.len() > MAX_OUTPUT_BYTES {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return None;
        }
        let status = child.wait().await.ok()?;
        status.success().then_some(output)
    })
    .await;
    match result {
        Ok(output) => output,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            None
        }
    }
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

#[derive(Deserialize)]
struct EvidenceEnvelope {
    #[serde(rename = "type")]
    kind: String,
    data: EvidenceResponse,
}

#[derive(Deserialize)]
struct EvidenceResponse {
    evidence_id: u64,
    source: EvidenceSource,
    excerpt: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EvidenceSource {
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
