use dioxus::prelude::*;

use crate::{
    api::{ApiClient, AskContext, AskRequest, AskTurn, Citation, DraftPreview},
    components::{CitationList, EvidenceDialog, WorkspaceContext, set_alert},
    markdown::render_markdown,
    route::Route,
};

async fn run_ask(
    api: ApiClient,
    notebook_id: u64,
    mut context: Signal<WorkspaceContext>,
    text: String,
    history: Vec<AskTurn>,
    agent_id: String,
    request: (u64, u64),
) {
    let result = api
        .ask(
            notebook_id,
            &AskRequest {
                question: text.clone(),
                history,
                agent_id,
                config: Default::default(),
            },
        )
        .await;
    match result {
        Ok(response) => {
            let preview = response
                .draft_previews
                .first()
                .map(|value| crate::state::PreviewState {
                    title: value.title.clone(),
                    markdown: value.markdown.clone(),
                    evidence_ids: value
                        .citations
                        .iter()
                        .map(|citation| citation.evidence.evidence_id)
                        .collect(),
                });
            let mut value = context.write();
            if value.model.is_current_ask(request, notebook_id) {
                value
                    .model
                    .ask_history
                    .push_pair(text, response.answer_markdown.clone());
                value.model.answer = Some(response.answer_markdown);
                value.model.context = Some(response.context);
                value.model.preview = preview;
                value.model.draft_previews = response.draft_previews;
                value.model.alert = None;
                value.model.status = "Answer ready".into();
            }
        }
        Err(error) => {
            if context.read().model.is_current_ask(request, notebook_id) {
                set_alert(context, error);
            }
        }
    }
}

#[component]
pub(crate) fn Ask(notebook_id: u64) -> Element {
    let mut context = use_context::<Signal<WorkspaceContext>>();
    let question = use_signal(String::new);
    let busy = use_signal(|| false);
    let api = use_hook(ApiClient::new);
    let (agent_id, agent_ready, agents, evidence, answer, ask_context, citations, preview) = {
        let snapshot = context.read();
        let agent_id = snapshot
            .agent
            .as_ref()
            .map_or_else(String::new, |agent| agent.id.clone());
        let agent_ready = snapshot
            .agent
            .as_ref()
            .is_some_and(|agent| agent.status == "ready");
        let agents = snapshot.model.agents.clone();
        let evidence = snapshot.evidence.clone();
        let current = snapshot.model.ask_notebook == Some(notebook_id);
        let answer = if current {
            snapshot.model.answer.clone()
        } else {
            None
        };
        let citations = if current {
            snapshot
                .model
                .context
                .as_ref()
                .map_or_else(Vec::new, |value| value.citations.clone())
        } else {
            Vec::new()
        };
        let ask_context = if current {
            snapshot.model.context.clone()
        } else {
            None
        };
        let preview = if current {
            snapshot.model.draft_previews.first().cloned()
        } else {
            None
        };
        (
            agent_id,
            agent_ready,
            agents,
            evidence,
            answer,
            ask_context,
            citations,
            preview,
        )
    };
    rsx! {
        section { class: "rounded-lg border border-line bg-panel p-5",
            h2 { class: "font-semibold text-ink-strong", "Ask" }
            AskControls {
                notebook_id,
                context,
                api: api.clone(),
                question,
                busy,
                agent_id,
                agent_ready,
                agents
            }
            AskAnswer { answer }
            AskCoverage { context: ask_context }
            AskEvidence { notebook_id, context, api, citations }
            AskPreview { notebook_id, preview }
        }
        EvidenceDialog {
            evidence,
            on_close: move |_| {
                context.write().evidence = None;
            }
        }
    }
}

#[component]
fn AskControls(
    notebook_id: u64,
    context: Signal<WorkspaceContext>,
    api: ApiClient,
    mut question: Signal<String>,
    mut busy: Signal<bool>,
    agent_id: String,
    agent_ready: bool,
    agents: Vec<crate::api::Agent>,
) -> Element {
    let can_ask = agent_ready && !question.read().trim().is_empty() && !*busy.read();
    let ask_api = api.clone();
    rsx! {
        textarea {
            class: "mt-4 min-h-32 w-full rounded border border-line bg-input p-3",
            aria_label: "Question",
            value: "{question}",
            oninput: move |event| question.set(event.value()),
            placeholder: "What should we investigate?"
        }
        if agents.is_empty() {
            p {
                class: "mt-3 rounded border border-line bg-muted p-3 text-sm text-ink-muted",
                "No agent configured — add an ACP v1 profile (any CLI backed by a cloud LLM) to <instance>/system/studio-agents.toml, then restart Studio."
            }
        } else {
            div { class: "mt-3 flex flex-wrap items-center gap-3",
                select {
                    class: "rounded border border-line bg-input px-3 py-2 text-sm",
                    aria_label: "Agent",
                    value: agent_id.clone(),
                    onchange: move |event| {
                        let id = event.value();
                        let mut value = context.write();
                        if let Some(agent) = value
                            .model
                            .agents
                            .iter()
                            .find(|agent| agent.id == id)
                            .cloned()
                        {
                            value.agent = Some(agent);
                        }
                        crate::session::Session::remember_agent(id);
                    },
                    for agent in agents {
                        option { value: "{agent.id}", "{agent.label} ({agent.status})" }
                    }
                }
                button {
                    class: "rounded bg-accent px-4 py-2 text-white disabled:opacity-50",
                    disabled: !can_ask,
                    onclick: move |_| {
                        let text = question.read().trim().to_owned();
                        let (history, request) = {
                            let mut state = context.write();
                            let request = state.model.begin_ask(notebook_id);
                            let history = if state.model.ask_notebook == Some(notebook_id) {
                                state
                                    .model
                                    .ask_history
                                    .messages()
                                    .iter()
                                    .map(|message| AskTurn {
                                        role: message.role.clone(),
                                        markdown: message.markdown.clone()
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            (history, request)
                        };
                        let api = ask_api.clone();
                        let request_agent_id = agent_id.clone();
                        busy.set(true);
                        spawn(async move {
                            run_ask(
                                api,
                                notebook_id,
                                context,
                                text,
                                history,
                                request_agent_id,
                                request
                            )
                            .await;
                            busy.set(false);
                        });
                    },
                    {if *busy.read() { "Asking…" } else { "Ask" }}
                }
                button {
                    class: "rounded border border-line px-4 py-2",
                    onclick: move |_| {
                        context.write().model.clear_ask(notebook_id, notebook_id);
                    },
                    "Clear"
                }
            }
        }
    }
}

#[component]
fn AskAnswer(answer: Option<String>) -> Element {
    let Some(answer) = answer else {
        return rsx! {};
    };
    rsx! {
        article {
            class: "markdown-body mt-6",
            h3 {
                class: "mb-3 text-lg font-semibold",
                "Answer"
            }
            div {
                dangerous_inner_html: render_markdown(&answer)
            }
        }
    }
}

#[component]
fn AskCoverage(context: Option<AskContext>) -> Element {
    let Some(context) = context else {
        return rsx! {};
    };
    let answerability = match context.answerability {
        Some(value) => value,
        None => "unknown".to_owned(),
    };
    let coverage = context.coverage.map_or_else(
        || "Coverage unavailable".to_owned(),
        |value| {
            format!(
                "{:.0}% across {} source(s)",
                value.percent_covered, value.distinct_sources
            )
        },
    );
    rsx! {
        section {
            class: "mt-4 rounded border border-line bg-muted p-3 text-sm",
            p { "Answerability: {answerability}" }
            p { "Coverage: {coverage}" }
            if !context.gaps.is_empty() {
                p { class: "mt-2 font-medium", "Gaps" }
                ul { class: "list-disc pl-5",
                    for gap in context.gaps {
                        li { gap }
                    }
                }
            }
        }
    }
}

#[component]
fn AskEvidence(
    notebook_id: u64,
    context: Signal<WorkspaceContext>,
    api: ApiClient,
    citations: Vec<Citation>,
) -> Element {
    if citations.is_empty() {
        return rsx! {};
    }
    let evidence_api = api.clone();
    rsx! {
        h3 { class: "mt-6 mb-3 text-lg font-semibold", "Evidence" }
        CitationList {
            citations,
            on_open: move |evidence_id| {
                let api = evidence_api.clone();
                spawn(async move {
                    match api.evidence(notebook_id, evidence_id).await {
                        Ok(evidence) => context.write().evidence = Some(evidence),
                        Err(error) => set_alert(context, error)
                    }
                });
            }
        }
    }
}

#[component]
fn AskPreview(notebook_id: u64, preview: Option<DraftPreview>) -> Element {
    let Some(preview) = preview else {
        return rsx! {};
    };
    let navigator = use_navigator();
    rsx! {
        article {
            class: "mt-6 rounded border border-frozen bg-purple-50 p-4",
            h3 { class: "font-semibold", "Unsaved draft preview" }
            p { "{preview.title}" }
            a {
                class: "mt-3 inline-block rounded bg-accent px-3 py-2 text-white",
                href: "/notebooks/{notebook_id}/drafts",
                onclick: move |event| {
                    event.prevent_default();
                    navigator.push(Route::NotebookDrafts { notebook_id });
                },
                "Transfer to Drafts"
            }
        }
    }
}
