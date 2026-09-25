use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use maestria_extensions::{FileSearchResult, MAX_CAPABILITY_RESPONSE_BYTES};
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::time::timeout;

use super::{SearchConsumerConfig, text::truncate_utf16};

const MAX_SEARCH_OUTPUT_BYTES: usize = 1_048_576;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = MAX_CAPABILITY_RESPONSE_BYTES - 1024;

#[derive(Debug, Clone, Copy)]
pub(super) enum SearchError {
    Unavailable,
    InvalidRequest,
    Failed,
}

pub(super) async fn file_search(
    config: &SearchConsumerConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<FileSearchResult>, SearchError> {
    if query.contains('\0') {
        return Err(SearchError::InvalidRequest);
    }
    let output = run_search(config, query, limit).await?;
    let response: SearchEnvelope =
        serde_json::from_slice(&output).map_err(|_| SearchError::Failed)?;
    if response.kind != "search" {
        return Err(SearchError::Failed);
    }
    let mut results = response
        .data
        .evidence
        .into_iter()
        .filter_map(search_result)
        .take(limit)
        .collect::<Vec<_>>();
    fit_response(&mut results)?;
    Ok(results)
}

async fn run_search(
    config: &SearchConsumerConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<u8>, SearchError> {
    let mut child = search_command(config, query, limit)
        .spawn()
        .map_err(|_| SearchError::Unavailable)?;
    let Some(stdout) = child.stdout.take() else {
        return stop_child(&mut child)
            .await
            .and(Err(SearchError::Unavailable));
    };
    match timeout(SEARCH_TIMEOUT, collect_output(&mut child, stdout)).await {
        Ok(result) => result,
        Err(_) => stop_child(&mut child)
            .await
            .and(Err(SearchError::Unavailable)),
    }
}

fn search_command(config: &SearchConsumerConfig, query: &str, limit: usize) -> Command {
    let mut command = Command::new(&config.executable);
    command
        .arg("interactive-search")
        .arg("--socket-path")
        .arg(&config.socket_path)
        .arg("--consumer-realm")
        .arg(&config.consumer_realm)
        .arg("--credential-file")
        .arg(&config.credential_file)
        .arg("--limit")
        .arg(limit.to_string())
        .arg("--")
        .arg(query)
        .current_dir(Path::new("/"))
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

async fn collect_output(
    child: &mut Child,
    stdout: tokio::process::ChildStdout,
) -> Result<Vec<u8>, SearchError> {
    let mut limited_stdout = stdout.take((MAX_SEARCH_OUTPUT_BYTES + 1) as u64);
    let mut output = Vec::with_capacity(16 * 1024);
    if limited_stdout.read_to_end(&mut output).await.is_err()
        || output.len() > MAX_SEARCH_OUTPUT_BYTES
    {
        return stop_child(child).await.and(Err(SearchError::Failed));
    }
    let status = child.wait().await.map_err(|_| SearchError::Unavailable)?;
    if status.success() {
        Ok(output)
    } else {
        Err(SearchError::Unavailable)
    }
}

async fn stop_child(child: &mut Child) -> Result<(), SearchError> {
    let kill = child.start_kill();
    match timeout(Duration::from_secs(1), child.wait()).await {
        Ok(Ok(_)) => kill.map_err(|_| SearchError::Unavailable),
        _ => Err(SearchError::Unavailable),
    }
}

fn search_result(evidence: SearchEvidence) -> Option<FileSearchResult> {
    let preview = evidence.preview?;
    let title = match &preview.location {
        SearchLocation::File { path } | SearchLocation::DocxParagraph { path } => {
            document_title(path)?
        }
        SearchLocation::Pdf { snapshot_id } | SearchLocation::PdfRegion { snapshot_id } => {
            format!("PDF snapshot {snapshot_id}")
        }
        SearchLocation::Other => return None,
    };
    let snippet = if preview.excerpt.is_empty() {
        None
    } else {
        Some(truncate_utf16(&preview.excerpt, 4096))
    };
    Some(FileSearchResult {
        file_id: format!("evidence-{}", evidence.evidence_id),
        title,
        snippet,
    })
}

fn document_title(path: &str) -> Option<String> {
    let name = Path::new(path).file_name()?.to_str()?;
    let sanitized = name
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    if sanitized.trim().is_empty() {
        None
    } else {
        Some(truncate_utf16(&sanitized, 120))
    }
}

fn fit_response(results: &mut Vec<FileSearchResult>) -> Result<(), SearchError> {
    loop {
        let response = serde_json::to_vec(results).map_err(|_| SearchError::Failed)?;
        if response.len() <= MAX_RESPONSE_BYTES {
            return Ok(());
        }
        if results.pop().is_none() {
            return Err(SearchError::Failed);
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
    #[serde(default)]
    evidence: Vec<SearchEvidence>,
}

#[derive(Deserialize)]
struct SearchEvidence {
    evidence_id: u64,
    #[serde(default)]
    preview: Option<SearchPreview>,
}

#[derive(Deserialize)]
struct SearchPreview {
    excerpt: String,
    location: SearchLocation,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SearchLocation {
    File {
        path: String,
    },
    DocxParagraph {
        path: String,
    },
    Pdf {
        snapshot_id: u64,
    },
    PdfRegion {
        snapshot_id: u64,
    },
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::{SearchEnvelope, search_result};

    #[test]
    fn search_only_grant_returns_cited_previews_without_unpreviewed_metadata()
    -> Result<(), serde_json::Error> {
        let response: SearchEnvelope = serde_json::from_str(
            r#"{"type":"search","data":{"evidence":[{"evidence_id":7,"source":"/tmp/approved/greeting.txt:1-1","preview":{"excerpt":"Greeting evidence: Hello from Sillage!","location":{"type":"file","path":"/tmp/approved/greeting.txt","start_line":1,"end_line":1,"content_hash":"sha256:verified"}}},{"evidence_id":8,"source":"/tmp/approved/stale.txt:1-1"},{"evidence_id":9,"preview":{"excerpt":"PDF cited text","location":{"type":"pdf","snapshot_id":42,"page_start":1,"page_end":1,"path":null}}}]}}"#,
        )?;
        let results = response
            .data
            .evidence
            .into_iter()
            .filter_map(search_result)
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].file_id, "evidence-7");
        assert_eq!(results[0].title, "greeting.txt");
        assert_eq!(
            results[0].snippet.as_deref(),
            Some("Greeting evidence: Hello from Sillage!")
        );
        assert_eq!(results[1].file_id, "evidence-9");
        assert_eq!(results[1].title, "PDF snapshot 42");
        assert_eq!(results[1].snippet.as_deref(), Some("PDF cited text"));
        Ok(())
    }
}
