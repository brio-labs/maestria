use super::*;
impl SearchSetup {
    pub(super) async fn rollback_pending(
        &self,
        owned: &mut OwnedState,
        state: Option<&LauncherState>,
        generation: Option<u64>,
        deadline: MonotonicInstant,
    ) -> Result<(), String> {
        let Some(pending) = owned.pending.clone() else {
            return Ok(());
        };
        let restore_previous = match state {
            Some(state) => match settings_snapshot(state) {
                Ok(current) => {
                    current.revision == pending.previous_revision
                        && current.search == pending.previous_service
                        && current.managed == pending.previous_managed
                        && pending
                            .previous_managed
                            .as_ref()
                            .is_some_and(|managed| managed.enabled)
                }
                Err(error) => {
                    if owned.child.is_some() {
                        let _ = self.stop_owned_child(owned, deadline).await;
                        owned.active = None;
                    }
                    return Err(error);
                }
            },
            None => false,
        };
        let result = if owned.child.is_some() {
            self.rollback_live_pending(owned, &pending, restore_previous, generation, deadline)
                .await
        } else {
            self.rollback_offline_pending(owned, &pending)
        };
        if let Err(error) = result {
            if owned.child.is_some() {
                let _ = self.stop_owned_child(owned, deadline).await;
                owned.active = None;
            }
            return Err(error);
        }
        owned.pending = None;
        if !restore_previous && owned.child.is_some() {
            let stopped = self.stop_owned_child(owned, deadline).await;
            owned.active = None;
            return stopped;
        }
        Ok(())
    }

    async fn rollback_live_pending(
        &self,
        owned: &mut OwnedState,
        pending: &PendingEnable,
        restore_previous: bool,
        generation: Option<u64>,
        deadline: MonotonicInstant,
    ) -> Result<(), String> {
        self.ensure_live_endpoint(owned)?;
        let mut marker = owned
            .marker
            .clone()
            .ok_or_else(|| "The launcher search ownership marker is unavailable.".to_string())?;
        self.rollback_pending_grant(owned, pending, &mut marker, generation, deadline)
            .await?;
        if !restore_previous {
            self.rollback_prior_grants(owned, &mut marker, generation, deadline)
                .await?;
        }
        let desired_root = restore_previous
            .then(|| {
                pending
                    .previous_managed
                    .as_ref()
                    .map(|managed| managed.root.clone())
            })
            .flatten();
        self.rollback_roots(
            owned,
            &mut marker,
            desired_root.as_deref(),
            generation,
            deadline,
        )
        .await?;
        marker.pending_root = None;
        marker.pending_credential_file = None;
        if !restore_previous {
            marker.roots.clear();
        }
        write_marker(owned.profile_root()?, &marker)?;
        owned.marker = Some(marker);
        Ok(())
    }

    async fn rollback_pending_grant(
        &self,
        owned: &mut OwnedState,
        pending: &PendingEnable,
        marker: &mut ProfileMarker,
        generation: Option<u64>,
        deadline: MonotonicInstant,
    ) -> Result<(), String> {
        let credential = pending
            .new_grant_file
            .as_ref()
            .or(marker.pending_credential_file.as_ref())
            .cloned();
        let digest = pending.new_grant_digest.clone().or_else(|| {
            credential
                .as_ref()
                .and_then(|file| expected_token_digest(file).ok())
        });
        if let Some(digest) = digest.as_deref() {
            self.revoke_grant_until(owned, digest, generation, Some(deadline))
                .await?;
            marker.grants.retain(|grant| grant.token_digest != digest);
        }
        if let Some(file) = credential.as_ref() {
            remove_private_file(owned.profile_root()?, file)?;
        }
        Ok(())
    }

    async fn rollback_prior_grants(
        &self,
        owned: &mut OwnedState,
        marker: &mut ProfileMarker,
        generation: Option<u64>,
        deadline: MonotonicInstant,
    ) -> Result<(), String> {
        for grant in marker.grants.clone() {
            self.revoke_grant_until(owned, &grant.token_digest, generation, Some(deadline))
                .await?;
            remove_private_file(owned.profile_root()?, &grant.credential_file)?;
            marker
                .grants
                .retain(|known| known.token_digest != grant.token_digest);
        }
        Ok(())
    }

    async fn rollback_roots(
        &self,
        owned: &mut OwnedState,
        marker: &mut ProfileMarker,
        desired_root: Option<&Path>,
        generation: Option<u64>,
        deadline: MonotonicInstant,
    ) -> Result<(), String> {
        let status = self
            .owner_roots_status_until(owned, generation, Some(deadline))
            .await?;
        for root in status
            .roots
            .iter()
            .map(|root| PathBuf::from(&root.path))
            .filter(|root| Some(root.as_path()) != desired_root)
        {
            if marker.roots.contains(&root) {
                self.remove_root_until(owned, &root, generation, Some(deadline))
                    .await?;
                marker.roots.retain(|known| known != &root);
            }
        }
        if let Some(root) = desired_root {
            let status = self
                .owner_roots_status_until(owned, generation, Some(deadline))
                .await?;
            if !status
                .roots
                .iter()
                .any(|approved| same_path(&approved.path, root))
            {
                let (program, profile) = owned.program_and_profile()?;
                let args = owner_args(
                    ["roots", "add"],
                    &profile,
                    Some(root.as_os_str().to_os_string()),
                );
                self.run_owned_cli(
                    owned,
                    generation,
                    Some(deadline),
                    &program,
                    "owner roots add",
                    args,
                )
                .await?;
            }
            if !marker.roots.iter().any(|known| known.as_path() == root) {
                marker.roots.push(root.to_path_buf());
            }
        }
        Ok(())
    }

    fn rollback_offline_pending(
        &self,
        owned: &mut OwnedState,
        pending: &PendingEnable,
    ) -> Result<(), String> {
        let pending_file = pending.new_grant_file.clone().or_else(|| {
            owned
                .marker
                .as_ref()
                .and_then(|marker| marker.pending_credential_file.clone())
        });
        if let Some(file) = pending_file {
            remove_private_file(owned.profile_root()?, &file)?;
        }
        if let Some(mut marker) = owned.marker.clone() {
            marker.pending_root = None;
            marker.pending_credential_file = None;
            write_marker(owned.profile_root()?, &marker)?;
            owned.marker = Some(marker);
        }
        Ok(())
    }
    pub(super) fn validate_owned_config(
        &self,
        owned: &mut OwnedState,
        service: &SearchServiceConfig,
        managed: &ManagedSearchConfig,
    ) -> Result<(), String> {
        if !managed.enabled
            || service.socket_path != managed.profile_root.join("system/daemon.sock")
            || service
                .program
                .as_ref()
                .is_none_or(|program| !program.is_absolute())
            || !service.credential_file.starts_with(&managed.profile_root)
            || managed.profile_root != managed_profile_root()?
        {
            return Err(
                "Saved search settings do not identify the private launcher-managed profile."
                    .to_string(),
            );
        }
        if let Some(profile) = owned.profile_root.as_ref()
            && profile != &managed.profile_root
        {
            return Err(
                "A different managed search profile is already owned by this launcher.".to_string(),
            );
        }
        let marker = read_marker(&managed.profile_root)?
            .ok_or_else(|| "The launcher-owned search profile marker is missing.".to_string())?;
        validate_marker(&marker, Some(managed))?;
        validate_marker_credentials(&marker, &managed.profile_root)?;
        if marker.consumer_realm != service.consumer_realm
            || !marker.grants.iter().any(|grant| {
                grant.token_digest == managed.grant_token_digest
                    && grant.root == managed.root
                    && grant.credential_file == service.credential_file
            })
            || expected_token_digest(&service.credential_file)? != managed.grant_token_digest
        {
            return Err(
                "Saved search grant provenance does not match the private profile.".to_string(),
            );
        }
        owned.profile_root = Some(managed.profile_root.clone());
        owned.socket_path = Some(service.socket_path.clone());
        owned.program = service.program.clone();
        owned.marker = Some(marker);
        owned.active = Some((service.clone(), managed.clone()));
        Ok(())
    }
}
