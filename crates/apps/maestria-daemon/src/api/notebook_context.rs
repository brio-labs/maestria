use std::collections::BTreeSet;

use anyhow::{Result, anyhow};
use maestria_domain::{IndexStatus, NotebookId};

use super::super::super::protocol::{
    CoverageResponse, NotebookCitationResponse, NotebookContextResponse,
};
use super::super::super::server::ApiContext;
use super::super::read_services::evidence_response;
use super::support::{manifest, source_allowed, state};

const DEFAULT_CONTEXT_LIMIT: usize = 8;
const MAX_CONTEXT_LIMIT: usize = 20;
const DEFAULT_CONTEXT_BYTES: usize = 49_152;
const MAX_CONTEXT_BYTES: usize = 49_152;

pub(crate) async fn context(
    context: &ApiContext,
    notebook_id: u64,
    query: String,
    limit: usize,
    max_context_bytes: usize,
) -> Result<NotebookContextResponse> {
    let (query, limit, max_context_bytes) =
        validate_context_request(query, limit, max_context_bytes)?;
    let state = state(context).await?;
    let manifest = manifest(context)?;
    let artifact_ids = selected_artifact_ids(&state, NotebookId::new(notebook_id), &manifest)?;
    let executor = context
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.search_executor())
        .ok_or_else(|| anyhow!("search executor unavailable"))?;
    let (plan, outcome) = executor
        .plan_and_search_selected(query.clone(), limit, artifact_ids.clone())
        .await
        .map_err(|error| anyhow!("selected source search: {error}"))?;
    let mut citations = Vec::new();
    for (rank, candidate) in outcome.evidence.iter().enumerate() {
        let output = crate::evidence_open::open_evidence_scoped(
            &context.layout,
            candidate.evidence_id().value(),
        )?;
        if !artifact_ids.contains(&output.evidence.artifact_id) {
            continue;
        }
        citations.push(NotebookCitationResponse {
            rank: rank + 1,
            score: candidate
                .scores()
                .lanes()
                .first()
                .map_or(0, |score| score.raw_score),
            evidence: evidence_response(output)?,
        });
        let serialized = serde_json::to_vec(&citations)
            .map_err(|error| anyhow!("serialize notebook context: {error}"))?;
        if serialized
            .len()
            .saturating_add(query.len())
            .saturating_add(1024)
            > max_context_bytes
        {
            citations.pop();
            break;
        }
    }
    if citations.is_empty()
        || !matches!(
            outcome.status,
            maestria_domain::SearchStatus::Answerable
                | maestria_domain::SearchStatus::AnswerableWithWarnings
        )
    {
        return Err(anyhow!(
            "no_evidence: selected sources did not yield answerable evidence"
        ));
    }
    let trace = outcome.trace_data.as_deref();
    Ok(NotebookContextResponse {
        query,
        query_id: plan.query_id().value(),
        trace_id: outcome.trace.value(),
        source_selection_digest: trace.and_then(|trace| trace.source_selection_digest.clone()),
        index_generation: outcome.index_generation.value(),
        fingerprint: outcome.fingerprint.as_str().to_owned(),
        answerability: format!("{:?}", outcome.status),
        coverage: CoverageResponse {
            percent_covered: outcome.coverage.percent_covered(),
            gaps: outcome.coverage.gaps_identified().to_vec(),
            distinct_sources: outcome.coverage.distinct_sources(),
            distinct_documents: outcome.coverage.distinct_documents(),
            distinct_sections: outcome.coverage.distinct_sections(),
        },
        gaps: outcome.coverage.gaps_identified().to_vec(),
        citations,
    })
}

pub(crate) async fn evidence(
    context: &ApiContext,
    notebook_id: u64,
    evidence_id: u64,
) -> Result<super::super::super::protocol::EvidenceResponse> {
    let state = state(context).await?;
    let notebook = state
        .notebooks
        .get(&NotebookId::new(notebook_id))
        .ok_or_else(|| anyhow!("notebook not found"))?;
    let evidence = state
        .evidences
        .get(&maestria_domain::EvidenceId::new(evidence_id))
        .ok_or_else(|| anyhow!("evidence not found"))?;
    if !source_allowed(context, &state, evidence.artifact_id)? {
        return Err(anyhow!("source_not_selected"));
    }
    let selected = notebook.source_keys.iter().any(|key| {
        state.active_sources.get(key) == Some(&evidence.artifact_id)
            && source_allowed(context, &state, evidence.artifact_id).is_ok_and(|allowed| allowed)
    });
    if !selected {
        return Err(anyhow!("source_not_selected"));
    }
    let output = crate::evidence_open::open_evidence_scoped(&context.layout, evidence_id)?;
    evidence_response(output)
}
fn validate_context_request(
    query: String,
    limit: usize,
    max_context_bytes: usize,
) -> Result<(String, usize, usize)> {
    let query = query.trim().to_owned();
    if query.is_empty() || query.len() > 4096 {
        return Err(anyhow!("notebook query must be between 1 and 4096 bytes"));
    }
    let limit = if limit == 0 {
        DEFAULT_CONTEXT_LIMIT
    } else {
        limit
    };
    if !(1..=MAX_CONTEXT_LIMIT).contains(&limit) {
        return Err(anyhow!(
            "notebook context limit must be between 1 and {MAX_CONTEXT_LIMIT}"
        ));
    }
    let max_context_bytes = if max_context_bytes == 0 {
        DEFAULT_CONTEXT_BYTES
    } else {
        max_context_bytes
    };
    if !(1024..=MAX_CONTEXT_BYTES).contains(&max_context_bytes) {
        return Err(anyhow!("notebook context byte limit is invalid"));
    }
    Ok((query, limit, max_context_bytes))
}

fn selected_artifact_ids(
    state: &maestria_domain::KernelState,
    notebook_id: NotebookId,
    manifest: &maestria_core::InstanceManifest,
) -> Result<BTreeSet<maestria_domain::ArtifactId>> {
    let notebook = state
        .notebooks
        .get(&notebook_id)
        .ok_or_else(|| anyhow!("notebook not found"))?;
    let artifact_ids: BTreeSet<_> = notebook
        .source_keys
        .iter()
        .filter_map(|key| {
            let id = state.active_sources.get(key).copied()?;
            if !manifest.allows_source(std::path::Path::new(key.as_str())) {
                return None;
            }
            state
                .artifacts
                .get(&id)
                .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed)
                .then_some(id)
        })
        .collect();
    if artifact_ids.is_empty() {
        return Err(anyhow!(
            "source_unavailable: notebook has no indexed selected sources"
        ));
    }
    Ok(artifact_ids)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use maestria_domain::{
        Artifact, ArtifactId, KernelState, Notebook, NotebookTitle, SecurityMetadata,
        SourceIdentityKey,
    };
    use std::collections::BTreeSet;

    #[test]
    fn selected_context_accepts_completed_indexed_source() -> Result<(), Box<dyn std::error::Error>>
    {
        let source_path = "/tmp/maestria-context-regression/source.md";
        let artifact_id = ArtifactId::new(7);
        let source_key = SourceIdentityKey::try_from(source_path.to_owned())?;
        let notebook_id = NotebookId::new(3);
        let mut state = KernelState::default();
        Arc::make_mut(&mut state.active_sources).insert(source_key.clone(), artifact_id);
        Arc::make_mut(&mut state.artifacts).insert(
            artifact_id,
            Artifact {
                id: artifact_id,
                title: "source.md".to_owned(),
                chunk_ids: BTreeSet::new(),
                card_ids: BTreeSet::new(),
                claim_ids: BTreeSet::new(),
                evidence_ids: BTreeSet::new(),
                index_status: IndexStatus::Indexed,
                content_hash: None,
                parse_status: None,
                security: SecurityMetadata::default(),
            },
        );
        Arc::make_mut(&mut state.notebooks).insert(
            notebook_id,
            Notebook {
                id: notebook_id,
                title: NotebookTitle::try_from("Regression".to_owned())?,
                source_keys: [source_key].into_iter().collect(),
                created_at: maestria_domain::LogicalTick::new(1),
                updated_at: maestria_domain::LogicalTick::new(1),
            },
        );
        let manifest = maestria_core::InstanceManifest::decode(
            "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/maestria-context-regression\nread_root=/tmp/maestria-context-regression\nexcluded_pattern=.env\n",
        )?;

        let selected = selected_artifact_ids(&state, notebook_id, &manifest)?;

        assert_eq!(selected, [artifact_id].into_iter().collect());
        assert!(state.pending_parsers.is_empty());
        Ok(())
    }
}
