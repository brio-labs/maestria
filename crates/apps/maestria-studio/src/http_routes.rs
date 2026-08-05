use anyhow::{Context, Result, anyhow};
use maestria_daemon::api::{ClientOperation, ClientResponse};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::Value;

#[path = "http_path.rs"]
mod path_encoding;
use self::path_encoding::decode_path_segment;
#[derive(RustEmbed)]
#[folder = "../../../web/dist/"]
struct Assets;

use super::{BootstrapResponse, StudioState, json_response};

fn asset_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("woff" | "woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

pub(super) async fn route(
    state: &StudioState,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(u16, &'static str, Vec<u8>)> {
    if method == "GET" {
        let requested = match path.strip_prefix('/') {
            Some(value) => value,
            None => path,
        };
        let key = if path == "/" { "index.html" } else { requested };
        let (resolved_key, asset) = match Assets::get(key) {
            Some(asset) => (key, Some(asset)),
            None if !path.starts_with("/api/") => ("index.html", Assets::get("index.html")),
            None => (key, None),
        };
        if let Some(asset) = asset {
            return Ok((
                200,
                asset_content_type(resolved_key),
                asset.data.into_owned(),
            ));
        }
    }
    if method == "GET" && path == "/api/bootstrap" {
        return route_bootstrap(state).await;
    }
    if path == "/api/notebooks" {
        return route_notebooks(state, method, body).await;
    }
    if path.starts_with("/api/notebooks/") {
        return route_notebook_path(state, method, path, body).await;
    }
    Err(anyhow!("route not found"))
}

async fn route_bootstrap(state: &StudioState) -> Result<(u16, &'static str, Vec<u8>)> {
    let status_response = state
        .client
        .request(ClientOperation::Status)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let status = sanitized_status(&status_response)?;
    let notebooks = state
        .client
        .request(ClientOperation::NotebookList)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let response = serde_json::to_vec(&BootstrapResponse {
        status,
        notebooks,
        agents: vec![state.agent.profile()],
    })
    .context("encode Studio bootstrap")?;
    Ok((200, "application/json", response))
}

fn sanitized_status(response: &ClientResponse) -> Result<Value> {
    let ClientResponse::Status(status) = response else {
        return Err(anyhow!("daemon returned an invalid status response"));
    };
    let instance_root = std::path::Path::new(&status.instance_root)
        .file_name()
        .and_then(|name| name.to_str())
        .map_or("instance", |name| name);
    let mut object = serde_json::Map::new();
    object.insert(
        "instance_root".to_owned(),
        Value::String(instance_root.to_owned()),
    );
    object.insert("event_count".to_owned(), Value::from(status.event_count));
    object.insert("task_count".to_owned(), Value::from(status.task_count));
    Ok(Value::Object(object))
}

async fn route_notebooks(
    state: &StudioState,
    method: &str,
    body: &[u8],
) -> Result<(u16, &'static str, Vec<u8>)> {
    match method {
        "GET" => {
            let response = state
                .client
                .request(ClientOperation::NotebookList)
                .await
                .map_err(|error| anyhow!(error.to_string()))?;
            json_response(200, &response)
        }
        "POST" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Create {
                title: String,
            }
            let input: Create = serde_json::from_slice(body).context("decode notebook create")?;
            let response = state
                .client
                .request(ClientOperation::NotebookCreate { title: input.title })
                .await
                .map_err(|error| anyhow!(error.to_string()))?;
            json_response(201, &response)
        }
        _ => Err(anyhow!("method not allowed")),
    }
}

async fn route_notebook_path(
    state: &StudioState,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(u16, &'static str, Vec<u8>)> {
    if let Some(notebook_id) = path
        .strip_prefix("/api/notebooks/")
        .and_then(|value| value.strip_suffix("/ask"))
        .and_then(|value| value.parse::<u64>().ok())
    {
        return super::http_ask::route_notebook_ask(state, method, body, notebook_id).await;
    }
    let (rest, _) = match path
        .strip_prefix("/api/notebooks/")
        .and_then(|value| value.split_once('?'))
    {
        Some(parts) => parts,
        None => (
            path.strip_prefix("/api/notebooks/")
                .ok_or_else(|| anyhow!("invalid notebook path"))?,
            "",
        ),
    };
    let mut parts = rest.split('/');
    let notebook_id = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("invalid notebook id"))?;
    let suffix = parts.collect::<Vec<_>>();
    route_notebook_suffix(state, method, body, notebook_id, &suffix).await
}

async fn route_notebook_suffix(
    state: &StudioState,
    method: &str,
    body: &[u8],
    notebook_id: u64,
    suffix: &[&str],
) -> Result<(u16, &'static str, Vec<u8>)> {
    if suffix.is_empty() {
        return match method {
            "GET" => {
                let response = state
                    .client
                    .request(ClientOperation::NotebookGet { notebook_id })
                    .await
                    .map_err(|error| anyhow!(error.to_string()))?;
                json_response(200, &response)
            }
            "PATCH" => route_notebook_rename(state, body, notebook_id).await,
            "DELETE" => {
                let response = state
                    .client
                    .request(ClientOperation::NotebookDelete { notebook_id })
                    .await
                    .map_err(|error| anyhow!(error.to_string()))?;
                json_response(200, &response)
            }
            _ => Err(anyhow!("method not allowed")),
        };
    }
    if suffix == ["rename"] && method == "POST" {
        return route_notebook_rename(state, body, notebook_id).await;
    }
    if suffix == ["drafts"] {
        return route_notebook_drafts(state, method, body, notebook_id).await;
    }
    if suffix.len() == 2 && suffix[0] == "drafts" {
        let draft_id = suffix[1].parse::<u64>().context("invalid draft id")?;
        return route_notebook_draft(state, method, body, notebook_id, draft_id).await;
    }
    if suffix[0] == "sources" {
        return route_notebook_sources(state, method, notebook_id, suffix).await;
    }
    if suffix.len() == 2 && suffix[0] == "evidence" && method == "GET" {
        let evidence_id = suffix[1].parse::<u64>().context("invalid evidence id")?;
        let response = state
            .client
            .request(ClientOperation::NotebookEvidence {
                notebook_id,
                evidence_id,
            })
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        return json_response(200, &response);
    }
    Err(anyhow!("route not found"))
}

async fn route_notebook_rename(
    state: &StudioState,
    body: &[u8],
    notebook_id: u64,
) -> Result<(u16, &'static str, Vec<u8>)> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Rename {
        title: String,
    }
    let input: Rename = serde_json::from_slice(body).context("decode notebook rename")?;
    let response = state
        .client
        .request(ClientOperation::NotebookRename {
            notebook_id,
            title: input.title,
        })
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    json_response(200, &response)
}

async fn route_notebook_drafts(
    state: &StudioState,
    method: &str,
    body: &[u8],
    notebook_id: u64,
) -> Result<(u16, &'static str, Vec<u8>)> {
    match method {
        "GET" => {
            let response = state
                .client
                .request(ClientOperation::NotebookDraftList { notebook_id })
                .await
                .map_err(|error| anyhow!(error.to_string()))?;
            json_response(200, &response)
        }
        "POST" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Save {
                draft_id: Option<u64>,
                expected_revision: Option<u64>,
                title: String,
                markdown: String,
                evidence_ids: Vec<u64>,
            }
            let input: Save = serde_json::from_slice(body).context("decode draft save")?;
            let response = state
                .client
                .request(ClientOperation::NotebookDraftSave {
                    notebook_id,
                    draft_id: input.draft_id,
                    expected_revision: input.expected_revision,
                    title: input.title,
                    markdown: input.markdown,
                    evidence_ids: input.evidence_ids,
                })
                .await
                .map_err(|error| anyhow!(error.to_string()))?;
            json_response(200, &response)
        }
        _ => Err(anyhow!("method not allowed")),
    }
}

async fn route_notebook_draft(
    state: &StudioState,
    method: &str,
    body: &[u8],
    notebook_id: u64,
    draft_id: u64,
) -> Result<(u16, &'static str, Vec<u8>)> {
    let response = match method {
        "GET" => {
            state
                .client
                .request(ClientOperation::NotebookDraftGet {
                    notebook_id,
                    draft_id,
                })
                .await
        }
        "PATCH" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Save {
                expected_revision: u64,
                title: String,
                markdown: String,
                evidence_ids: Vec<u64>,
            }
            let input: Save = serde_json::from_slice(body).context("decode draft update")?;
            state
                .client
                .request(ClientOperation::NotebookDraftSave {
                    notebook_id,
                    draft_id: Some(draft_id),
                    expected_revision: Some(input.expected_revision),
                    title: input.title,
                    markdown: input.markdown,
                    evidence_ids: input.evidence_ids,
                })
                .await
        }
        "DELETE" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Delete {
                expected_revision: u64,
            }
            let input: Delete = serde_json::from_slice(body).context("decode draft delete")?;
            state
                .client
                .request(ClientOperation::NotebookDraftDelete {
                    notebook_id,
                    draft_id,
                    expected_revision: input.expected_revision,
                })
                .await
        }
        _ => return Err(anyhow!("method not allowed")),
    }
    .map_err(|error| anyhow!(error.to_string()))?;
    json_response(200, &response)
}

async fn route_notebook_sources(
    state: &StudioState,
    method: &str,
    notebook_id: u64,
    suffix: &[&str],
) -> Result<(u16, &'static str, Vec<u8>)> {
    if suffix.len() == 1 && method == "GET" {
        let response = state
            .client
            .request(ClientOperation::NotebookSourceCatalog {
                query: None,
                offset: 0,
                limit: 100,
            })
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        return json_response(200, &response);
    }
    if suffix.len() != 2 {
        return Err(anyhow!("route not found"));
    }
    let source_key = decode_path_segment(suffix[1])?;
    let operation = match method {
        "POST" => ClientOperation::NotebookSourceAttach {
            notebook_id,
            source_key,
        },
        "DELETE" => ClientOperation::NotebookSourceDetach {
            notebook_id,
            source_key,
        },
        _ => return Err(anyhow!("method not allowed")),
    };
    let response = state
        .client
        .request(operation)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    json_response(200, &response)
}
