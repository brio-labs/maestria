use super::*;
pub(super) struct EnablePlan {
    root: PathBuf,
    startup_root: PathBuf,
    pub(super) previously_consented_root: Option<PathBuf>,
    program: PathBuf,
    profile_root: PathBuf,
    socket_path: PathBuf,
    revision: u64,
    previous_service: Option<SearchServiceConfig>,
    previous_managed: Option<ManagedSearchConfig>,
}

impl EnablePlan {
    pub(super) fn new(state: &LauncherState, selected_root: PathBuf) -> Result<Self, String> {
        let root = canonical_read_root(&selected_root)?;
        if root != selected_root {
            return Err(
                "The approved folder changed since selection; choose the folder again.".to_string(),
            );
        }
        state
            .settings()
            .map_err(|error| error.to_string())?
            .can_update_search_configuration()?;
        let settings = settings_snapshot(state)?;
        let profile_root = managed_profile_root()?;
        let socket_path = profile_root.join("system/daemon.sock");
        validate_socket_path(&socket_path)?;
        if settings
            .managed
            .as_ref()
            .is_some_and(|managed| managed.profile_root != profile_root)
        {
            return Err("The managed profile location changed; restore its original XDG data directory to continue safely.".to_string());
        }
        let previously_consented_root = settings
            .managed
            .as_ref()
            .filter(|managed| managed.enabled)
            .map(|managed| managed.root.clone());
        let startup_root = match previously_consented_root.as_ref() {
            Some(previous) => previous.clone(),
            None => root.clone(),
        };
        Ok(Self {
            root,
            startup_root,
            previously_consented_root,
            program: resolve_search_binary()?,
            profile_root,
            socket_path,
            revision: settings.revision,
            previous_service: settings.search,
            previous_managed: settings.managed,
        })
    }
}

impl SearchSetup {
    pub(super) fn prepare_enable_profile(
        &self,
        owned: &mut OwnedState,
        plan: &EnablePlan,
    ) -> Result<ProfileMarker, String> {
        if owned
            .profile_root
            .as_ref()
            .is_some_and(|profile| profile != &plan.profile_root)
        {
            return Err(
                "A different managed search profile is already owned by this launcher.".to_string(),
            );
        }
        if owned.child.is_none() && socket_path_exists(&plan.socket_path)? {
            return Err("A search service already owns the private managed socket, but this launcher does not own its process. Close that service before enabling managed search.".to_string());
        }
        let profile_created = ensure_profile_directory(&plan.profile_root)?;
        let existing = read_marker(&plan.profile_root)?;
        let is_new_marker = existing.is_none();
        let mut marker = match existing {
            Some(marker) => marker,
            None => {
                if !profile_created
                    || plan.previous_managed.is_some()
                    || unmarked_profile_has_state(&plan.profile_root)?
                {
                    return Err("The existing search profile has no launcher ownership marker; no profile data or service was changed.".to_string());
                }
                new_profile_marker()?
            }
        };
        validate_marker(&marker, plan.previous_managed.as_ref())?;
        validate_marker_credentials(&marker, &plan.profile_root)?;
        ensure_private_profile_dirs(&plan.profile_root)?;
        if owned.profile_lock.is_none() {
            owned.profile_lock = Some(acquire_profile_lock(&plan.profile_root)?);
        }
        if owned.child.is_none() && socket_path_exists(&plan.socket_path)? {
            return Err("A search service already owns the private managed socket, but this launcher does not own its process. Close that service before enabling managed search.".to_string());
        }
        let mut marker_changed = is_new_marker;
        if !marker.roots.contains(&plan.root) {
            if marker.pending_root.as_ref() != Some(&plan.root) {
                marker.pending_root = Some(plan.root.clone());
                marker_changed = true;
            }
            if manifest_is_missing(&plan.profile_root)? {
                marker.roots.push(plan.root.clone());
                marker_changed = true;
            }
        }
        if marker_changed {
            write_marker(&plan.profile_root, &marker)?;
        }
        owned.profile_root = Some(plan.profile_root.clone());
        owned.socket_path = Some(plan.socket_path.clone());
        owned.program = Some(plan.program.clone());
        owned.marker = Some(marker.clone());
        owned.pending = Some(PendingEnable {
            previous_service: plan.previous_service.clone(),
            previous_managed: plan.previous_managed.clone(),
            previous_revision: plan.revision,
            new_grant_file: None,
            new_grant_digest: None,
        });
        self.update_snapshot(|snapshot| {
            snapshot.root = Some(plan.root.clone());
            snapshot.enabled = plan
                .previous_managed
                .as_ref()
                .is_some_and(|saved| saved.enabled);
            snapshot.status = "Preparing approved folder".to_string();
            snapshot.detail = "Reconciling the private read-only search profile.".to_string();
        });
        Ok(marker)
    }

    pub(super) async fn retire_unconsented_child(
        &self,
        owned: &mut OwnedState,
        generation: u64,
    ) -> Result<(), String> {
        if owned.child.is_none() {
            return Ok(());
        }
        let deadline = MonotonicInstant::now() + SHUTDOWN_TIMEOUT;
        let cleanup = if owned.marker.is_some() {
            self.cleanup_owned_profile(owned, Some(generation), Some(deadline))
                .await
        } else {
            Ok(())
        };
        let stop = self.stop_owned_child(owned, deadline).await;
        owned.active = None;
        match (cleanup, stop) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(cleanup), Ok(())) => Err(cleanup),
            (Ok(()), Err(stop)) => Err(stop),
            (Err(cleanup), Err(stop)) => {
                Err(format!("{cleanup}; owned service stop failed: {stop}"))
            }
        }
    }

    pub(super) async fn run_enable_transaction(
        &self,
        state: &LauncherState,
        owned: &mut OwnedState,
        marker: &mut ProfileMarker,
        plan: &EnablePlan,
        generation: u64,
    ) -> Result<(), String> {
        self.check_intent(generation)?;
        initialize_profile_if_new(
            self,
            &plan.profile_root,
            &plan.startup_root,
            plan.previously_consented_root.as_deref(),
            &plan.program,
            generation,
        )
        .await?;
        if owned.child.is_none() {
            self.start_owned_service(
                owned,
                &plan.program,
                &plan.profile_root,
                &plan.socket_path,
                generation,
            )
            .await?;
        } else {
            self.ensure_live_endpoint(owned)?;
        }
        self.reconcile_selected_root(owned, marker, &plan.root, generation)
            .await?;
        let keep_digest = match plan.previous_managed.as_ref() {
            Some(previous) if previous.profile_identity == marker.profile_identity => {
                previous.grant_token_digest.as_str()
            }
            _ => "",
        };
        self.revoke_old_grants(owned, marker, keep_digest, generation)
            .await?;
        let grant = self
            .create_grant(owned, marker, &plan.root, generation)
            .await?;
        self.commit_enabled_search(state, owned, marker, plan, grant, generation)
            .await
    }

    async fn reconcile_selected_root(
        &self,
        owned: &mut OwnedState,
        marker: &mut ProfileMarker,
        root: &Path,
        generation: u64,
    ) -> Result<(), String> {
        let started_at = MonotonicInstant::now();
        loop {
            self.check_intent(generation)?;
            match self.owner_roots_status(owned, generation).await {
                Ok(status) => {
                    self.reconcile_roots(owned, marker, &status, root, generation)
                        .await?;
                    return Ok(());
                }
                Err(error) if started_at.elapsed() < STARTUP_TIMEOUT => {
                    if owned
                        .child
                        .as_mut()
                        .and_then(|child| child.try_wait().ok().flatten())
                        .is_some()
                    {
                        return Err(
                            "The managed Sillage search service exited during startup.".to_string()
                        );
                    }
                    self.update_snapshot(|snapshot| {
                        snapshot.status = "Starting read-only search".to_string();
                        snapshot.detail =
                            "Waiting for the private search service to become ready.".to_string();
                    });
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    if started_at.elapsed() >= STARTUP_TIMEOUT {
                        return Err(error);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn commit_enabled_search(
        &self,
        state: &LauncherState,
        owned: &mut OwnedState,
        marker: &mut ProfileMarker,
        plan: &EnablePlan,
        grant: OwnedGrant,
        generation: u64,
    ) -> Result<(), String> {
        let managed = ManagedSearchConfig {
            enabled: true,
            root: plan.root.clone(),
            profile_root: plan.profile_root.clone(),
            profile_identity: marker.profile_identity.clone(),
            grant_token_digest: grant.token_digest.clone(),
            grant_expires_at_unix_seconds: grant.expires_at_unix_seconds,
        };
        let service = SearchServiceConfig {
            socket_path: plan.socket_path.clone(),
            consumer_realm: marker.consumer_realm.clone(),
            credential_file: grant.credential_file,
            program: Some(plan.program.clone()),
        };
        self.check_intent(generation)?;
        state
            .settings()
            .map_err(|error| error.to_string())?
            .set_search_configuration_if_revision(
                plan.revision,
                Some(service.clone()),
                Some(managed.clone()),
            )?;
        owned.pending = None;
        owned.active = Some((service, managed.clone()));
        self.update_snapshot(|snapshot| {
            snapshot.enabled = true;
            snapshot.error = None;
        });
        self.refresh_owner_status(owned, &managed, generation)
            .await?;
        if plan
            .previous_managed
            .as_ref()
            .is_some_and(|previous| previous.profile_identity == managed.profile_identity)
        {
            self.revoke_old_grants(owned, marker, &managed.grant_token_digest, generation)
                .await?;
        }
        Ok(())
    }
}

fn new_profile_marker() -> Result<ProfileMarker, String> {
    Ok(ProfileMarker {
        schema_version: 1,
        profile_identity: random_hex(32)?,
        consumer_realm: random_hex(32)?,
        roots: Vec::new(),
        grants: Vec::new(),
        pending_root: None,
        pending_credential_file: None,
    })
}

fn manifest_is_missing(profile_root: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(profile_root.join("manifest.txt")) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err("Could not inspect the private search profile manifest.".to_string()),
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "enable_tests.rs"]
mod tests;
