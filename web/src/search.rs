use dioxus::prelude::*;

use crate::{
    api::{
        ApiClient, CoverageWire, SearchEvidence, SearchRawRank, SearchResponse, SearchScore,
        SearchScoreScale,
    },
    components::{EvidenceDialog, Shell, WorkspaceContext, alert},
    session::Session,
    state::LoadState,
};

#[component]
pub(crate) fn SearchWorkspace() -> Element {
    let mut context = use_context::<Signal<WorkspaceContext>>();
    let api = use_hook(ApiClient::new);
    let query = use_signal(Session::remembered_query);
    let results: Signal<LoadState<SearchResponse>> = use_signal(|| LoadState::Empty);
    let mut evidence = use_signal(|| None);
    let busy = matches!(results(), LoadState::Loading);
    rsx! {
        Shell { title: "Search", active_notebook: None,
            SearchForm {
                api: api.clone(),
                query,
                results,
                busy
            }
            if let LoadState::Failed(error) = results() {
                {alert(&error)}
            }
            if let LoadState::Ready(response) = results() {
                CoveragePanel {
                    coverage: response.coverage.clone(),
                    conflict_count: response.conflict_count
                }
                SearchResults {
                    evidence: response.evidence.clone(),
                    on_open: move |evidence_id| {
                        evidence.set(None);
                        let api = api.clone();
                        spawn(async move {
                            match api.evidence_global(evidence_id).await {
                                Ok(value) => evidence.set(Some(value)),
                                Err(error) => context.write().model.alert = Some(error),
                            }
                        });
                    }
                }
            }
        }
        EvidenceDialog {
            evidence: evidence(),
            on_close: move |_| evidence.set(None)
        }
    }
}

#[component]
fn SearchForm(
    api: ApiClient,
    mut query: Signal<String>,
    mut results: Signal<LoadState<SearchResponse>>,
    busy: bool,
) -> Element {
    rsx! {
        form {
            class: "mb-6 flex flex-col gap-3 sm:flex-row",
            onsubmit: move |event| {
                event.prevent_default();
                let text = query.read().trim().to_owned();
                if text.is_empty() {
                    return;
                }
                Session::remember_query(&text);
                results.set(LoadState::Loading);
                let api = api.clone();
                spawn(async move {
                    match api.search(&text, 10).await {
                        Ok(response) => results.set(LoadState::Ready(response)),
                        Err(error) => results.set(LoadState::Failed(error)),
                    }
                });
            },
            label { class: "sr-only", r#for: "search-query", "Search query" }
            input {
                id: "search-query",
                class: "flex-1 rounded border border-line bg-input px-3 py-2",
                placeholder: "Ask the index…",
                value: "{query}",
                oninput: move |event| query.set(event.value())
            }
            button {
                class: "rounded bg-accent px-4 py-2 font-semibold text-white disabled:opacity-50",
                disabled: busy || query.read().trim().is_empty(),
                {if busy { "Searching…" } else { "Search" }}
            }
        }
    }
}

#[component]
fn SearchResults(evidence: Vec<SearchEvidence>, on_open: EventHandler<u64>) -> Element {
    if evidence.is_empty() {
        return rsx! { p { class: "text-ink-muted", "No evidence matched this query." } };
    }
    rsx! {
        div { class: "space-y-4",
            for item in evidence {
                SearchResultCard { item, on_open }
            }
        }
    }
}

#[component]
fn SearchResultCard(item: SearchEvidence, on_open: EventHandler<u64>) -> Element {
    let evidence_id = item.evidence_id;
    rsx! {
        article {
            class: "rounded-lg border border-line bg-panel p-4 hover:border-accent",
            button {
                class: "block w-full text-left",
                onclick: move |_| on_open.call(evidence_id),
                header {
                    class: "flex flex-wrap items-center justify-between gap-2",
                    h3 { class: "font-semibold text-ink-strong", "{item.source}" }
                    div { class: "flex gap-2 text-xs",
                        span { class: "rounded bg-muted px-2 py-1 text-ink-muted", "trust {item.trust}" }
                        span { class: "rounded bg-muted px-2 py-1 text-ink-muted", "{item.freshness}" }
                    }
                }
                div { class: "mt-3 flex flex-wrap gap-2",
                    for score in item.scores {
                        ScoreBadge { score }
                    }
                }
            }
        }
    }
}

#[component]
fn ScoreBadge(score: SearchScore) -> Element {
    let value = score_value(&score);
    let detail = format!(
        "{} · fingerprint {}",
        score.representation, score.fingerprint
    );
    rsx! {
        span {
            class: "rounded border border-line bg-muted px-2 py-1 font-mono text-xs",
            title: detail,
            "{score.score_kind}: {value}"
        }
    }
}

fn score_value(score: &SearchScore) -> String {
    match &score.scale {
        SearchScoreScale::Binary => score.raw_score.to_string(),
        SearchScoreScale::Unbounded { name, .. } => {
            format!("{name} {}", score.raw_score)
        }
        SearchScoreScale::FixedPoint {
            name, denominator, ..
        } => {
            let denominator = *denominator as f64;
            let value = if denominator == 0.0 {
                0.0
            } else {
                score.raw_score as f64 / denominator
            };
            format!("{name} {value:.3}")
        }
        SearchScoreScale::RankDerived { name, .. } => match &score.raw_rank {
            SearchRawRank::Ranked { rank } => format!("{name} #{rank}"),
            SearchRawRank::Unavailable { reason } => format!("{name} {reason}"),
        },
    }
}

#[component]
fn CoveragePanel(coverage: CoverageWire, conflict_count: usize) -> Element {
    let percent = coverage.percent_covered.min(100);
    rsx! {
        section {
            class: "mb-6 rounded-lg border border-line bg-panel p-4",
            div { class: "flex items-center justify-between gap-3",
                h2 { class: "font-semibold text-ink-strong", "Coverage" }
                span { class: "text-sm text-ink-muted", "{percent}%" }
            }
            div {
                class: "mt-2 h-2 w-full overflow-hidden rounded bg-muted",
                aria_label: "Coverage percent",
                div {
                    class: "h-full bg-accent",
                    style: "width: {percent}%"
                }
            }
            p { class: "mt-3 text-sm text-ink-muted",
                "{coverage.distinct_sources} sources · {coverage.distinct_documents} documents · {coverage.distinct_sections} sections · {conflict_count} conflicts"
            }
            if coverage.gaps.is_empty() {
                p { class: "mt-2 text-sm text-success", "No gaps" }
            } else {
                div { class: "mt-2",
                    p { class: "text-sm font-medium text-ink-muted", "Gaps" }
                    ul { class: "list-disc pl-5 text-sm text-ink-muted",
                        for gap in coverage.gaps {
                            li { {gap} }
                        }
                    }
                }
            }
        }
    }
}
