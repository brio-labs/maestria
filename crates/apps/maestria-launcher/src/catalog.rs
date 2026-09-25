use std::sync::{Arc, Mutex, MutexGuard};

use crate::errors::LauncherError;
use crate::model::{SearchStatus, SearchStatusKind};

/// Owned index data; native GIO objects never cross this boundary.
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

/// One usable application snapshot and at most one refresh plus one coalesced refresh.
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

    pub fn refresh_if_dirty(self: &Arc<Self>) -> Result<(), LauncherError> {
        self.start(false)
    }

    pub fn request_refresh(self: &Arc<Self>) -> Result<(), LauncherError> {
        self.start(true)
    }

    fn start(self: &Arc<Self>, force: bool) -> Result<(), LauncherError> {
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
                revision: previous.revision.saturating_add(1),
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

        let catalog = Arc::clone(self);
        std::thread::Builder::new()
            .name("maestria-launcher-catalog".to_string())
            .spawn(move || {
                loop {
                    let result = crate::platform::enumerate_apps();
                    let again = match catalog.state() {
                        Ok(mut state) => {
                            let previous = &state.snapshot;
                            let (apps, status) = match result {
                                Ok(apps) => {
                                    let warning = state.monitor_warning.clone().or_else(|| {
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
                                    (
                                        Arc::new(apps),
                                        SearchStatus {
                                            kind,
                                            message: warning,
                                        },
                                    )
                                }
                                Err(error) => (
                                    Arc::clone(&previous.apps),
                                    SearchStatus {
                                        kind: SearchStatusKind::Error,
                                        message: Some(error.message),
                                    },
                                ),
                            };
                            let again = state.dirty;
                            state.snapshot = Arc::new(CatalogSnapshot {
                                revision: previous.revision.saturating_add(1),
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
                    if !again {
                        return;
                    }
                }
            })
            .map_err(|error| {
                if let Ok(mut state) = self.state() {
                    state.running = false;
                    state.dirty = true;
                }
                LauncherError::platform_unavailable(format!(
                    "Application discovery could not start: {error}"
                ))
            })?;
        Ok(())
    }
}
