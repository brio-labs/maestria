use super::super::super::server::ApiContext;
use anyhow::{Result, anyhow};

use maestria_domain::{DomainInput, IndexStatus, NotebookId};

use super::super::super::protocol::{
    NotebookListResponse, NotebookResponse, NotebookSourceCatalogEntry,
    NotebookSourceCatalogResponse, NotebookSummary,
};
use super::support::{manifest, notebook_response, state, submit};

const DEFAULT_SOURCE_LIMIT: usize = 50;
const MAX_SOURCE_LIMIT: usize = 100;

pub(crate) async fn list(context: &ApiContext) -> Result<NotebookListResponse> {
    let state = state(context).await?;
    Ok(NotebookListResponse {
        notebooks: state
            .notebooks
            .values()
            .map(|notebook| NotebookSummary {
                notebook_id: notebook.id.value(),
                title: notebook.title.to_string(),
                source_count: notebook.source_keys.len(),
                updated_at: notebook.updated_at.value(),
            })
            .collect(),
    })
}

pub(crate) async fn create(context: &ApiContext, title: String) -> Result<NotebookResponse> {
    let result = submit(
        context,
        DomainInput::CreateNotebook(maestria_domain::CreateNotebookInput { title }),
    )
    .await?;
    let notebook_id = result
        .events
        .iter()
        .find_map(|envelope| match &envelope.event {
            maestria_domain::DomainEvent::NotebookCreated { notebook_id, .. } => Some(*notebook_id),
            _ => None,
        })
        .ok_or_else(|| anyhow!("notebook creation result missing identity"))?;
    let state = state(context).await?;
    notebook_response(&state, notebook_id)
}

pub(crate) async fn get(context: &ApiContext, notebook_id: u64) -> Result<NotebookResponse> {
    let state = state(context).await?;
    notebook_response(&state, NotebookId::new(notebook_id))
}

pub(crate) async fn rename(
    context: &ApiContext,
    notebook_id: u64,
    title: String,
) -> Result<NotebookResponse> {
    submit(
        context,
        DomainInput::RenameNotebook(maestria_domain::RenameNotebookInput {
            notebook_id: NotebookId::new(notebook_id),
            title,
        }),
    )
    .await?;
    get(context, notebook_id).await
}

pub(crate) async fn delete(context: &ApiContext, notebook_id: u64) -> Result<()> {
    submit(
        context,
        DomainInput::DeleteNotebook(maestria_domain::DeleteNotebookInput {
            notebook_id: NotebookId::new(notebook_id),
        }),
    )
    .await?;
    Ok(())
}

pub(crate) async fn source_catalog(
    context: &ApiContext,
    query: Option<String>,
    offset: usize,
    limit: usize,
) -> Result<NotebookSourceCatalogResponse> {
    let limit = if limit == 0 {
        DEFAULT_SOURCE_LIMIT
    } else {
        limit
    };
    if !(1..=MAX_SOURCE_LIMIT).contains(&limit) {
        return Err(anyhow!(
            "source catalog limit must be between 1 and {MAX_SOURCE_LIMIT}"
        ));
    }
    let state = state(context).await?;
    let manifest = manifest(context)?;
    let query = match query {
        Some(query) => query.to_lowercase(),
        None => String::new(),
    };
    let mut entries: Vec<_> = state
        .active_sources
        .iter()
        .filter_map(|(key, artifact_id)| {
            let parser = state.pending_parsers.get(artifact_id)?;
            if !manifest.allows_source(std::path::Path::new(&parser.source_path)) {
                return None;
            }
            let artifact = state.artifacts.get(artifact_id)?;
            if !query.is_empty()
                && !artifact.title.to_lowercase().contains(&query)
                && !key.as_str().to_lowercase().contains(&query)
            {
                return None;
            }
            Some(NotebookSourceCatalogEntry {
                source_key: key.to_string(),
                artifact_id: Some(artifact_id.value()),
                title: Some(artifact.title.clone()),
                content_hash: artifact
                    .content_hash
                    .as_ref()
                    .map(|hash| hash.as_str().to_owned()),
                index_status: format!("{:?}", artifact.index_status),
                parse_status: artifact
                    .parse_status
                    .as_ref()
                    .map(|status| format!("{status:?}")),
                source_kind: "file".to_owned(),
                available: artifact.index_status == IndexStatus::Indexed,
            })
        })
        .collect();
    entries.sort_by_key(|entry| {
        let title = match entry.title.as_deref() {
            Some(title) => title.to_lowercase(),
            None => String::new(),
        };
        (title, entry.source_key.clone())
    });
    let sources = entries.into_iter().skip(offset).take(limit).collect();
    Ok(NotebookSourceCatalogResponse {
        sources,
        offset,
        limit,
    })
}

pub(crate) async fn attach(
    context: &ApiContext,
    notebook_id: u64,
    source_key: String,
) -> Result<NotebookResponse> {
    submit(
        context,
        DomainInput::AttachNotebookSource(maestria_domain::AttachNotebookSourceInput {
            notebook_id: NotebookId::new(notebook_id),
            source_key,
        }),
    )
    .await?;
    get(context, notebook_id).await
}

pub(crate) async fn detach(
    context: &ApiContext,
    notebook_id: u64,
    source_key: String,
) -> Result<NotebookResponse> {
    submit(
        context,
        DomainInput::DetachNotebookSource(maestria_domain::DetachNotebookSourceInput {
            notebook_id: NotebookId::new(notebook_id),
            source_key,
        }),
    )
    .await?;
    get(context, notebook_id).await
}
