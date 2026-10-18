use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

use super::{AsyncMutex, SearchSetup, SearchSetupController};
use crate::application::{Frontend, FrontendModel};
use crate::ipc::LauncherState;
use crate::settings::SettingsManager;

fn controller() -> Result<Arc<SearchSetupController>, Box<dyn Error>> {
    let source = Arc::new(crate::application::source_choice::SessionSourceState::default());
    let (generation_updates, _) = tokio::sync::watch::channel(0_u64);
    let frontend = Arc::new(Frontend {
        source_choice: source,
        generation: std::sync::atomic::AtomicU64::new(0),
        active_search: std::sync::atomic::AtomicU64::new(0),
        generation_updates,
        interactive_search: tokio::sync::Mutex::new(()),
        model: std::sync::Mutex::new(FrontendModel {
            query: String::new(),
            passage_search_pending: false,
            accepted: Vec::new(),
            selected_file: None,
            catalog_ticks_until_refresh: crate::application::CATALOG_REFRESH_TICKS,
            accepted_passages: Vec::new(),
            accepted_paths: Vec::new(),
            passages_loaded: false,
            displayed: Vec::new(),
            result_filter: "all".to_string(),
            content_view_passages: Vec::new(),
        }),
    });
    let state = Arc::new(LauncherState::new(SettingsManager::load(Err(
        "route regression uses session-only settings".to_string(),
    )))?);
    Ok(Arc::new(SearchSetupController {
        backend: SearchSetup::new(),
        state,
        frontend,
        runtime: tokio::runtime::Handle::current(),
        ui: slint::Weak::default(),
        selected: std::sync::Mutex::new(None),
        picker_error: std::sync::Mutex::new(None),
        presentation: std::sync::Mutex::new(None),
        busy: std::sync::atomic::AtomicBool::new(false),
        stopping: std::sync::atomic::AtomicBool::new(false),
        presentation_revision: std::sync::atomic::AtomicU64::new(0),
        source_route_exclusive: std::sync::atomic::AtomicBool::new(false),
        folder_suspended: std::sync::atomic::AtomicBool::new(false),
        route_transition: AsyncMutex::new(()),
        restart_query: std::sync::atomic::AtomicBool::new(false),
        poll: std::sync::Mutex::new(None),
        action: std::sync::Mutex::new(None),
        view: std::sync::Mutex::new(None),
    }))
}

#[tokio::test]
async fn queued_folder_enable_after_source_suspension_is_rejected() -> Result<(), Box<dyn Error>> {
    let controller = controller()?;
    let root = PathBuf::from("/tmp/queued-folder-enable-regression");
    let queued_enable = controller.enable_in_current_route(root.clone());

    controller
        .suspend_for_source_choice()
        .await
        .map_err(std::io::Error::other)?;
    let suspended_snapshot = controller.backend.snapshot();
    let enabled = queued_enable.await;

    match enabled {
        Err(_) => {}
        Ok(()) => return Err(std::io::Error::other("queued folder enable was admitted").into()),
    }
    let snapshot = controller.backend.snapshot();
    assert_eq!(snapshot, suspended_snapshot);
    assert_ne!(snapshot.root.as_ref(), Some(&root));
    assert!(!snapshot.enabled);
    assert!(!snapshot.busy);
    Ok(())
}
