use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sillage_domain::GrantTokenDigest;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, watch};
use tokio::time::{Instant as MonotonicInstant, timeout};

use crate::ipc::LauncherState;
use crate::settings::{ManagedSearchConfig, SearchServiceConfig};
mod consent;
mod enable;
mod grant;
mod owner;
mod profile;
mod provenance;
mod resume;
mod rollback;
mod service;
mod status;

use enable::*;
use grant::*;
use owner::*;
use profile::*;
use provenance::*;
use service::*;

const PROFILE_COMPONENTS: [&str; 2] = ["sillage", "launcher-search"];
const MARKER_FILE: &str = "launcher-owner.json";
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(4);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const GRANT_TTL_SECONDS: u64 = 86_400;
const GRANT_RENEWAL_WINDOW_SECONDS: u64 = 300;
const MAX_RESULTS: &str = "100";
const MAX_EVIDENCE_BYTES: &str = "65536";
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetupSnapshot {
    pub(crate) enabled: bool,
    pub(crate) busy: bool,
    pub(crate) root: Option<PathBuf>,
    pub(crate) status: String,
    pub(crate) detail: String,
    pub(crate) error: Option<String>,
}

impl Default for SetupSnapshot {
    fn default() -> Self {
        Self {
            enabled: false,
            busy: false,
            root: None,
            status: "Search is off".to_string(),
            detail: "Applications remain available without document search.".to_string(),
            error: None,
        }
    }
}

/// Owns only the standalone Sillage search process started by this launcher.
pub(crate) struct SearchSetup {
    operation: Mutex<OwnedState>,
    snapshot: StdMutex<Arc<SetupSnapshot>>,
    intent: watch::Sender<u64>,
    intent_counter: AtomicU64,
    shutdown_requested: AtomicBool,
}

impl SearchSetup {
    pub(crate) fn new() -> Self {
        let (intent, _) = watch::channel(0);
        Self {
            operation: Mutex::new(OwnedState::default()),
            snapshot: StdMutex::new(Arc::new(SetupSnapshot::default())),
            intent,
            intent_counter: AtomicU64::new(0),
            shutdown_requested: AtomicBool::new(false),
        }
    }

    pub(crate) fn snapshot(&self) -> Arc<SetupSnapshot> {
        let snapshot = match self.snapshot.lock() {
            Ok(snapshot) => snapshot,
            Err(poisoned) => poisoned.into_inner(),
        };
        Arc::clone(&snapshot)
    }

    pub(crate) async fn enable(&self, state: &LauncherState, root: PathBuf) -> Result<(), String> {
        let generation = self.begin_intent();
        self.update_snapshot(|snapshot| {
            snapshot.busy = true;
            snapshot.error = None;
            if !snapshot.enabled {
                snapshot.status = "Preparing search".to_string();
                snapshot.detail = "The approved folder will be indexed locally.".to_string();
            }
        });

        let result = self.enable_inner(state, root, generation).await;
        self.finish_operation(result, state).await
    }

    pub(crate) async fn resume(&self, state: &LauncherState) -> Result<(), String> {
        let generation = self.begin_intent();
        self.update_snapshot(|snapshot| {
            snapshot.busy = true;
            snapshot.error = None;
        });
        let result = self.resume_inner(state, generation).await;
        self.finish_operation(result, state).await
    }

    fn begin_intent(&self) -> u64 {
        let generation = self
            .intent_counter
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        self.intent.send_replace(generation);
        generation
    }

    fn current_intent(&self) -> u64 {
        self.intent_counter.load(Ordering::Acquire)
    }

    fn check_intent(&self, generation: u64) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::Acquire)
            || self.intent_counter.load(Ordering::Acquire) != generation
        {
            Err("Managed search setup was cancelled by a newer user action.".to_string())
        } else {
            Ok(())
        }
    }

    fn update_snapshot(&self, update: impl FnOnce(&mut SetupSnapshot)) {
        let mut current = match self.snapshot.lock() {
            Ok(snapshot) => snapshot,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut next = (**current).clone();
        update(&mut next);
        if next != **current {
            *current = Arc::new(next);
        }
    }

    fn mark_error(&self, root: Option<PathBuf>, enabled: bool, error: String) {
        self.update_snapshot(|snapshot| {
            snapshot.busy = false;
            snapshot.enabled = enabled;
            if root.is_some() {
                snapshot.root = root;
            }
            snapshot.status = if enabled {
                "Search needs attention".to_string()
            } else {
                "Search unavailable".to_string()
            };
            snapshot.detail = error.clone();
            snapshot.error = Some(error);
        });
    }
}

#[derive(Default)]
struct OwnedState {
    child: Option<Child>,
    marker: Option<ProfileMarker>,
    profile_root: Option<PathBuf>,
    socket_path: Option<PathBuf>,
    program: Option<PathBuf>,
    socket_identity: Option<SocketIdentity>,
    profile_lock: Option<File>,
    active: Option<(SearchServiceConfig, ManagedSearchConfig)>,
    pending: Option<PendingEnable>,
}

impl OwnedState {
    fn profile_root(&self) -> Result<&Path, String> {
        self.profile_root
            .as_deref()
            .ok_or_else(|| "The managed search profile is unavailable.".to_string())
    }

    fn program_and_profile(&self) -> Result<(PathBuf, PathBuf), String> {
        Ok((
            self.program
                .clone()
                .ok_or_else(|| "The Sillage search executable is unavailable.".to_string())?,
            self.profile_root()?.to_path_buf(),
        ))
    }
}

#[derive(Debug, Clone)]
struct PendingEnable {
    previous_service: Option<SearchServiceConfig>,
    previous_managed: Option<ManagedSearchConfig>,
    previous_revision: u64,
    new_grant_file: Option<PathBuf>,
    new_grant_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProfileMarker {
    schema_version: u32,
    profile_identity: String,
    consumer_realm: String,
    roots: Vec<PathBuf>,
    grants: Vec<OwnedGrant>,
    pending_root: Option<PathBuf>,
    pending_credential_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct OwnedGrant {
    root: PathBuf,
    token_digest: String,
    credential_file: PathBuf,
    expires_at_unix_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct RootsStatus {
    roots: Vec<RootStatus>,
    indexing: IndexingStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct RootStatus {
    path: String,
    indexed_file_count: usize,
    excluded_file_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct IndexingStatus {
    scanning: bool,
    pending_file_count: usize,
    last_scan_unix_ms: Option<u64>,
    last_error: Option<String>,
}

struct CommandOutput {
    stdout: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone)]
struct SettingsSnapshot {
    search: Option<SearchServiceConfig>,
    managed: Option<ManagedSearchConfig>,
    revision: u64,
}

fn settings_snapshot(state: &LauncherState) -> Result<SettingsSnapshot, String> {
    let settings = state.settings().map_err(|error| error.to_string())?;
    Ok(SettingsSnapshot {
        search: settings.search_service(),
        managed: settings.managed_search(),
        revision: settings.search_revision(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_length_guard_matches_unix_path_capacity() {
        #[cfg(target_os = "linux")]
        {
            let capacity = sun_path_capacity();
            let accepted = PathBuf::from(format!("/{}", "x".repeat(capacity - 2)));
            assert!(validate_socket_path(&accepted).is_ok());
            let rejected = PathBuf::from(format!("/{}", "x".repeat(capacity - 1)));
            assert!(validate_socket_path(&rejected).is_err());
        }
    }

    #[test]
    fn grant_receipt_rejects_unbounded_expiry() {
        let output = format!(
            "grant_token_digest={}\nexpires_at_unix_seconds={}\n",
            "a".repeat(64),
            u64::MAX
        );
        assert!(parse_grant_receipt(output.as_bytes()).is_err());
    }
}
