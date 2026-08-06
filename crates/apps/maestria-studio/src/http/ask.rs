use axum::{extract::State, response::Json};
use maestria_daemon::api::{ClientOperation, ClientResponse, NotebookContextResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    error::{ProblemCode, StudioError},
    extract::{ApiJson, ApiPath},
    state::StudioState,
};

const MAX_HISTORY_TURNS: usize = 12;
const MAX_HISTORY_BYTES: usize = 32 * 1024;
const MAX_ANSWER_BYTES: usize = 16 * 1024;
const MAX_DRAFTS: usize = 3;
const MAX_DRAFT_BYTES: usize = 32 * 1024;
const MAX_TITLE_BYTES: usize = 200;
const MAX_CITATIONS: usize = 12;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudioAskRequest {
    pub question: String,
    #[serde(default)]
    pub history: Vec<StudioTurn>,
    pub agent_id: String,
    #[serde(default)]
    pub config: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudioTurn {
    pub role: StudioTurnRole,
    pub markdown: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StudioTurnRole {
    User,
    Assistant,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentOutput {
    pub answer_markdown: String,
    pub citation_ids: Vec<u64>,
    pub draft_previews: Vec<DraftPreview>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftPreview {
    pub title: String,
    pub markdown: String,
    pub citation_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioAskContext {
    pub answerability: String,
    pub coverage: StudioCoverage,
    pub gaps: Vec<String>,
    pub citations: Vec<StudioCitation>,
    pub trace_id: u64,
    pub query_id: u64,
    pub source_selection_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioCoverage {
    pub percent_covered: u8,
    pub gaps: Vec<String>,
    pub distinct_sources: usize,
    pub distinct_documents: usize,
    pub distinct_sections: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioCitation {
    pub rank: usize,
    pub score: i64,
    pub evidence: StudioEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioEvidence {
    pub evidence_id: u64,
    pub artifact_id: u64,
    pub artifact_title: String,
    pub artifact_content_hash: Option<String>,
    pub source: Value,
    pub excerpt: String,
    pub observed_at: u64,
}

#[derive(Debug, Serialize)]
pub struct StudioAskResponse {
    pub answer_markdown: String,
    pub citations: Vec<StudioCitation>,
    pub draft_previews: Vec<DraftPreviewResponse>,
    pub context: StudioAskContext,
}

#[derive(Debug, Serialize)]
pub struct DraftPreviewResponse {
    pub title: String,
    pub markdown: String,
    pub citations: Vec<StudioCitation>,
}

/// # Cancellation
///
/// Dropping the future cancels the daemon context lookup or agent operation.
pub async fn ask(
    State(state): State<StudioState>,
    ApiPath(notebook_id): ApiPath<u64>,
    ApiJson(input): ApiJson<StudioAskRequest>,
) -> Result<Json<StudioAskResponse>, StudioError> {
    validate_input(&input)?;
    let profile = state.agent.profile();
    if input.agent_id != profile.id || !input.config.is_empty() {
        return Err(StudioError::new(ProblemCode::InvalidInput));
    }
    let daemon_response = state
        .client
        .request(ClientOperation::NotebookContext {
            notebook_id,
            query: input.question.trim().to_owned(),
            limit: 8,
            max_context_bytes: 49_152,
        })
        .await?;
    let ClientResponse::NotebookContext(daemon_context) = daemon_response else {
        return Err(StudioError::new(ProblemCode::Internal));
    };
    let context = map_context(&daemon_context)?;
    let prompt = build_prompt(&input, &context)
        .map_err(|error| StudioError::with_source(ProblemCode::Internal, error))?;
    let raw = state.agent.ask(prompt).await?;
    let output = serde_json::from_str::<AgentOutput>(&raw).map_err(|error| {
        StudioError::with_source(ProblemCode::InvalidAgentOutput, anyhow::Error::new(error))
    })?;
    let records = evidence_records(&context);
    let citations = checked_citations(&output.citation_ids, &records)?;
    if output.draft_previews.len() > MAX_DRAFTS {
        return Err(StudioError::new(ProblemCode::InvalidAgentOutput));
    }
    let mut previews = Vec::with_capacity(output.draft_previews.len());
    for preview in output.draft_previews {
        if preview.title.trim().is_empty()
            || preview.title.len() > MAX_TITLE_BYTES
            || preview.markdown.is_empty()
            || preview.markdown.len() > MAX_DRAFT_BYTES
        {
            return Err(StudioError::new(ProblemCode::InvalidAgentOutput));
        }
        previews.push(DraftPreviewResponse {
            title: preview.title,
            markdown: preview.markdown,
            citations: checked_citations(&preview.citation_ids, &records)?,
        });
    }
    if output.answer_markdown.is_empty() || output.answer_markdown.len() > MAX_ANSWER_BYTES {
        return Err(StudioError::new(ProblemCode::InvalidAgentOutput));
    }
    Ok(Json(StudioAskResponse {
        answer_markdown: output.answer_markdown,
        citations,
        draft_previews: previews,
        context,
    }))
}

fn validate_input(input: &StudioAskRequest) -> Result<(), StudioError> {
    if !(1..=4096).contains(&input.question.trim().len())
        || input.history.len() > MAX_HISTORY_TURNS
        || input
            .history
            .iter()
            .map(|turn| turn.markdown.len())
            .sum::<usize>()
            > MAX_HISTORY_BYTES
    {
        Err(StudioError::new(ProblemCode::InvalidInput))
    } else {
        Ok(())
    }
}

fn map_context(context: &NotebookContextResponse) -> Result<StudioAskContext, StudioError> {
    let citations = context
        .citations
        .iter()
        .map(map_citation)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StudioAskContext {
        answerability: context.answerability.clone(),
        coverage: StudioCoverage {
            percent_covered: context.coverage.percent_covered,
            gaps: context.coverage.gaps.clone(),
            distinct_sources: context.coverage.distinct_sources,
            distinct_documents: context.coverage.distinct_documents,
            distinct_sections: context.coverage.distinct_sections,
        },
        gaps: context.gaps.clone(),
        citations,
        trace_id: context.trace_id,
        query_id: context.query_id,
        source_selection_digest: context.source_selection_digest.clone(),
    })
}

fn map_citation(
    citation: &maestria_daemon::api::NotebookCitationResponse,
) -> Result<StudioCitation, StudioError> {
    let source = serde_json::to_value(&citation.evidence.source).map_err(|error| {
        StudioError::with_source(ProblemCode::Internal, anyhow::Error::new(error))
    })?;
    Ok(StudioCitation {
        rank: citation.rank,
        score: citation.score,
        evidence: StudioEvidence {
            evidence_id: citation.evidence.evidence_id,
            artifact_id: citation.evidence.artifact_id,
            artifact_title: citation.evidence.artifact_title.clone(),
            artifact_content_hash: citation.evidence.artifact_content_hash.clone(),
            source,
            excerpt: citation.evidence.excerpt.clone(),
            observed_at: citation.evidence.observed_at,
        },
    })
}

fn build_prompt(
    input: &StudioAskRequest,
    context: &StudioAskContext,
) -> Result<String, anyhow::Error> {
    let context_json = serde_json::to_string(context)?;
    let mut prompt = format!(
        "Return exactly one JSON object with keys answer_markdown, citation_ids, and draft_previews. Every grounded answer must include exactly one transferable draft preview in draft_previews: give it a concise title, Markdown that preserves the answer, and citation_ids drawn only from the cited context. Never return an empty draft_previews array. Use only the delimited source context.\n<question>{}</question>\n<context>{context_json}</context>",
        input.question.trim(),
    );
    if !input.history.is_empty() {
        prompt.push_str("\n<history>");
        for turn in &input.history {
            let role = match turn.role {
                StudioTurnRole::User => "user",
                StudioTurnRole::Assistant => "assistant",
            };
            prompt.push_str(&format!("<{role}>{}</{role}>", turn.markdown));
        }
        prompt.push_str("</history>");
    }
    Ok(prompt)
}

fn evidence_records(context: &StudioAskContext) -> std::collections::BTreeMap<u64, StudioCitation> {
    context
        .citations
        .iter()
        .map(|citation| (citation.evidence.evidence_id, citation.clone()))
        .collect()
}

fn checked_citations(
    ids: &[u64],
    records: &std::collections::BTreeMap<u64, StudioCitation>,
) -> Result<Vec<StudioCitation>, StudioError> {
    if ids.len() > MAX_CITATIONS {
        return Err(StudioError::new(ProblemCode::InvalidAgentOutput));
    }
    let mut unique = std::collections::BTreeSet::new();
    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.insert(*id) {
            return Err(StudioError::new(ProblemCode::InvalidAgentOutput));
        }
        let Some(record) = records.get(id) else {
            return Err(StudioError::new(ProblemCode::InvalidAgentOutput));
        };
        result.push(record.clone());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{StudioAskRequest, build_prompt, map_context};
    use maestria_daemon::api::NotebookContextResponse;

    fn grounded_context() -> Result<super::StudioAskContext, Box<dyn std::error::Error>> {
        let daemon: NotebookContextResponse = serde_json::from_str(
            r#"{"query":"rust","query_id":7,"trace_id":8,"source_selection_digest":"digest","index_generation":3,"fingerprint":"fp","answerability":"grounded","coverage":{"percent_covered":75,"gaps":["missing API"],"distinct_sources":1,"distinct_documents":1,"distinct_sections":2},"gaps":["missing API"],"citations":[{"rank":1,"score":930,"evidence":{"evidence_id":42,"artifact_id":9,"artifact_title":"Guide","artifact_content_hash":"hash","source":{"type":"file","path":"guide.md","start_line":4,"end_line":9,"content_hash":"hash"},"excerpt":"Axum routes","observed_at":11}}]}"#,
        )?;
        match map_context(&daemon) {
            Ok(value) => Ok(value),
            Err(_) => Err("context mapping failed".into()),
        }
    }

    #[test]
    fn context_wire_is_flattened_for_studio_clients() -> Result<(), Box<dyn std::error::Error>> {
        let daemon: NotebookContextResponse = serde_json::from_str(
            r#"{"query":"rust","query_id":7,"trace_id":8,"source_selection_digest":"digest","index_generation":3,"fingerprint":"fp","answerability":"grounded","coverage":{"percent_covered":75,"gaps":["missing API"],"distinct_sources":1,"distinct_documents":1,"distinct_sections":2},"gaps":["missing API"],"citations":[{"rank":1,"score":930,"evidence":{"evidence_id":42,"artifact_id":9,"artifact_title":"Guide","artifact_content_hash":"hash","source":{"type":"file","path":"guide.md","start_line":4,"end_line":9,"content_hash":"hash"},"excerpt":"Axum routes","observed_at":11}}]}"#,
        )?;
        let studio = match map_context(&daemon) {
            Ok(value) => value,
            Err(_) => return Err("context mapping failed".into()),
        };
        let json = serde_json::to_value(studio)?;
        assert_eq!(json["answerability"], "grounded");
        assert_eq!(json["coverage"]["percent_covered"], 75);
        assert_eq!(json["citations"][0]["evidence"]["evidence_id"], 42);
        assert_eq!(json["citations"][0]["evidence"]["source"]["type"], "file");
        Ok(())
    }
    #[test]
    fn prompt_requires_transferable_draft_preview() -> Result<(), Box<dyn std::error::Error>> {
        let input = StudioAskRequest {
            question: "What should every answer expose?".into(),
            history: Vec::new(),
            agent_id: "omp".into(),
            config: Default::default(),
        };
        let prompt = build_prompt(&input, &grounded_context()?)?;
        assert!(
            prompt.contains(
                "Every grounded answer must include exactly one transferable draft preview"
            )
        );
        assert!(prompt.contains("Never return an empty draft_previews array"));
        Ok(())
    }
}
