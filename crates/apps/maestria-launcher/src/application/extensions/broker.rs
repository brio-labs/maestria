use std::collections::BTreeMap;
use std::path::PathBuf;

use thiserror::Error;

mod desktop;
mod executor;
mod network;
mod search;
mod selected_file;
mod storage;
mod text;

pub use selected_file::SelectedFile;

const CODE_PERMISSION_DENIED: &str = "permission_denied";
const CODE_NOT_FOUND: &str = "not_found";
const CODE_INVALID_REQUEST: &str = "invalid_request";
const CODE_UNAVAILABLE: &str = "unavailable";
const CODE_FAILED: &str = "failed";

/// A search grant dedicated to one extension, never the launcher's own realm.
#[derive(Clone, Debug)]
pub struct SearchConsumerConfig {
    extension_id: String,
    consumer_realm: String,
    executable: PathBuf,
    socket_path: PathBuf,
    credential_file: PathBuf,
}

impl SearchConsumerConfig {
    /// The credential must live in the dedicated directory named for `extension_id`.
    pub fn for_extension(
        extension_id: impl Into<String>,
        consumer_realm: impl Into<String>,
        executable: PathBuf,
        socket_path: PathBuf,
        credential_file: PathBuf,
    ) -> Result<Self, BrokerConfigError> {
        let extension_id = extension_id.into();
        let consumer_realm = consumer_realm.into();
        let credential_owner = credential_file
            .parent()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str());
        if !valid_extension_id(&extension_id)
            || !valid_realm(&consumer_realm)
            || !executable.is_absolute()
            || !socket_path.is_absolute()
            || !credential_file.is_absolute()
            || credential_owner != Some(extension_id.as_str())
        {
            return Err(BrokerConfigError::SearchConfig);
        }
        Ok(Self {
            extension_id,
            consumer_realm,
            executable,
            socket_path,
            credential_file,
        })
    }
}

/// Trusted host paths and opaque file selections for a single active extension.
pub struct BrokerContext {
    extension_id: String,
    launcher_realm: Option<String>,
    storage_root: PathBuf,
    selected_files: BTreeMap<String, SelectedFile>,
    search_consumer: Option<SearchConsumerConfig>,
}

impl BrokerContext {
    pub fn new(
        extension_id: impl Into<String>,
        launcher_realm: Option<String>,
        storage_root: PathBuf,
    ) -> Result<Self, BrokerConfigError> {
        let extension_id = extension_id.into();
        if !valid_extension_id(&extension_id)
            || launcher_realm
                .as_deref()
                .is_some_and(|realm| !valid_realm(realm))
            || !storage_root.is_absolute()
        {
            return Err(BrokerConfigError::Context);
        }
        Ok(Self {
            extension_id,
            launcher_realm,
            storage_root,
            selected_files: BTreeMap::new(),
            search_consumer: None,
        })
    }

    pub fn with_selected_file(
        mut self,
        selection_id: String,
        file: SelectedFile,
    ) -> Result<Self, BrokerConfigError> {
        if !valid_opaque_id(&selection_id) || self.selected_files.contains_key(&selection_id) {
            return Err(BrokerConfigError::Selection);
        }
        self.selected_files.insert(selection_id, file);
        Ok(self)
    }

    pub fn with_search_consumer(
        mut self,
        config: SearchConsumerConfig,
    ) -> Result<Self, BrokerConfigError> {
        if config.extension_id != self.extension_id
            || self
                .launcher_realm
                .as_deref()
                .is_some_and(|realm| config.consumer_realm.eq_ignore_ascii_case(realm))
            || self.search_consumer.is_some()
        {
            return Err(BrokerConfigError::SearchConfig);
        }
        self.search_consumer = Some(config);
        Ok(self)
    }
}

/// Invalid host-side broker configuration. No capability request has run.
#[derive(Debug, Error)]
pub enum BrokerConfigError {
    #[error("invalid extension broker context")]
    Context,
    #[error("invalid per-extension search consumer configuration")]
    SearchConfig,
    #[error("invalid or duplicate host-selected file authorization")]
    Selection,
}

/// Executes authorized capability effects through trusted native adapters.
pub struct CapabilityBroker {
    context: BrokerContext,
}
impl CapabilityBroker {
    pub fn new(context: BrokerContext) -> Self {
        Self { context }
    }
}

fn valid_extension_id(id: &str) -> bool {
    id.len() <= 128
        && id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && id.split(['.', '-']).all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn valid_realm(realm: &str) -> bool {
    realm.len() == 64 && realm.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_opaque_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
