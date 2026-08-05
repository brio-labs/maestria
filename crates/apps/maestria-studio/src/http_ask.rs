use anyhow::{Context, Result, anyhow};
use maestria_daemon::api::ClientOperation;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{StudioState, json_response};

const MAX_HISTORY_TURNS: usize = 12;
const MAX_HISTORY_BYTES: usize = 32 * 1024;
const MAX_ANSWER_BYTES: usize = 16 * 1024;
const MAX_DRAFTS: usize = 3;
const MAX_DRAFT_BYTES: usize = 32 * 1024;
const MAX_TITLE_BYTES: usize = 200;
const MAX_CITATIONS: usize = 12;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StudioTurn {
    role: StudioTurnRole,
    markdown: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum StudioTurnRole {
    User,
    Assistant,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StudioAskRequest {
    question: String,
    #[serde(default)]
    history: Vec<StudioTurn>,
    agent_id: String,
    #[serde(default)]
    config: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentOutput {
    answer_markdown: String,
    citation_ids: Vec<u64>,
    draft_previews: Vec<DraftPreview>,
}

#[derive(Debug, Serialize)]
struct StudioAskResponse {
    answer_markdown: String,
    citations: Vec<Value>,
    draft_previews: Vec<DraftPreviewResponse>,
    context: Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DraftPreview {
    title: String,
    markdown: String,
    citation_ids: Vec<u64>,
}

#[derive(Debug, Serialize)]
struct DraftPreviewResponse {
    title: String,
    markdown: String,
    citations: Vec<Value>,
}

pub(super) async fn route_notebook_ask(
    state: &StudioState,
    method: &str,
    body: &[u8],
    notebook_id: u64,
) -> Result<(u16, &'static str, Vec<u8>)> {
    if method != "POST" {
        return Err(anyhow!("method not allowed"));
    }
    let input: StudioAskRequest =
        serde_json::from_slice(body).context("decode Studio ask request")?;
    let question = input.question.trim();
    if !(1..=4096).contains(&question.len()) {
        return Err(anyhow!("question must be 1..=4096 bytes"));
    }
    if input.history.len() > MAX_HISTORY_TURNS {
        return Err(anyhow!("ask history exceeds Studio limit"));
    }
    let history_bytes: usize = input.history.iter().map(|turn| turn.markdown.len()).sum();
    if history_bytes > MAX_HISTORY_BYTES {
        return Err(anyhow!("ask history exceeds Studio limit"));
    }
    let profile = state.agent.profile();
    if profile.status == "agent_unconfigured" {
        return Err(anyhow!("agent is unconfigured"));
    }
    if input.agent_id != profile.id {
        return Err(anyhow!("agent profile is unavailable"));
    }
    if !input.config.is_empty() {
        return Err(anyhow!("agent config option is unavailable"));
    }
    let context = state
        .client
        .request(ClientOperation::NotebookContext {
            notebook_id,
            query: question.to_owned(),
            limit: 8,
            max_context_bytes: 49_152,
        })
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let context_value = serde_json::to_value(&context).context("encode notebook context")?;
    let mut prompt = format!(
        "Return exactly one JSON object with keys answer_markdown, citation_ids, and draft_previews. Use only the delimited source context.\\n<question>{question}</question>\\n<context>{}</context>",
        serde_json::to_string(&context_value).context("encode notebook context")?
    );
    if !input.history.is_empty() {
        prompt.push_str("\\n<history>");
        for turn in input.history {
            let role = match turn.role {
                StudioTurnRole::User => "user",
                StudioTurnRole::Assistant => "assistant",
            };
            prompt.push_str(&format!("<{role}>{}</{role}>", turn.markdown));
        }
        prompt.push_str("</history>");
    }
    let raw = state.agent.ask(prompt).await?;
    let output: AgentOutput =
        serde_json::from_str(&raw).context("agent output must be one JSON object")?;
    let records = evidence_records(&context_value);
    let citations = checked_citations(&output.citation_ids, &records)?;
    if output.draft_previews.len() > MAX_DRAFTS {
        return Err(anyhow!("agent returned too many draft previews"));
    }
    let mut previews = Vec::with_capacity(output.draft_previews.len());
    for preview in output.draft_previews {
        if preview.title.trim().is_empty() || preview.title.len() > MAX_TITLE_BYTES {
            return Err(anyhow!("agent draft title exceeds Studio limit"));
        }
        if preview.markdown.is_empty() || preview.markdown.len() > MAX_DRAFT_BYTES {
            return Err(anyhow!("agent draft preview exceeds Studio limit"));
        }
        let preview_citations = checked_citations(&preview.citation_ids, &records)?;
        previews.push(DraftPreviewResponse {
            title: preview.title,
            markdown: preview.markdown,
            citations: preview_citations,
        });
    }
    if output.answer_markdown.is_empty() || output.answer_markdown.len() > MAX_ANSWER_BYTES {
        return Err(anyhow!("agent answer exceeds Studio limit"));
    }
    let response = StudioAskResponse {
        answer_markdown: output.answer_markdown,
        citations,
        draft_previews: previews,
        context: context_value,
    };
    json_response(200, &response)
}

fn evidence_records(context: &Value) -> std::collections::BTreeMap<u64, Value> {
    fn walk(value: &Value, records: &mut std::collections::BTreeMap<u64, Value>) {
        match value {
            Value::Object(map) => {
                if let Some(id) = map.get("evidence_id").and_then(Value::as_u64) {
                    records.insert(id, value.clone());
                }
                map.values().for_each(|child| walk(child, records));
            }
            Value::Array(values) => values.iter().for_each(|child| walk(child, records)),
            _ => {}
        }
    }
    let mut records = std::collections::BTreeMap::new();
    walk(context, &mut records);
    records
}

fn checked_citations(
    ids: &[u64],
    records: &std::collections::BTreeMap<u64, Value>,
) -> Result<Vec<Value>> {
    if ids.len() > MAX_CITATIONS {
        return Err(anyhow!("agent returned too many citations"));
    }
    let mut unique = std::collections::BTreeSet::new();
    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.insert(*id) {
            return Err(anyhow!("agent returned duplicate citation ID"));
        }
        result.push(
            records
                .get(id)
                .cloned()
                .ok_or_else(|| anyhow!("agent cited evidence outside the grounded context"))?,
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{AgentOutput, Value, checked_citations, evidence_records};

    #[test]
    fn agent_output_rejects_unknown_fields() {
        let parsed = serde_json::from_str::<AgentOutput>(
            r#"{"answer_markdown":"ok","citation_ids":[],"unexpected":true}"#,
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn citations_must_be_unique_and_grounded() -> Result<(), Box<dyn std::error::Error>> {
        let context: Value =
            serde_json::from_str(r#"{"citations":[{"evidence_id":7,"excerpt":"x"}]}"#)?;
        let records = evidence_records(&context);
        assert!(checked_citations(&[7], &records).is_ok());
        assert!(checked_citations(&[7, 7], &records).is_err());
        assert!(checked_citations(&[9], &records).is_err());
        Ok(())
    }
}
