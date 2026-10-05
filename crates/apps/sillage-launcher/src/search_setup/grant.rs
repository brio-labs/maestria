use super::*;

#[derive(Debug)]
pub(super) struct GrantReceipt {
    token_digest: String,
    expires_at_unix_seconds: u64,
}

pub(super) fn parse_grant_receipt(output: &[u8]) -> Result<GrantReceipt, String> {
    let text = std::str::from_utf8(output)
        .map_err(|_| "Sillage search returned an invalid managed grant receipt.".to_string())?;
    let mut digest = None;
    let mut expiry = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("grant_token_digest=") {
            digest = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("expires_at_unix_seconds=") {
            expiry = value.trim().parse::<u64>().ok();
        }
    }
    let token_digest =
        digest.ok_or_else(|| "Sillage search did not confirm the managed grant.".to_string())?;
    if !is_hex_digest(&token_digest) {
        return Err("Sillage search returned an invalid managed grant identifier.".to_string());
    }
    let now = unix_time()?;
    let expires_at_unix_seconds = expiry
        .filter(|expiry| *expiry > now && *expiry <= now.saturating_add(GRANT_TTL_SECONDS + 60))
        .ok_or_else(|| "Sillage search returned an invalid managed grant expiry.".to_string())?;
    Ok(GrantReceipt {
        token_digest,
        expires_at_unix_seconds,
    })
}

pub(super) fn should_renew(managed: &ManagedSearchConfig) -> Result<bool, String> {
    Ok(managed.grant_expires_at_unix_seconds
        <= unix_time()?.saturating_add(GRANT_RENEWAL_WINDOW_SECONDS))
}

fn unix_time() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| "The system clock is before the Unix epoch.".to_string())
}

impl SearchSetup {
    pub(super) async fn create_grant(
        &self,
        owned: &mut OwnedState,
        marker: &mut ProfileMarker,
        root: &Path,
        generation: u64,
    ) -> Result<OwnedGrant, String> {
        let random = random_hex(8)?;
        let credential_file = owned
            .profile_root()?
            .join("system")
            .join(format!("launcher-search-{random}.credential"));
        marker.pending_root = Some(root.to_path_buf());
        marker.pending_credential_file = Some(credential_file.clone());
        write_marker(owned.profile_root()?, marker)?;
        owned.marker = Some(marker.clone());
        if let Some(pending) = owned.pending.as_mut() {
            pending.new_grant_file = Some(credential_file.clone());
        }
        let (program, profile) = owned.program_and_profile()?;
        let args = vec![
            "owner".into(),
            "grant".into(),
            "create-external".into(),
            "--instance-dir".into(),
            profile.as_os_str().to_os_string(),
            "--consumer-realm".into(),
            marker.consumer_realm.clone().into(),
            "--credential-file".into(),
            credential_file.as_os_str().to_os_string(),
            "--access".into(),
            "search-and-open-evidence".into(),
            "--max-sensitivity".into(),
            "internal".into(),
            "--max-results".into(),
            MAX_RESULTS.into(),
            "--max-evidence-bytes".into(),
            MAX_EVIDENCE_BYTES.into(),
            "--read-root".into(),
            path_argument(root)?,
            "--expires-in-seconds".into(),
            GRANT_TTL_SECONDS.to_string().into(),
        ];
        self.check_intent(generation)?;
        let output = self
            .cli(generation, &program, "owner grant create", args)
            .await?;
        self.ensure_live_endpoint(owned)?;
        let receipt = parse_grant_receipt(&output.stdout)?;
        if expected_token_digest(&credential_file)? != receipt.token_digest {
            return Err(
                "The managed grant credential did not match its private provenance.".to_string(),
            );
        }
        let grant = OwnedGrant {
            root: root.to_path_buf(),
            token_digest: receipt.token_digest,
            credential_file,
            expires_at_unix_seconds: receipt.expires_at_unix_seconds,
        };
        marker
            .grants
            .retain(|existing| existing.token_digest != grant.token_digest);
        marker.grants.push(grant.clone());
        marker.pending_root = None;
        marker.pending_credential_file = None;
        write_marker(owned.profile_root()?, marker)?;
        owned.marker = Some(marker.clone());
        if let Some(pending) = owned.pending.as_mut() {
            pending.new_grant_digest = Some(grant.token_digest.clone());
        }
        Ok(grant)
    }

    pub(super) async fn revoke_old_grants(
        &self,
        owned: &mut OwnedState,
        marker: &mut ProfileMarker,
        current_digest: &str,
        generation: u64,
    ) -> Result<(), String> {
        if marker.pending_root.is_some() || marker.pending_credential_file.is_some() {
            if let Some(file) = marker.pending_credential_file.clone() {
                if !valid_credential_path(owned.profile_root()?, &file) {
                    return Err("The launcher search marker contains an out-of-profile credential reference.".to_string());
                }
                match fs::symlink_metadata(&file) {
                    Ok(_) => {
                        let digest = expected_token_digest(&file)?;
                        if digest != current_digest {
                            self.revoke_grant(owned, &digest, generation).await?;
                            remove_private_file(owned.profile_root()?, &file)?;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => {
                        return Err(
                            "Could not inspect the pending private search credential.".to_string()
                        );
                    }
                }
            }
            marker.pending_root = None;
            marker.pending_credential_file = None;
            write_marker(owned.profile_root()?, marker)?;
            owned.marker = Some(marker.clone());
        }
        let obsolete = marker
            .grants
            .iter()
            .filter(|grant| grant.token_digest != current_digest)
            .cloned()
            .collect::<Vec<_>>();
        for grant in obsolete {
            self.revoke_grant(owned, &grant.token_digest, generation)
                .await?;
            remove_private_file(owned.profile_root()?, &grant.credential_file)?;
            marker
                .grants
                .retain(|known| known.token_digest != grant.token_digest);
            write_marker(owned.profile_root()?, marker)?;
            owned.marker = Some(marker.clone());
        }
        Ok(())
    }

    pub(super) async fn renew_grant(
        &self,
        state: &LauncherState,
        owned: &mut OwnedState,
        service: &SearchServiceConfig,
        managed: &ManagedSearchConfig,
        generation: u64,
        expected_revision: u64,
    ) -> Result<(), String> {
        let current = settings_snapshot(state)?;
        if current.revision != expected_revision
            || current.search.as_ref() != Some(service)
            || current.managed.as_ref() != Some(managed)
        {
            return Err(
                "Managed search consent changed before the grant could be renewed.".to_string(),
            );
        }
        owned.pending = Some(PendingEnable {
            previous_service: Some(service.clone()),
            previous_managed: Some(managed.clone()),
            previous_revision: expected_revision,
            new_grant_file: None,
            new_grant_digest: None,
        });
        let result = async {
            let mut marker = owned
                .marker
                .clone()
                .ok_or_else(|| "The managed search profile marker is unavailable.".to_string())?;
            let new_grant = self
                .create_grant(owned, &mut marker, &managed.root, generation)
                .await?;
            let new_service = SearchServiceConfig {
                socket_path: service.socket_path.clone(),
                consumer_realm: service.consumer_realm.clone(),
                credential_file: new_grant.credential_file.clone(),
                program: service.program.clone(),
            };
            let mut new_managed = managed.clone();
            new_managed.grant_token_digest = new_grant.token_digest.clone();
            new_managed.grant_expires_at_unix_seconds = new_grant.expires_at_unix_seconds;
            self.check_intent(generation)?;
            state
                .settings()
                .map_err(|error| error.to_string())?
                .set_search_configuration_if_revision(
                    expected_revision,
                    Some(new_service.clone()),
                    Some(new_managed.clone()),
                )?;
            owned.pending = None;
            owned.active = Some((new_service, new_managed));
            self.revoke_old_grants(owned, &mut marker, &new_grant.token_digest, generation)
                .await?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            if owned.pending.is_some() {
                let rollback = self
                    .rollback_pending(
                        owned,
                        Some(state),
                        Some(generation),
                        MonotonicInstant::now() + SHUTDOWN_TIMEOUT,
                    )
                    .await;
                if let Err(rollback_error) = rollback {
                    return Err(format!(
                        "{error}; managed search rollback could not be completed: {rollback_error}"
                    ));
                }
            }
            return Err(error);
        }
        Ok(())
    }

    pub(super) async fn revoke_grant(
        &self,
        owned: &mut OwnedState,
        digest: &str,
        generation: u64,
    ) -> Result<(), String> {
        self.revoke_grant_until(owned, digest, Some(generation), None)
            .await
    }

    pub(super) async fn revoke_grant_until(
        &self,
        owned: &mut OwnedState,
        digest: &str,
        generation: Option<u64>,
        deadline: Option<MonotonicInstant>,
    ) -> Result<(), String> {
        self.ensure_live_endpoint(owned)?;
        let (program, profile) = owned.program_and_profile()?;
        let args = owner_args(["grant", "revoke"], &profile, Some(digest.into()));
        self.run_owned_cli(
            owned,
            generation,
            deadline,
            &program,
            "owner grant revoke",
            args,
        )
        .await?;
        Ok(())
    }
}
