use anyhow::{Result, anyhow};
use maestria_domain::{
    DomainInput, FrozenNotebookCitation, IndexStatus, NotebookDraftRevision, NotebookId,
};

use super::super::super::protocol::{
    FrozenNotebookCitationResponse, NotebookDraftDeletedResponse, NotebookDraftListResponse,
    NotebookDraftResponse, NotebookDraftSavedResponse, NotebookDraftSummary, NotebookResponse,
    NotebookSourceSelection,
};
use super::super::super::server::ApiContext;

pub(crate) async fn draft_list(
    context: &ApiContext,
    notebook_id: u64,
) -> Result<NotebookDraftListResponse> {
    let state = state(context).await?;
    if !state.notebooks.contains_key(&NotebookId::new(notebook_id)) {
        return Err(anyhow!("notebook not found"));
    }
    Ok(NotebookDraftListResponse {
        drafts: state
            .notebook_drafts
            .values()
            .filter(|draft| draft.notebook_id.value() == notebook_id)
            .map(|draft| NotebookDraftSummary {
                draft_id: draft.id.value(),
                title: draft.title.to_string(),
                revision: draft.revision.value(),
                citation_count: draft.citations.len(),
                created_at: draft.created_at.value(),
                updated_at: draft.updated_at.value(),
            })
            .collect(),
    })
}

pub(crate) async fn draft_get(
    context: &ApiContext,
    notebook_id: u64,
    draft_id: u64,
) -> Result<NotebookDraftResponse> {
    let state = state(context).await?;
    let draft = state
        .notebook_drafts
        .get(&maestria_domain::NotebookDraftId::new(draft_id))
        .ok_or_else(|| anyhow!("draft not found"))?;
    if draft.notebook_id.value() != notebook_id {
        return Err(anyhow!("draft not found"));
    }
    let markdown = crate::notebook_draft_open::open(context, draft)?;
    Ok(NotebookDraftResponse {
        draft_id,
        notebook_id,
        title: draft.title.to_string(),
        markdown,
        body_hash: draft.body_hash.as_str().to_owned(),
        revision: draft.revision.value(),
        citations: draft.citations.iter().map(frozen_citation).collect(),
        created_at: draft.created_at.value(),
        updated_at: draft.updated_at.value(),
    })
}

pub(crate) async fn draft_save(
    context: &ApiContext,
    notebook_id: u64,
    draft_id: Option<u64>,
    expected_revision: Option<u64>,
    title: String,
    markdown: String,
    evidence_ids: Vec<u64>,
) -> Result<NotebookDraftSavedResponse> {
    if evidence_ids.len() > 12 {
        return Err(anyhow!("at most 12 citations are supported"));
    }
    let snapshot = state(context).await?;
    let notebook = snapshot
        .notebooks
        .get(&NotebookId::new(notebook_id))
        .ok_or_else(|| anyhow!("notebook not found"))?;
    let mut citations = Vec::new();
    for evidence_id in unique_ids(&snapshot, &evidence_ids)? {
        let output =
            crate::evidence_open::open_evidence_scoped(&context.layout, evidence_id.value())?;
        if !notebook
            .source_keys
            .iter()
            .any(|key| snapshot.active_sources.get(key) == Some(&output.artifact.id))
            || !source_allowed(context, &snapshot, output.artifact.id)?
        {
            return Err(anyhow!("source_not_selected"));
        }
        let artifact_content_hash = output
            .artifact
            .content_hash
            .clone()
            .ok_or_else(|| anyhow!("evidence artifact has no content hash"))?;
        citations.push(FrozenNotebookCitation {
            evidence_id: output.evidence.id,
            artifact_id: output.artifact.id,
            artifact_title: output.artifact.title,
            artifact_content_hash,
            source: output.evidence.kind,
            excerpt: output.evidence.excerpt,
            observed_at: output.evidence.observed_at,
        });
    }
    let result = submit_durable(
        context,
        DomainInput::SaveNotebookDraftRequested(maestria_domain::SaveNotebookDraftRequested {
            notebook_id: NotebookId::new(notebook_id),
            draft_id: draft_id.map(maestria_domain::NotebookDraftId::new),
            expected_revision: expected_revision
                .map(NotebookDraftRevision::try_from)
                .transpose()?,
            title,
            body: markdown,
            citations,
        }),
    )
    .await?;
    result
        .events
        .iter()
        .find_map(|envelope| match &envelope.event {
            maestria_domain::DomainEvent::NotebookDraftSaved {
                draft_id, revision, ..
            } => Some(NotebookDraftSavedResponse {
                draft_id: draft_id.value(),
                revision: revision.value(),
            }),
            _ => None,
        })
        .ok_or_else(|| anyhow!("draft save result missing identity"))
}

pub(crate) async fn draft_delete(
    context: &ApiContext,
    notebook_id: u64,
    draft_id: u64,
    expected_revision: u64,
) -> Result<NotebookDraftDeletedResponse> {
    submit(
        context,
        DomainInput::DeleteNotebookDraft(maestria_domain::DeleteNotebookDraftInput {
            notebook_id: NotebookId::new(notebook_id),
            draft_id: maestria_domain::NotebookDraftId::new(draft_id),
            expected_revision: NotebookDraftRevision::try_from(expected_revision)?,
        }),
    )
    .await?;
    Ok(NotebookDraftDeletedResponse {
        notebook_id,
        draft_id,
        revision: expected_revision,
    })
}

pub(super) fn unique_ids(
    state: &maestria_domain::KernelState,
    ids: &[u64],
) -> Result<Vec<maestria_domain::EvidenceId>> {
    let mut values: Vec<_> = ids
        .iter()
        .copied()
        .map(maestria_domain::EvidenceId::new)
        .collect();
    values.sort();
    values.dedup();
    if let Some(unknown) = values.iter().find(|id| !state.evidences.contains_key(id)) {
        return Err(anyhow!("evidence not found: {}", unknown.value()));
    }
    Ok(values)
}
pub(super) fn frozen_citation(citation: &FrozenNotebookCitation) -> FrozenNotebookCitationResponse {
    FrozenNotebookCitationResponse {
        evidence_id: citation.evidence_id.value(),
        artifact_id: citation.artifact_id.value(),
        artifact_title: citation.artifact_title.clone(),
        artifact_content_hash: citation.artifact_content_hash.as_str().to_owned(),
        source: format!("{:?}", citation.source),
        excerpt: citation.excerpt.clone(),
        observed_at: citation.observed_at.value(),
    }
}

pub(super) fn notebook_response(
    state: &maestria_domain::KernelState,
    id: NotebookId,
) -> Result<NotebookResponse> {
    let notebook = state
        .notebooks
        .get(&id)
        .ok_or_else(|| anyhow!("notebook not found"))?;
    Ok(NotebookResponse {
        notebook_id: id.value(),
        title: notebook.title.to_string(),
        sources: notebook
            .source_keys
            .iter()
            .map(|key| NotebookSourceSelection {
                source_key: key.to_string(),
                artifact_id: state.active_sources.get(key).map(|id| id.value()),
                available: state.active_sources.get(key).is_some_and(|id| {
                    state
                        .artifacts
                        .get(id)
                        .is_some_and(|a| a.index_status == IndexStatus::Indexed)
                }),
            })
            .collect(),
        created_at: notebook.created_at.value(),
        updated_at: notebook.updated_at.value(),
    })
}

pub(crate) async fn state(context: &ApiContext) -> Result<maestria_domain::KernelState> {
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| anyhow!("runtime unavailable"))?;
    Ok(runtime.kernel_state().await)
}
pub(super) fn manifest(context: &ApiContext) -> Result<maestria_core::InstanceManifest> {
    let contents = std::fs::read_to_string(&context.layout.manifest_path)
        .map_err(|error| anyhow!("read instance manifest: {error}"))?;
    maestria_core::InstanceManifest::decode(&contents)
        .map_err(|error| anyhow!("decode instance manifest: {error}"))
}

pub(super) fn source_allowed(
    context: &ApiContext,
    state: &maestria_domain::KernelState,
    artifact_id: maestria_domain::ArtifactId,
) -> Result<bool> {
    let Some(parser) = state.pending_parsers.get(&artifact_id) else {
        return Ok(false);
    };
    Ok(manifest(context)?.allows_source(std::path::Path::new(&parser.source_path)))
}

pub(crate) async fn submit(
    context: &ApiContext,
    input: DomainInput,
) -> Result<maestria_runtime::DomainApplicationResult> {
    context
        .runtime
        .as_ref()
        .ok_or_else(|| anyhow!("runtime unavailable"))?
        .submit(input)
        .await
        .map_err(|error| anyhow!("domain mutation failed: {error}"))
}

pub(crate) async fn submit_durable(
    context: &ApiContext,
    input: DomainInput,
) -> Result<maestria_runtime::DomainApplicationResult> {
    context
        .runtime
        .as_ref()
        .ok_or_else(|| anyhow!("runtime unavailable"))?
        .submit_durable(input)
        .await
        .map_err(|error| anyhow!("durable domain mutation failed: {error}"))
}
