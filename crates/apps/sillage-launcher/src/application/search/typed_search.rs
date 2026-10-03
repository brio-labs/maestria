use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::super::window::show_notice;
use super::super::{Frontend, UiWeak, lock};
use crate::ipc::LauncherState;

pub(in crate::application) fn start_catalog_search(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: &tokio::runtime::Handle,
    ui: UiWeak,
    query: String,
    generation: u64,
) -> tokio::sync::oneshot::Receiver<()> {
    let (applied, wait_for_application) = tokio::sync::oneshot::channel::<()>();
    runtime.spawn(async move {
        let response = state.search(query, generation).await;
        let _ = slint::invoke_from_event_loop(move || {
            if frontend.generation.load(Ordering::Acquire) == generation
                && let Some(window) = ui.upgrade()
            {
                match response {
                    Ok(response) => super::apply_search_response(&window, &frontend, response),
                    Err(error) => super::apply_search_error(&window, &frontend, error.message),
                }
                let _ = applied.send(());
            }
        });
    });
    wait_for_application
}

pub(in crate::application) fn apply_passage_result(
    frontend: Arc<Frontend>,
    ui: UiWeak,
    generation: u64,
    query: String,
    search_result: Option<super::super::passages::PassageSearchResult>,
) -> tokio::sync::oneshot::Receiver<()> {
    let (applied, wait_for_application) = tokio::sync::oneshot::channel::<()>();
    let _ = slint::invoke_from_event_loop(move || {
        if frontend.generation.load(Ordering::Acquire) == generation
            && let Some(window) = ui.upgrade()
        {
            if let Some(search_result) = search_result {
                super::apply_passages(&window, &frontend, generation, &query, search_result, false);
            } else {
                window.set_index_status("Document search unavailable".into());
            }
        }
        drop(applied);
    });
    wait_for_application
}

pub(in crate::application) fn finish_active_search(frontend: &Frontend, generation: u64) {
    let _ =
        frontend
            .active_search
            .compare_exchange(generation, 0, Ordering::AcqRel, Ordering::Acquire);
}

pub(in crate::application) struct TypedSearch {
    pub(in crate::application) generation: u64,
    pub(in crate::application) has_search_service: bool,
    pub(in crate::application) catalog_applied: tokio::sync::oneshot::Receiver<()>,
}

/// Serialize shared-realm searches and cancel requests when a newer UI search starts.
pub(in crate::application) async fn search_passages(
    frontend: &Frontend,
    state: &LauncherState,
    generation: u64,
    query: &str,
    ui: &UiWeak,
) -> Option<super::super::passages::PassageSearchResult> {
    let mut generation_updates = frontend.generation_updates.subscribe();
    let _interactive_search =
        acquire_interactive_search_slot(frontend, generation, &mut generation_updates).await?;

    // Resolve the authorized service after waiting for the shared request slot.
    let config = match state.settings() {
        Ok(settings) => settings.search_service(),
        Err(error) => {
            show_notice(ui, error.message);
            return None;
        }
    }?;
    tokio::select! {
        biased;
        _ = generation_updates.changed() => None,
        result = super::super::passages::search(config, query) => result,
    }
}

async fn acquire_interactive_search_slot<'a>(
    frontend: &'a Frontend,
    generation: u64,
    generation_updates: &mut tokio::sync::watch::Receiver<u64>,
) -> Option<tokio::sync::MutexGuard<'a, ()>> {
    let guard = tokio::select! {
        biased;
        _ = generation_updates.changed() => return None,
        guard = frontend.interactive_search.lock() => guard,
    };
    (frontend.generation.load(Ordering::Acquire) == generation).then_some(guard)
}

pub(in crate::application) fn advance_search_generation(frontend: &Frontend) -> Option<u64> {
    let previous =
        match frontend
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1)
            }) {
            Ok(previous) => previous,
            Err(_) => return None,
        };
    let generation = previous + 1;
    frontend.generation_updates.send_replace(generation);
    Some(generation)
}

pub(in crate::application) fn start_typed_search(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    query: String,
) -> Option<TypedSearch> {
    frontend.active_search.store(0, Ordering::Release);
    let generation = advance_search_generation(&frontend)?;
    let has_search_service = match state.settings() {
        Ok(settings) => settings.search_service().is_some(),
        Err(error) => {
            show_notice(&ui, error.message);
            false
        }
    };
    let catalog_applied = start_catalog_search(
        state,
        Arc::clone(&frontend),
        &runtime,
        ui,
        query,
        generation,
    );
    Some(TypedSearch {
        generation,
        has_search_service,
        catalog_applied,
    })
}

pub(in crate::application) fn take_pending_passage_search(
    frontend: &Frontend,
) -> Option<(String, u64)> {
    let mut model = lock(&frontend.model);
    if !model.passage_search_pending {
        return None;
    }
    model.passage_search_pending = false;
    Some((
        model.query.clone(),
        frontend.generation.load(Ordering::Acquire),
    ))
}

pub(in crate::application) fn start_typed_passage_search(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    query: String,
    generation: u64,
    catalog_applied: tokio::sync::oneshot::Receiver<()>,
) {
    if query.is_empty() {
        finish_active_search(&frontend, generation);
        return;
    }
    frontend.active_search.store(generation, Ordering::Release);
    runtime.spawn(async move {
        let mut generation_updates = frontend.generation_updates.subscribe();
        let search_result = search_passages(&frontend, &state, generation, &query, &ui).await;
        if frontend.generation.load(Ordering::Acquire) != generation {
            finish_active_search(&frontend, generation);
            return;
        }
        // The catalog apply resets passage rows, so only the RPC may run in parallel.
        if !wait_for_catalog_application(
            &frontend,
            generation,
            &mut generation_updates,
            catalog_applied,
        )
        .await
        {
            finish_active_search(&frontend, generation);
            return;
        }
        if frontend.generation.load(Ordering::Acquire) != generation {
            finish_active_search(&frontend, generation);
            return;
        }
        let applied =
            apply_passage_result(Arc::clone(&frontend), ui, generation, query, search_result);
        let _ = applied.await;
        finish_active_search(&frontend, generation);
    });
}

async fn wait_for_catalog_application(
    frontend: &Frontend,
    generation: u64,
    generation_updates: &mut tokio::sync::watch::Receiver<u64>,
    catalog_applied: tokio::sync::oneshot::Receiver<()>,
) -> bool {
    tokio::select! {
        biased;
        _ = generation_updates.changed() => false,
        result = catalog_applied => match result {
            Ok(()) => frontend.generation.load(Ordering::Acquire) == generation,
            Err(_) => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_frontend(generation: u64, query: &str, passage_search_pending: bool) -> Frontend {
        let (generation_updates, _) = tokio::sync::watch::channel(generation);
        Frontend {
            generation: std::sync::atomic::AtomicU64::new(generation),
            active_search: std::sync::atomic::AtomicU64::new(0),
            generation_updates,
            interactive_search: tokio::sync::Mutex::new(()),
            model: std::sync::Mutex::new(super::super::super::FrontendModel {
                query: query.to_string(),
                passage_search_pending,
                accepted: Vec::new(),
                selected_file: None,
                catalog_ticks_until_refresh: super::super::super::CATALOG_REFRESH_TICKS,
                accepted_passages: Vec::new(),
                accepted_paths: Vec::new(),
                passages_loaded: false,
                displayed: Vec::new(),
                result_filter: "all".to_string(),
                content_view_passages: Vec::new(),
            }),
        }
    }

    #[test]
    fn pending_passage_timer_takes_only_the_latest_generation_once()
    -> Result<(), Box<dyn std::error::Error>> {
        let frontend = test_frontend(1, "first", true);
        let cancelled_generation = frontend.generation_updates.subscribe();
        let stale_generation = frontend.generation.load(Ordering::Acquire);
        {
            let mut model = lock(&frontend.model);
            model.query = "latest".to_string();
            model.passage_search_pending = true;
        }

        let latest_generation = advance_search_generation(&frontend)
            .ok_or("search generation unexpectedly exhausted")?;
        assert!(cancelled_generation.has_changed()?);
        assert_eq!(*cancelled_generation.borrow(), latest_generation);
        assert_ne!(latest_generation, stale_generation);
        assert_eq!(
            take_pending_passage_search(&frontend),
            Some(("latest".to_string(), latest_generation))
        );
        assert_eq!(take_pending_passage_search(&frontend), None);
        Ok(())
    }

    #[tokio::test]
    async fn superseded_search_drops_while_waiting_for_the_shared_slot() {
        let frontend = test_frontend(1, "stale", false);
        let _held_slot = frontend.interactive_search.lock().await;
        let mut generation_updates = frontend.generation_updates.subscribe();
        let pending = acquire_interactive_search_slot(&frontend, 1, &mut generation_updates);
        tokio::pin!(pending);
        tokio::task::yield_now().await;

        assert_eq!(advance_search_generation(&frontend), Some(2));
        match tokio::time::timeout(std::time::Duration::from_millis(100), &mut pending).await {
            Ok(acquired) => assert!(acquired.is_none()),
            Err(_) => panic!("generation cancellation must release a queued search"),
        }
    }

    #[tokio::test]
    async fn superseded_catalog_gate_drops_passage_result() {
        let frontend = test_frontend(1, "stale", false);
        let mut generation_updates = frontend.generation_updates.subscribe();
        let (_catalog_applied, wait_for_catalog) = tokio::sync::oneshot::channel();
        let waiting =
            wait_for_catalog_application(&frontend, 1, &mut generation_updates, wait_for_catalog);
        tokio::pin!(waiting);

        assert_eq!(advance_search_generation(&frontend), Some(2));
        assert!(!waiting.await);
    }
}
