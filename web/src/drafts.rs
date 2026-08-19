use dioxus::prelude::*;

use crate::{
    api::{ApiClient, ClientError, CreateDraft, Draft, DraftSummary, UpdateDraft},
    components::{WorkspaceContext, set_alert},
    state::LoadState,
};

#[component]
pub fn Drafts(notebook_id: u64) -> Element {
    let context = use_context::<Signal<WorkspaceContext>>();
    let (drafts, preview_title, preview_markdown, conflict) = {
        let snapshot = context.read();
        let drafts = match &snapshot.model.drafts {
            LoadState::Ready(values) => values.clone(),
            _ => Vec::new(),
        };
        let preview = snapshot.model.preview.clone();
        let conflict = snapshot
            .model
            .alert
            .as_ref()
            .is_some_and(|error| error.problem_code() == Some("revision-conflict"));
        let preview_title = preview
            .as_ref()
            .map_or_else(String::new, |value| value.title.clone());
        let preview_markdown = preview
            .as_ref()
            .map_or_else(String::new, |value| value.markdown.clone());
        (drafts, preview_title, preview_markdown, conflict)
    };
    let selected = use_signal(|| None::<Draft>);
    let title = use_signal(move || preview_title.clone());
    let markdown = use_signal(move || preview_markdown.clone());
    let api = use_hook(ApiClient::new);
    rsx! {
        div { class: "grid gap-6 lg:grid-cols-[16rem_1fr]",
            DraftList {
                notebook_id,
                drafts: drafts.clone(),
                api: api.clone(),
                title,
                markdown,
                selected,
                context
            }
            DraftEditor {
                notebook_id,
                api,
                context,
                title,
                markdown,
                selected,
                conflict
            }
        }
    }
}

#[component]
fn DraftList(
    notebook_id: u64,
    drafts: Vec<DraftSummary>,
    api: ApiClient,
    mut title: Signal<String>,
    mut markdown: Signal<String>,
    mut selected: Signal<Option<Draft>>,
    context: Signal<WorkspaceContext>,
) -> Element {
    rsx! {
        aside { class: "rounded-lg border border-line bg-panel p-4",
            h2 { class: "mb-3 font-semibold", "Saved drafts" }
            if drafts.is_empty() {
                p { class: "text-sm text-ink-muted", "No saved drafts" }
            }
            for draft in drafts {
                div { class: "mb-2 flex items-start gap-2",
                    DraftButton {
                        notebook_id,
                        draft: draft.clone(),
                        api: api.clone(),
                        title,
                        markdown,
                        selected,
                        context
                    }
                    DeleteDraftButton {
                        notebook_id,
                        draft,
                        api: api.clone(),
                        title,
                        markdown,
                        selected,
                        context
                    }
                }
            }
        }
    }
}

#[component]
fn DeleteDraftButton(
    notebook_id: u64,
    draft: DraftSummary,
    api: ApiClient,
    mut title: Signal<String>,
    mut markdown: Signal<String>,
    mut selected: Signal<Option<Draft>>,
    context: Signal<WorkspaceContext>,
) -> Element {
    let draft_id = draft.draft_id;
    rsx! {
        button {
            class: "rounded border border-danger px-2 py-1 text-sm text-danger",
            aria_label: "Delete {draft.title}",
            onclick: move |_| {
                let api = api.clone();
                spawn(async move {
                    match api.delete_draft(notebook_id, draft_id, draft.revision).await {
                        Err(error) => set_alert(context, error),
                        Ok(()) => match api.drafts(notebook_id).await {
                            Err(error) => set_alert(context, error),
                            Ok(drafts) => {
                                let was_selected = selected
                                    .read()
                                    .as_ref()
                                    .is_some_and(|value| value.draft_id == draft_id);
                                let mut value = context.write();
                                value.model.drafts = LoadState::ready_or_empty(drafts);
                                value.model.alert = None;
                                value.model.status = "Draft deleted".into();
                                if was_selected {
                                    selected.set(None);
                                    title.set(String::new());
                                    markdown.set(String::new());
                                }
                            }
                        }
                    }
                });
            },
            "Delete"
        }
    }
}

#[component]
fn DraftEditor(
    notebook_id: u64,
    api: ApiClient,
    context: Signal<WorkspaceContext>,
    mut title: Signal<String>,
    mut markdown: Signal<String>,
    selected: Signal<Option<Draft>>,
    conflict: bool,
) -> Element {
    rsx! {
        section { class: "rounded-lg border border-line bg-panel p-5",
            h2 { class: "font-semibold text-ink-strong", "Draft editor" }
            if conflict {
                div {
                    class: "mb-4 rounded border border-conflict-line bg-conflict-bg p-3 text-conflict-ink",
                    p { "The saved revision changed; your editor contents are preserved." }
                    button {
                        class: "mt-2 underline",
                        onclick: move |_| {
                            if let Some(value) = selected.read().as_ref() {
                                title.set(value.title.clone());
                                markdown.set(value.markdown.clone());
                            }
                        },
                        "Reload saved revision"
                    }
                }
            }
            label {
                class: "mt-4 block text-sm font-medium",
                r#for: "draft-title",
                "Title"
            }
            input {
                id: "draft-title",
                class: "mt-1 w-full rounded border border-line bg-input px-3 py-2",
                value: "{title}",
                oninput: move |event| title.set(event.value())
            }
            label {
                class: "mt-4 block text-sm font-medium",
                r#for: "draft-markdown",
                "Markdown"
            }
            textarea {
                id: "draft-markdown",
                class: "mt-1 min-h-80 w-full rounded border border-line bg-input p-3 font-mono",
                value: "{markdown}",
                oninput: move |event| markdown.set(event.value())
            }
            SaveDraftButton {
                notebook_id,
                api,
                context,
                title,
                markdown,
                selected
            }
        }
    }
}

#[component]
fn SaveDraftButton(
    notebook_id: u64,
    api: ApiClient,
    context: Signal<WorkspaceContext>,
    title: Signal<String>,
    markdown: Signal<String>,
    selected: Signal<Option<Draft>>,
) -> Element {
    rsx! {
        button {
            class: "mt-4 rounded bg-accent px-4 py-2 text-white",
            onclick: move |_| {
                let draft_title = title.read().clone();
                let draft_markdown = markdown.read().clone();
                let selected_draft = selected.read().clone();
                let evidence_ids = selected_draft.as_ref().map_or_else(
                    || {
                        context
                            .read()
                            .model
                            .preview
                            .as_ref()
                            .map_or_else(Vec::new, |preview| preview.evidence_ids.clone())
                    },
                    |draft| {
                        draft
                            .citations
                            .iter()
                            .map(|citation| citation.evidence_id)
                            .collect()
                    }
                );
                let api = api.clone();
                let mut title_field = title;
                let mut markdown_field = markdown;
                let mut selected_field = selected;
                spawn(async move {
                    let result = match selected_draft {
                        Some(draft) => api
                            .update_draft(
                                notebook_id,
                                draft.draft_id,
                                &UpdateDraft {
                                    expected_revision: draft.revision,
                                    title: draft_title,
                                    markdown: draft_markdown,
                                    evidence_ids
                                }
                            )
                            .await,
                        None => api
                            .create_draft(
                                notebook_id,
                                &CreateDraft {
                                    title: draft_title,
                                    markdown: draft_markdown,
                                    evidence_ids
                                }
                            )
                            .await
                    };
                    match result {
                        Err(error) => set_alert(context, error),
                        Ok(saved) => match async {
                            let draft = api.draft(notebook_id, saved.draft_id).await?;
                            let drafts = api.drafts(notebook_id).await?;
                            Ok::<_, ClientError>((draft, drafts))
                        }
                        .await
                        {
                            Ok((draft, drafts)) => {
                                title_field.set(draft.title.clone());
                                markdown_field.set(draft.markdown.clone());
                                selected_field.set(Some(draft));
                                let mut value = context.write();
                                value.model.drafts = LoadState::ready_or_empty(drafts);
                                value.model.preview = None;
                                value.model.draft_previews.clear();
                                value.model.alert = None;
                                value.model.status = "Draft saved".into();
                            }
                            Err(error) => set_alert(context, error)
                        }
                    }
                });
            },
            "Save draft"
        }
    }
}

/// Whether the user is actively editing one of the draft editor fields.
///
/// On native targets (unit tests) no DOM exists, so the answer is `false`
/// and the signal-comparison guard remains the only protection.
fn editor_field_focused() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .is_some_and(|element| {
                let id = element.id();
                id == "draft-title" || id == "draft-markdown"
            })
    }
}

#[component]
pub fn DraftButton(
    notebook_id: u64,
    draft: DraftSummary,
    api: ApiClient,
    mut title: Signal<String>,
    mut markdown: Signal<String>,
    mut selected: Signal<Option<Draft>>,
    context: Signal<WorkspaceContext>,
) -> Element {
    let draft_id = draft.draft_id;
    // Snapshot the editor state at click time; a slower draft fetch must
    // not clobber edits the user makes while it is in flight.
    let prior_title = title.read().clone();
    let prior_markdown = markdown.read().clone();
    rsx! {
        button {
            class: "mb-2 block w-full rounded border border-line p-3 text-left",
            onclick: move |_| {
                let api = api.clone();
                let prior_title = prior_title.clone();
                let prior_markdown = prior_markdown.clone();
                spawn(async move {
                    match api.draft(notebook_id, draft_id).await {
                        Ok(value) => {
                            // Apply the fetched draft only when the user is
                            // not actively editing a field (the active
                            // element check) and has not changed the editor
                            // state since the click. A mid-fill rewrite by
                            // the controlled input would corrupt the value
                            // being typed.
                            if !editor_field_focused()
                                && title.read().as_str() == prior_title.as_str()
                                && markdown.read().as_str() == prior_markdown.as_str()
                            {
                                title.set(value.title.clone());
                                markdown.set(value.markdown.clone());
                            }
                            selected.set(Some(value));
                        }
                        Err(error) => {
                            let mut state = context.write();
                            state.model.alert = Some(error);
                            state.model.status = "Draft load failed".into();
                        }
                    }
                });
            },
            "{draft.title}"
            p { class: "text-sm text-ink-muted", "Revision {draft.revision}" }
        }
    }
}
