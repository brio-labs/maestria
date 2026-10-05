use super::*;
impl SearchSetup {
    pub(super) fn show_search_off(&self, root: Option<PathBuf>, detail: &str) {
        self.update_snapshot(|snapshot| {
            snapshot.enabled = false;
            snapshot.busy = false;
            snapshot.root = root;
            snapshot.status = "Search is off".to_string();
            snapshot.detail = detail.to_string();
            snapshot.error = None;
        });
    }
    pub(super) fn publish_disabled_warning(&self, detail: &str) {
        self.update_snapshot(|snapshot| {
            snapshot.enabled = false;
            snapshot.busy = false;
            snapshot.root = None;
            snapshot.status = "Search disabled locally".to_string();
            snapshot.detail = detail.to_string();
            snapshot.error = Some(detail.to_string());
        });
    }

    pub(super) async fn resume_managed(
        &self,
        state: &LauncherState,
        service: SearchServiceConfig,
        managed: ManagedSearchConfig,
        revision: u64,
        generation: u64,
    ) -> Result<(), String> {
        self.update_snapshot(|snapshot| {
            snapshot.enabled = true;
            snapshot.root = Some(managed.root.clone());
            snapshot.status = "Resuming approved search".to_string();
            snapshot.detail = "Starting the read-only Sillage search service.".to_string();
        });
        let mut owned = self.operation.lock().await;
        self.check_intent(generation)?;
        self.require_saved_consent(state, &service, &managed, Some(revision))?;
        if owned.child.is_none() && socket_path_exists(&service.socket_path)? {
            return Err("A search service already owns the saved managed socket, but this launcher does not own its process. Close that service before resuming managed search.".to_string());
        }
        self.validate_owned_config(&mut owned, &service, &managed)?;
        self.lock_resume_profile(&mut owned, &managed)?;
        self.start_saved_service(&mut owned, &service, &managed, generation)
            .await?;
        self.wait_for_saved_service(state, &mut owned, &service, &managed, revision, generation)
            .await?;
        self.finish_saved_resume(state, &mut owned, &service, &managed, revision, generation)
            .await
    }

    fn require_saved_consent(
        &self,
        state: &LauncherState,
        service: &SearchServiceConfig,
        managed: &ManagedSearchConfig,
        revision: Option<u64>,
    ) -> Result<(), String> {
        if ensure_saved_managed_search(state, service, managed, revision).is_err() {
            return Err(
                "Persisted managed search consent changed before service startup.".to_string(),
            );
        }
        Ok(())
    }

    fn lock_resume_profile(
        &self,
        owned: &mut OwnedState,
        managed: &ManagedSearchConfig,
    ) -> Result<(), String> {
        if owned.profile_lock.is_none() {
            owned.profile_lock = Some(acquire_profile_lock(&managed.profile_root)?);
        } else if owned
            .profile_root
            .as_ref()
            .is_some_and(|root| root != &managed.profile_root)
        {
            return Err(
                "A different managed search profile is already owned by this launcher.".to_string(),
            );
        }
        Ok(())
    }

    async fn start_saved_service(
        &self,
        owned: &mut OwnedState,
        service: &SearchServiceConfig,
        managed: &ManagedSearchConfig,
        generation: u64,
    ) -> Result<(), String> {
        if owned.child.is_some() {
            return self.ensure_live_endpoint(owned);
        }
        let program = service.program.as_deref().ok_or_else(|| {
            "The saved managed search binary is missing from preferences.".to_string()
        })?;
        validate_manifest_roots(&managed.profile_root, Some(&managed.root))?;
        self.start_owned_service(
            owned,
            program,
            &managed.profile_root,
            &service.socket_path,
            generation,
        )
        .await
    }

    async fn wait_for_saved_service(
        &self,
        state: &LauncherState,
        owned: &mut OwnedState,
        service: &SearchServiceConfig,
        managed: &ManagedSearchConfig,
        revision: u64,
        generation: u64,
    ) -> Result<(), String> {
        let started_at = MonotonicInstant::now();
        loop {
            if self
                .require_saved_consent(state, service, managed, Some(revision))
                .is_err()
            {
                return self.stop_after_consent_loss(owned, generation).await;
            }
            let status = self.refresh_owner_status(owned, managed, generation).await;
            if self
                .require_saved_consent(state, service, managed, Some(revision))
                .is_err()
            {
                return self.stop_after_consent_loss(owned, generation).await;
            }
            match status {
                Ok(()) => return Ok(()),
                Err(error) if started_at.elapsed() < STARTUP_TIMEOUT => {
                    if owned.child.is_none()
                        || owned
                            .child
                            .as_mut()
                            .and_then(|child| child.try_wait().ok().flatten())
                            .is_some()
                    {
                        return Err(
                            "The managed Sillage search service exited or stopped during startup."
                                .to_string(),
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    if started_at.elapsed() >= STARTUP_TIMEOUT {
                        return Err(error);
                    }
                }
                Err(error) => return Err(error),
            }
            self.check_intent(generation)?;
        }
    }

    async fn finish_saved_resume(
        &self,
        state: &LauncherState,
        owned: &mut OwnedState,
        service: &SearchServiceConfig,
        managed: &ManagedSearchConfig,
        revision: u64,
        generation: u64,
    ) -> Result<(), String> {
        self.require_saved_consent(state, service, managed, Some(revision))?;
        let mut marker = owned
            .marker
            .clone()
            .ok_or_else(|| "The launcher-owned search profile marker is missing.".to_string())?;
        self.revoke_old_grants(owned, &mut marker, &managed.grant_token_digest, generation)
            .await?;
        self.require_saved_consent(state, service, managed, Some(revision))?;
        if should_renew(managed)? {
            self.renew_grant(state, owned, service, managed, generation, revision)
                .await?;
        }
        let (active_service, active_managed) = owned.active.as_ref().ok_or_else(|| {
            "The resumed managed search configuration is unavailable.".to_string()
        })?;
        self.require_saved_consent(state, active_service, active_managed, None)
    }
}
fn ensure_saved_managed_search(
    state: &LauncherState,
    service: &SearchServiceConfig,
    managed: &ManagedSearchConfig,
    expected_revision: Option<u64>,
) -> Result<(), String> {
    let current = settings_snapshot(state)?;
    if expected_revision.is_some_and(|revision| revision != current.revision)
        || current.search.as_ref() != Some(service)
        || current.managed.as_ref() != Some(managed)
        || !managed.enabled
    {
        return Err("Persisted managed search consent changed.".to_string());
    }
    Ok(())
}
