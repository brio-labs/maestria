use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;

use tauri::{AppHandle, Emitter, Manager};

use crate::errors::LauncherError;
use crate::ipc::LauncherState;
use crate::model::{LAUNCHER_WINDOW_LABEL, SearchStatus, SearchStatusKind};

#[derive(Clone, Serialize)]
struct CatalogChangedEvent {
    revision: u64,
}

/// Owned index data; native GIO and GTK objects never cross this boundary.
#[derive(Debug, Clone)]
pub struct AppEntry {
    pub desktop_id: String,
    pub name: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub icon_descriptor: Option<String>,
    pub desktop_file: String,
    pub normalized_name: String,
    pub normalized_description: String,
    pub normalized_keywords: String,
}

impl AppEntry {
    pub fn new(
        desktop_id: String,
        name: String,
        description: String,
        keywords: Vec<String>,
        icon_descriptor: Option<String>,
        desktop_file: String,
    ) -> Self {
        let normalized_name = name.to_lowercase();
        let normalized_description = description.to_lowercase();
        let normalized_keywords = keywords.join(" ").to_lowercase();
        Self {
            desktop_id,
            name,
            description,
            keywords,
            icon_descriptor,
            desktop_file,
            normalized_name,
            normalized_description,
            normalized_keywords,
        }
    }
}

pub struct CatalogSnapshot {
    pub revision: u64,
    pub apps: Arc<Vec<AppEntry>>,
    pub status: SearchStatus,
}

struct CatalogState {
    snapshot: Arc<CatalogSnapshot>,
    dirty: bool,
    running: bool,
    monitor_warning: Option<String>,
}

/// One usable snapshot and at most one refresh plus one coalesced pending refresh.
pub struct Catalog {
    state: Mutex<CatalogState>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            state: Mutex::new(CatalogState {
                snapshot: Arc::new(CatalogSnapshot {
                    revision: 0,
                    apps: Arc::new(Vec::new()),
                    status: SearchStatus {
                        kind: SearchStatusKind::Loading,
                        message: None,
                    },
                }),
                dirty: true,
                running: false,
                monitor_warning: None,
            }),
        }
    }
}

impl Catalog {
    fn state(&self) -> Result<MutexGuard<'_, CatalogState>, LauncherError> {
        self.state.lock().map_err(|_| {
            LauncherError::platform_unavailable("The application catalog is unavailable")
        })
    }

    pub fn snapshot(&self) -> Result<Arc<CatalogSnapshot>, LauncherError> {
        Ok(Arc::clone(&self.state()?.snapshot))
    }

    pub fn mark_dirty(&self) -> Result<(), LauncherError> {
        self.state()?.dirty = true;
        Ok(())
    }

    pub fn monitor_failed(&self, error: LauncherError) -> Result<(), LauncherError> {
        self.state()?.monitor_warning = Some(error.message);
        Ok(())
    }

    pub fn refresh_if_dirty(self: &Arc<Self>, app: &AppHandle) -> Result<(), LauncherError> {
        self.start(app, false)
    }

    pub fn request_refresh(self: &Arc<Self>, app: &AppHandle) -> Result<(), LauncherError> {
        self.start(app, true)
    }

    fn start(self: &Arc<Self>, app: &AppHandle, force: bool) -> Result<(), LauncherError> {
        {
            let mut state = self.state()?;
            state.dirty |= force;
            if state.running || !state.dirty {
                return Ok(());
            }
            state.running = true;
            state.dirty = false;
            let previous = &state.snapshot;
            state.snapshot = Arc::new(CatalogSnapshot {
                revision: previous.revision + 1,
                apps: Arc::clone(&previous.apps),
                status: SearchStatus {
                    kind: if previous.revision == 0 {
                        SearchStatusKind::Loading
                    } else {
                        SearchStatusKind::Refreshing
                    },
                    message: None,
                },
            });
        }
        self.notify_visible(app);
        let catalog = Arc::clone(self);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let result = tauri::async_runtime::spawn_blocking(crate::platform::enumerate_apps)
                    .await
                    .map_err(|error| {
                        LauncherError::platform_unavailable(format!(
                            "Application discovery failed: {error}"
                        ))
                    })
                    .and_then(|result| result);
                let visible = is_visible(&app);
                let again = match catalog.state() {
                    Ok(mut state) => {
                        let previous = &state.snapshot;
                        let (apps, status) = match result {
                            Ok(apps) => {
                                let message = state.monitor_warning.clone().or_else(|| {
                                    apps.is_empty().then(|| {
                                        "No applications found. Built-in commands are available."
                                            .to_string()
                                    })
                                });
                                let kind = if state.monitor_warning.is_some() {
                                    SearchStatusKind::Error
                                } else {
                                    SearchStatusKind::Ready
                                };
                                (Arc::new(apps), SearchStatus { kind, message })
                            }
                            Err(error) => (
                                Arc::clone(&previous.apps),
                                SearchStatus {
                                    kind: SearchStatusKind::Error,
                                    message: Some(error.message),
                                },
                            ),
                        };
                        let again = state.dirty && visible;
                        state.snapshot = Arc::new(CatalogSnapshot {
                            revision: previous.revision + 1,
                            apps,
                            status,
                        });
                        state.running = again;
                        if again {
                            state.dirty = false;
                        }
                        again
                    }
                    Err(_) => return,
                };
                catalog.notify_visible(&app);
                if !again {
                    return;
                }
            }
        });
        Ok(())
    }
    fn notify_visible(&self, app: &AppHandle) {
        if is_visible(app)
            && let Ok(snapshot) = self.snapshot()
        {
            let _ = app.emit_to(
                LAUNCHER_WINDOW_LABEL,
                "launcher://catalog-changed",
                CatalogChangedEvent {
                    revision: snapshot.revision,
                },
            );
        }
    }
}

fn is_visible(app: &AppHandle) -> bool {
    app.get_webview_window(LAUNCHER_WINDOW_LABEL)
        .is_some_and(|window| matches!(window.is_visible(), Ok(true)))
}

pub fn changed(app: &AppHandle) {
    let state = app.state::<LauncherState>();
    if state.catalog().mark_dirty().is_ok() && is_visible(app) {
        let _ = state.catalog().refresh_if_dirty(app);
    }
}
