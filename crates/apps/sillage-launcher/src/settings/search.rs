use super::{ManagedSearchConfig, SearchServiceConfig, SettingsManager};

impl SettingsManager {
    pub(crate) fn search_service(&self) -> Option<SearchServiceConfig> {
        self.values.search.clone()
    }

    pub(crate) fn has_search_service(&self) -> bool {
        self.values.search.is_some()
    }

    pub(crate) fn managed_search(&self) -> Option<ManagedSearchConfig> {
        self.values.managed_search.clone()
    }

    pub(crate) fn search_revision(&self) -> u64 {
        self.search_revision
    }

    pub(crate) fn can_update_search_configuration(&self) -> Result<(), String> {
        if self.read_only {
            return Err("Preferences from an unknown schema version are read-only".to_string());
        }
        if self.preserve_existing {
            return Err(
                "The existing preferences file was preserved; confirm Reset before changing managed search"
                    .to_string(),
            );
        }
        if self.path.is_none() {
            return Err("Managed search requires a persistent preferences directory".to_string());
        }
        Ok(())
    }

    pub(crate) fn set_search_configuration_if_revision(
        &mut self,
        expected_revision: u64,
        search: Option<SearchServiceConfig>,
        managed_search: Option<ManagedSearchConfig>,
    ) -> Result<(), String> {
        self.can_update_search_configuration()?;
        if self.search_revision != expected_revision {
            return Err(
                "Search preferences changed while managed setup was in progress".to_string(),
            );
        }
        validate_search_configuration(search.as_ref(), managed_search.as_ref())?;
        let previous_search = std::mem::replace(&mut self.values.search, search);
        let previous_managed = std::mem::replace(&mut self.values.managed_search, managed_search);
        if let Err(error) = self.persist() {
            self.values.search = previous_search;
            self.values.managed_search = previous_managed;
            return Err(error);
        }
        self.search_revision = self.search_revision.saturating_add(1);
        Ok(())
    }
}

pub(super) fn validate_search_configuration(
    search: Option<&SearchServiceConfig>,
    managed: Option<&ManagedSearchConfig>,
) -> Result<(), String> {
    if let Some(search) = search
        && (!search.socket_path.is_absolute()
            || !search.credential_file.is_absolute()
            || search.consumer_realm.len() != 64
            || !search
                .consumer_realm
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || search
                .program
                .as_ref()
                .is_some_and(|path| !path.is_absolute()))
    {
        return Err(
            "Search requires absolute paths and a 64-character hexadecimal consumer realm"
                .to_string(),
        );
    }
    if let Some(managed) = managed {
        let Some(search) = search else {
            return Err(
                "Managed search consent requires a search service configuration".to_string(),
            );
        };
        if !managed.enabled
            || !managed.root.is_absolute()
            || !managed.profile_root.is_absolute()
            || managed.profile_identity.len() != 64
            || !managed
                .profile_identity
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || managed.grant_token_digest.len() != 64
            || !managed
                .grant_token_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || managed.grant_expires_at_unix_seconds == 0
            || search.program.is_none()
            || search.socket_path != managed.profile_root.join("system/daemon.sock")
            || !search.credential_file.starts_with(&managed.profile_root)
        {
            return Err("Managed search provenance is incomplete or inconsistent".to_string());
        }
    }
    Ok(())
}
