use std::collections::BTreeSet;

use dioxus::prelude::*;

use crate::{
    api::{ApiClient, CatalogSource, Notebook},
    components::{WorkspaceContext, set_alert},
    state::LoadState,
};

pub(super) async fn refresh_notebook(
    api: &ApiClient,
    notebook_id: u64,
) -> Result<(Notebook, Vec<CatalogSource>), crate::api::ClientError> {
    let notebook = api.notebook(notebook_id).await?;
    let sources = api.sources().await?;
    Ok((notebook, sources))
}

#[component]
pub(super) fn Sources(notebook_id: u64) -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let (sources, selected_keys) = {
        let snapshot = context.read();
        let sources = match &snapshot.model.sources {
            LoadState::Ready(values) => values.clone(),
            _ => Vec::new(),
        };
        let selected_keys: BTreeSet<String> = match &snapshot.model.notebook {
            LoadState::Ready(notebook) => notebook
                .sources
                .iter()
                .filter(|item| item.available)
                .map(|item| item.source_key.clone())
                .collect(),
            _ => BTreeSet::new(),
        };
        (sources, selected_keys)
    };
    let selected_count = sources.iter().filter(|source| source.available).count();
    let api = use_hook(ApiClient::new);
    rsx! {
        section { class: "rounded-lg border border-line bg-panel p-5",
            h2 { class: "font-semibold text-ink-strong", "Sources" }
            p {
                class: "mt-1 text-sm text-ink-muted",
                "{selected_count} selected / {sources.len()} available"
            }
            div { class: "mt-4 divide-y divide-line",
                for source in sources {
                    SourceRow {
                        notebook_id,
                        source: source.clone(),
                        selected: selected_keys.contains(&source.source_key),
                        api: api.clone(),
                        context
                    }
                }
            }
        }
    }
}

#[component]
fn SourceRow(
    notebook_id: u64,
    source: CatalogSource,
    selected: bool,
    api: ApiClient,
    context: Signal<WorkspaceContext>,
) -> Element {
    let key = source.source_key.clone();
    let title = match source.title.clone() {
        Some(value) => value,
        None => source.source_key.clone(),
    };
    rsx! {
        div { class: "flex items-center justify-between gap-3 py-3",
            div {
                p { class: "font-medium", {title} }
                p {
                    class: "text-sm text-ink-muted",
                    "{source.source_key} · {source.index_status}"
                }
            }
            button {
                class: "rounded border border-line px-3 py-2",
                disabled: !source.available,
                onclick: move |_| {
                    let key = key.clone();
                    let api = api.clone();
                    spawn(async move {
                        let result = if selected {
                            api.detach_source(notebook_id, &key).await
                        } else {
                            api.attach_source(notebook_id, &key).await
                        };
                        match result {
                            Err(error) => set_alert(context, error),
                            Ok(()) => match refresh_notebook(&api, notebook_id).await {
                                Ok((notebook, sources)) => {
                                    let mut value = context.write();
                                    value.model.notebook = LoadState::Ready(notebook);
                                    value.model.sources = LoadState::ready_or_empty(sources);
                                    value.model.alert = None;
                                    value.model.status = "Sources updated".into();
                                }
                                Err(error) => set_alert(context, error),
                            },
                        }
                    });
                },
                {if selected { "Detach" } else { "Attach" }}
            }
        }
    }
}
