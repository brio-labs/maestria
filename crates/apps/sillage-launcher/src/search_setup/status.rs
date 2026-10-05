use super::*;

impl SearchSetup {
    pub(super) async fn finish_operation(
        &self,
        result: Result<(), String>,
        state: &LauncherState,
    ) -> Result<(), String> {
        match result {
            Ok(()) => {
                let settings = settings_snapshot(state).ok();
                self.update_snapshot(|snapshot| {
                    snapshot.busy = false;
                    if let Some(settings) = settings.as_ref() {
                        if settings
                            .managed
                            .as_ref()
                            .is_some_and(|managed| managed.enabled)
                        {
                            snapshot.enabled = true;
                            snapshot.root = settings
                                .managed
                                .as_ref()
                                .map(|managed| managed.root.clone());
                        } else if !snapshot.enabled {
                            snapshot.root = None;
                        }
                    }
                });
                Ok(())
            }
            Err(error) => {
                let settings = settings_snapshot(state).ok();
                let current_root = settings.as_ref().and_then(|settings| {
                    settings
                        .managed
                        .as_ref()
                        .map(|managed| managed.root.clone())
                });
                let is_enabled = settings
                    .as_ref()
                    .and_then(|settings| settings.managed.as_ref())
                    .is_some_and(|managed| managed.enabled);
                let safe = sanitize_error(
                    &error,
                    settings
                        .as_ref()
                        .and_then(|settings| settings.search.as_ref()),
                    settings
                        .as_ref()
                        .and_then(|settings| settings.managed.as_ref()),
                );
                self.mark_error(current_root, is_enabled, safe.clone());
                Err(safe)
            }
        }
    }
    pub(crate) async fn refresh(&self, state: &LauncherState) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        let generation = self.current_intent();
        let mut owned = self.operation.lock().await;
        if self.shutdown_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        let settings = settings_snapshot(state)?;
        let revision = settings.revision;
        match (settings.managed, settings.search) {
            (None, Some(_)) => {
                self.publish_status(
                    true,
                    None,
                    "External configured",
                    "Search is configured outside launcher-managed setup; provider availability has not been checked and it will not be started or stopped here.",
                    None,
                );
                Ok(())
            }
            (None, None) => self.cleanup_unconsented_owned(&mut owned, generation).await,
            (Some(managed), Some(service)) if managed.enabled => {
                let result: Result<(), String> = async {
                    self.validate_owned_config(&mut owned, &service, &managed)?;
                    self.ensure_live_endpoint(&mut owned)?;
                    self.refresh_owner_status(&mut owned, &managed, generation)
                        .await?;
                    let marker = owned.marker.as_ref().ok_or_else(|| {
                        "The managed search profile marker is unavailable.".to_string()
                    })?;
                    let needs_grant_cleanup = marker.pending_root.is_some()
                        || marker.pending_credential_file.is_some()
                        || marker
                            .grants
                            .iter()
                            .any(|grant| grant.token_digest != managed.grant_token_digest);
                    if needs_grant_cleanup {
                        let mut marker = owned.marker.clone().ok_or_else(|| {
                            "The managed search profile marker is unavailable.".to_string()
                        })?;
                        self.revoke_old_grants(
                            &mut owned,
                            &mut marker,
                            &managed.grant_token_digest,
                            generation,
                        )
                        .await?;
                    }
                    if should_renew(&managed)? {
                        self.renew_grant(
                            state, &mut owned, &service, &managed, generation, revision,
                        )
                        .await?;
                    }
                    Ok(())
                }
                .await;
                match result {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        let safe = sanitize_error(&error, Some(&service), Some(&managed));
                        self.mark_error(Some(managed.root), true, safe.clone());
                        Err(safe)
                    }
                }
            }
            (Some(_), Some(_)) => self.cleanup_unconsented_owned(&mut owned, generation).await,
            (Some(managed), None) if !managed.enabled => {
                self.cleanup_unconsented_owned(&mut owned, generation).await
            }
            (Some(managed), None) => {
                let error = "Managed search preferences are incomplete; no service was started."
                    .to_string();
                self.mark_error(Some(managed.root), false, error.clone());
                Err(error)
            }
        }
    }

    async fn cleanup_unconsented_owned(
        &self,
        owned: &mut OwnedState,
        generation: u64,
    ) -> Result<(), String> {
        let deadline = MonotonicInstant::now() + SHUTDOWN_TIMEOUT;
        let has_resources = owned.marker.as_ref().is_some_and(|marker| {
            !marker.grants.is_empty()
                || !marker.roots.is_empty()
                || marker.pending_root.is_some()
                || marker.pending_credential_file.is_some()
        });
        let cleanup = if owned.child.is_some() && owned.marker.is_some() {
            self.cleanup_owned_profile(owned, Some(generation), Some(deadline))
                .await
                .err()
        } else if has_resources || owned.child.is_some() {
            Some("Managed owner authority cleanup is deferred.".to_string())
        } else {
            None
        };
        let stop = if owned.child.is_some() {
            self.stop_owned_child(owned, deadline).await
        } else {
            Ok(())
        };
        owned.active = None;
        if stop.is_err() && owned.child.is_some() {
            let error =
                "The launcher-owned search service did not stop before the five-second deadline."
                    .to_string();
            self.mark_error(None, false, error.clone());
            return Err(error);
        }
        if cleanup.is_some() {
            let error = "Managed search authority cleanup is deferred until its launcher-owned service is available.".to_string();
            self.mark_error(None, false, error.clone());
            return Err(error);
        }
        if stop.is_err() {
            let error = "The launcher-owned process stopped, but its socket path could not be confirmed absent.".to_string();
            self.mark_error(None, false, error.clone());
            return Err(error);
        }
        self.publish_status(
            false,
            None,
            "Search is off",
            "Applications remain available without document search.",
            None,
        );
        Ok(())
    }

    pub(super) async fn refresh_owner_status(
        &self,
        owned: &mut OwnedState,
        managed: &ManagedSearchConfig,
        generation: u64,
    ) -> Result<(), String> {
        let status = self.owner_roots_status(owned, generation).await?;
        if status.roots.len() != 1 || !same_path(&status.roots[0].path, &managed.root) {
            let deadline = MonotonicInstant::now() + SHUTDOWN_TIMEOUT;
            if owned.marker.is_some() {
                let _ = self
                    .cleanup_owned_profile(owned, Some(generation), Some(deadline))
                    .await;
            }
            let stop = self.stop_owned_child(owned, deadline).await;
            owned.active = None;
            if stop.is_err() && owned.child.is_some() {
                return Err("The live managed search service exceeded the approved-folder scope and did not stop before the five-second deadline.".to_string());
            }
            return Err("The live managed search service exceeded the approved-folder scope; the launcher-owned process was stopped.".to_string());
        }
        let root = &status.roots[0];
        let has_indexing_error = status
            .indexing
            .last_error
            .as_deref()
            .is_some_and(|error| !error.trim().is_empty());
        let detail = format!(
            "{} files indexed · {} pending{}",
            root.indexed_file_count,
            status.indexing.pending_file_count,
            if root.excluded_file_count == 0 {
                String::new()
            } else {
                format!(" · {} excluded", root.excluded_file_count)
            }
        );
        let ready = !status.indexing.scanning
            && status.indexing.pending_file_count == 0
            && status.indexing.last_scan_unix_ms.is_some()
            && !has_indexing_error;
        let error = has_indexing_error.then(|| "Sillage reported an indexing issue.".to_string());
        let label = if has_indexing_error {
            "Indexing issue"
        } else if ready {
            "Ready"
        } else {
            "Indexing"
        };
        self.publish_status(true, Some(managed.root.clone()), label, &detail, error);
        Ok(())
    }

    fn publish_status(
        &self,
        enabled: bool,
        root: Option<PathBuf>,
        status: &str,
        detail: &str,
        error: Option<String>,
    ) {
        let current = self.snapshot();
        if current.enabled == enabled
            && !current.busy
            && current.root == root
            && current.status == status
            && current.detail == detail
            && current.error == error
        {
            return;
        }
        self.update_snapshot(|snapshot| {
            snapshot.enabled = enabled;
            snapshot.busy = false;
            snapshot.root = root;
            snapshot.status = status.to_string();
            snapshot.detail = detail.to_string();
            snapshot.error = error;
        });
    }
}
