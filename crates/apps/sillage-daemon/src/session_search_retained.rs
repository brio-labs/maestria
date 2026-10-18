use std::{
    ffi::{CString, OsStr},
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use sillage_domain::ContentHash;
use sillage_search_tantivy::RamMemoryBudget;

#[path = "session_search_retained_durable.rs"]
mod durable_store;
#[path = "session_search_retained_purge.rs"]
mod purge;
#[path = "session_search_retained_sources.rs"]
mod source_handles;
#[path = "session_search_retained_fs.rs"]
mod store_fs;

use self::store_fs::namespace::{
    ensure_owned_directory, open_owned_child, open_store_root, verify_marker,
};
use self::store_fs::{
    c_name, link_at, open_file_at, set_private_file_mode, unlink_at, verify_private_file,
    visit_directory_entries,
};

const STORE_MARKER: &str = "sillage-selected-source-store-v1\n";
const SOURCE_MARKER_PREFIX: &str = "sillage-selected-source-v1\n";
const STORE_MARKER_MAX_BYTES: usize = 256;
const STORE_DIRECTORY_MODE: u32 = 0o700;
const STORE_FILE_MODE: u32 = 0o600;

pub(in crate::session_search) use durable_store::{PersistedGeneration, RetainedDocumentaryStore};
pub(in crate::session_search) use purge::purge_source;
#[cfg(all(test, unix))]
pub(in crate::session_search) use purge::{
    PurgeTestAction, PurgeTestBoundary, arm_purge_test_action, clear_purge_test_action,
};
pub(in crate::session_search) use source_handles::{
    Admission, OwnershipPermit, RetainedBinding, RetainedSourceObject, RetainedSources,
};

fn hash_key(bytes: &[u8]) -> String {
    let hash = sillage_domain::content_hash(bytes);
    if let Some(key) = hash.strip_prefix("sha256:") {
        key.to_owned()
    } else {
        hash
    }
}

#[cfg(all(test, unix))]
#[path = "session_search_retained_tests.rs"]
mod tests;
