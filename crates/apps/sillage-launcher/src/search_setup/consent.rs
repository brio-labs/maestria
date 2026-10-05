use super::*;
impl SearchSetup {
    pub(crate) async fn disable(&self, state: &LauncherState) -> Result<(), String> {
        let deadline = MonotonicInstant::now() + SHUTDOWN_TIMEOUT;
        let generation = self.begin_intent();
        self.update_snapshot(|snapshot| {
            snapshot.busy = true;
            snapshot.error = None;
            snapshot.status = "Turning search off".to_string();
        });
        let settings = match settings_snapshot(state) {
            Ok(settings) => settings,
            Err(_) => {
                let error =
                    "Launcher search preferences could not be read before disable.".to_string();
                self.mark_error(None, false, error.clone());
                return Err(error);
            }
        };
        let is_external = settings.managed.is_none() && settings.search.is_some();
        if settings.managed.is_some() || settings.search.is_some() {
            let cleared = state
                .settings()
                .map_err(|_| "Launcher search preferences could not be cleared.".to_string())
                .and_then(|mut preferences| {
                    preferences.set_search_configuration_if_revision(settings.revision, None, None)
                });
            if cleared.is_err() {
                let error = "Launcher search preferences could not be cleared; the previous configuration was retained.".to_string();
                self.mark_error(
                    settings
                        .managed
                        .as_ref()
                        .map(|managed| managed.root.clone()),
                    true,
                    error.clone(),
                );
                return Err(error);
            }
        }
        if is_external {
            self.update_snapshot(|snapshot| {
                snapshot.enabled = false;
                snapshot.busy = false;
                snapshot.root = None;
                snapshot.status = "Disconnected external".to_string();
                snapshot.detail = "The launcher disconnected from the external search service; its provider was not stopped or modified.".to_string();
                snapshot.error = None;
            });
            return Ok(());
        }
        let managed = settings.managed;
        let remaining = deadline.saturating_duration_since(MonotonicInstant::now());
        let mut owned = match timeout(remaining, self.operation.lock()).await {
            Ok(owned) => owned,
            Err(_) => {
                let error = "Search settings were cleared locally; owned authority cleanup is deferred while another operation finishes.".to_string();
                self.update_snapshot(|snapshot| {
                    snapshot.enabled = false;
                    snapshot.busy = false;
                    snapshot.root = None;
                    snapshot.status = "Search disabled locally".to_string();
                    snapshot.detail = error.clone();
                    snapshot.error = Some(error.clone());
                });
                return Err(error);
            }
        };
        self.disable_inner(&mut owned, managed, generation, deadline)
            .await
    }

    /// Signals cancellation before waiting for the serialized owner operation.
    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        self.shutdown_requested.store(true, Ordering::Release);
        self.begin_intent();
        let deadline = MonotonicInstant::now() + SHUTDOWN_TIMEOUT;
        self.update_snapshot(|snapshot| {
            snapshot.busy = true;
            snapshot.status = "Stopping managed search".to_string();
            snapshot.detail = "The read-only search process is shutting down.".to_string();
        });
        let remaining = deadline.saturating_duration_since(MonotonicInstant::now());
        let mut owned = match timeout(remaining, self.operation.lock()).await {
            Ok(owned) => owned,
            Err(_) => {
                let error = "Managed search did not stop within five seconds.".to_string();
                let snapshot = self.snapshot();
                self.mark_error(snapshot.root.clone(), snapshot.enabled, error.clone());
                return Err(error);
            }
        };

        let rollback = if owned.pending.is_some() {
            self.rollback_pending(&mut owned, None, None, deadline)
                .await
        } else {
            Ok(())
        };
        let stop = if owned.child.is_some() {
            self.stop_owned_child(&mut owned, deadline).await
        } else {
            Ok(())
        };
        if owned.child.is_none() {
            owned.profile_lock = None;
        }
        if let Some(error) = rollback.err().or_else(|| stop.err()) {
            self.mark_error(
                self.snapshot().root.clone(),
                self.snapshot().enabled,
                error.clone(),
            );
            return Err(error);
        }
        self.update_snapshot(|snapshot| {
            snapshot.busy = false;
            if !snapshot.enabled {
                snapshot.status = "Search is off".to_string();
            }
        });
        Ok(())
    }
    pub(super) async fn enable_inner(
        &self,
        state: &LauncherState,
        selected_root: PathBuf,
        generation: u64,
    ) -> Result<(), String> {
        let plan = EnablePlan::new(state, selected_root)?;
        let mut owned = self.operation.lock().await;
        self.check_intent(generation)?;
        if owned.pending.is_some() {
            self.rollback_pending(
                &mut owned,
                Some(state),
                None,
                MonotonicInstant::now() + SHUTDOWN_TIMEOUT,
            )
            .await?;
        }
        if plan.previously_consented_root.is_none() {
            self.retire_unconsented_child(&mut owned, generation)
                .await?;
        }
        let mut marker = self.prepare_enable_profile(&mut owned, &plan)?;
        let result = self
            .run_enable_transaction(state, &mut owned, &mut marker, &plan, generation)
            .await;
        if let Err(error) = result {
            if owned.pending.is_some() {
                let rollback = self
                    .rollback_pending(
                        &mut owned,
                        Some(state),
                        None,
                        MonotonicInstant::now() + SHUTDOWN_TIMEOUT,
                    )
                    .await;
                if let Err(rollback_error) = rollback {
                    return Err(format!(
                        "{error}; rollback could not be completed: {rollback_error}"
                    ));
                }
            }
            return Err(error);
        }
        owned.pending = None;
        Ok(())
    }

    pub(super) async fn resume_inner(
        &self,
        state: &LauncherState,
        generation: u64,
    ) -> Result<(), String> {
        let settings = settings_snapshot(state)?;
        let revision = settings.revision;
        match (settings.managed, settings.search) {
            (None, Some(_)) => {
                self.update_snapshot(|snapshot| {
                    snapshot.enabled = true;
                    snapshot.busy = false;
                    snapshot.root = None;
                    snapshot.status = "External configured".to_string();
                    snapshot.detail = "Search is configured outside launcher-managed setup; provider availability has not been checked here.".to_string();
                    snapshot.error = None;
                });
                Ok(())
            }
            (None, None) => {
                let mut owned = self.operation.lock().await;
                if owned.child.is_some() {
                    self.retire_unconsented_child(&mut owned, generation)
                        .await?;
                } else if owned.marker.as_ref().is_some_and(|marker| {
                    !marker.grants.is_empty()
                        || !marker.roots.is_empty()
                        || marker.pending_root.is_some()
                        || marker.pending_credential_file.is_some()
                }) {
                    return Err("Managed search authority cleanup is deferred until its launcher-owned service is available.".to_string());
                }
                self.show_search_off(
                    None,
                    "Applications remain available without document search.",
                );
                Ok(())
            }
            (Some(managed), Some(_)) if !managed.enabled => {
                self.show_search_off(
                    Some(managed.root),
                    "Previously approved search remains disabled.",
                );
                Ok(())
            }
            (Some(_), None) => {
                let error = "Managed search preferences are incomplete; no service was started."
                    .to_string();
                self.mark_error(None, false, error.clone());
                Err(error)
            }
            (Some(managed), Some(service)) => {
                self.resume_managed(state, service, managed, revision, generation)
                    .await
            }
        }
    }

    pub(super) async fn stop_after_consent_loss(
        &self,
        owned: &mut OwnedState,
        generation: u64,
    ) -> Result<(), String> {
        let deadline = MonotonicInstant::now() + SHUTDOWN_TIMEOUT;
        let cleanup = if owned.child.is_some() && owned.marker.is_some() {
            self.cleanup_owned_profile(owned, Some(generation), Some(deadline))
                .await
        } else {
            Ok(())
        };
        let stop = if owned.child.is_some() {
            self.stop_owned_child(owned, deadline).await
        } else {
            Ok(())
        };
        self.update_snapshot(|snapshot| {
            snapshot.enabled = false;
            snapshot.busy = false;
            snapshot.root = None;
            snapshot.status = "Search is off".to_string();
            snapshot.detail = "Persisted consent changed; managed search was stopped.".to_string();
            snapshot.error = None;
        });
        let message = "Persisted managed search consent changed during startup.".to_string();
        if let Err(error) = cleanup {
            return Err(format!(
                "{message} Managed authority cleanup failed: {error}"
            ));
        }
        if let Err(error) = stop {
            return Err(format!(
                "{message} The owned search process did not stop: {error}"
            ));
        }
        Err(message)
    }

    pub(super) async fn disable_inner(
        &self,
        owned: &mut OwnedState,
        managed: Option<ManagedSearchConfig>,
        generation: u64,
        deadline: MonotonicInstant,
    ) -> Result<(), String> {
        let was_running = owned.child.is_some();
        let cleanup = if owned.child.is_some() && owned.marker.is_some() {
            self.cleanup_owned_profile(owned, Some(generation), Some(deadline))
                .await
        } else if managed.is_some() || owned.marker.is_some() {
            Err(
                "Managed owner authority cleanup is deferred while its service is offline."
                    .to_string(),
            )
        } else {
            Ok(())
        };
        let stop = if owned.child.is_some() {
            self.stop_owned_child(owned, deadline).await
        } else {
            Ok(())
        };
        owned.active = None;
        if stop.is_err() && owned.child.is_some() {
            let error = if cleanup.is_err() {
                "Search settings were cleared locally, but the launcher-owned service did not stop before the five-second deadline; authority cleanup is also deferred.".to_string()
            } else {
                "Search settings were cleared locally, but the launcher-owned service did not stop before the five-second deadline.".to_string()
            };
            self.publish_disabled_warning(&error);
            return Err(error);
        }
        let socket_warning = stop.is_err();
        if cleanup.is_err() || socket_warning {
            let detail = if cleanup.is_err() && was_running {
                "Search settings were cleared and the launcher-owned process was stopped; authority cleanup is deferred until its service is available."
            } else if cleanup.is_err() {
                "Search settings were cleared locally; managed authority cleanup is deferred while its service is offline."
            } else {
                "Search settings were cleared and the launcher-owned process stopped; socket absence could not be confirmed, so no endpoint was removed."
            };
            let error = if cleanup.is_err() {
                "Managed owner authority cleanup is deferred; no external service was contacted."
            } else {
                "The owned process stopped, but its socket path could not be confirmed absent."
            };
            self.update_snapshot(|snapshot| {
                snapshot.enabled = false;
                snapshot.busy = false;
                snapshot.root = None;
                snapshot.status = "Search disabled locally".to_string();
                snapshot.detail = detail.to_string();
                snapshot.error = Some(error.to_string());
            });
            return Ok(());
        }
        self.show_search_off(
            None,
            "Applications remain available without document search.",
        );
        Ok(())
    }
}
