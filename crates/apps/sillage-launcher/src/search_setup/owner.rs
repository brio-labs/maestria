use super::*;

pub(super) fn owner_args<const N: usize>(
    command: [&str; N],
    profile: &Path,
    final_argument: Option<std::ffi::OsString>,
) -> Vec<std::ffi::OsString> {
    let mut args = Vec::with_capacity(N + 3 + usize::from(final_argument.is_some()));
    args.push("owner".into());
    args.extend(command.into_iter().map(std::ffi::OsString::from));
    args.push("--instance-dir".into());
    args.push(profile.as_os_str().to_os_string());
    if let Some(argument) = final_argument {
        args.push(argument);
    }
    args
}

impl SearchSetup {
    pub(super) async fn owner_roots_status(
        &self,
        owned: &mut OwnedState,
        generation: u64,
    ) -> Result<RootsStatus, String> {
        self.ensure_live_endpoint(owned)?;
        let (program, profile) = owned.program_and_profile()?;
        let output = self
            .cli(
                generation,
                &program,
                "owner roots status",
                owner_args(["roots", "status"], &profile, None),
            )
            .await?;
        self.ensure_live_endpoint(owned)?;
        let status: RootsStatus = serde_json::from_slice(&output.stdout).map_err(|_| {
            "The managed search service returned an invalid indexing status.".to_string()
        })?;
        Ok(status)
    }

    pub(super) async fn reconcile_roots(
        &self,
        owned: &mut OwnedState,
        marker: &mut ProfileMarker,
        status: &RootsStatus,
        selected: &Path,
        generation: u64,
    ) -> Result<(), String> {
        let observed = status
            .roots
            .iter()
            .map(|root| PathBuf::from(&root.path))
            .collect::<Vec<_>>();
        for root in &observed {
            if !marker.roots.iter().any(|known| known == root) {
                return Err("The private search profile contains an unowned approved folder; no folder authority was changed.".to_string());
            }
        }
        // Remove the previously consented root before adding a different one.
        for old in observed.iter().filter(|root| root.as_path() != selected) {
            self.check_intent(generation)?;
            let (program, profile) = owned.program_and_profile()?;
            self.cli(
                generation,
                &program,
                "owner roots remove",
                owner_args(
                    ["roots", "remove"],
                    &profile,
                    Some(old.as_os_str().to_os_string()),
                ),
            )
            .await?;
            marker.roots.retain(|known| known != old);
            write_marker(owned.profile_root()?, marker)?;
            owned.marker = Some(marker.clone());
        }
        if !observed.iter().any(|root| root.as_path() == selected) {
            if !marker.roots.iter().any(|root| root == selected) {
                marker.roots.push(selected.to_path_buf());
                write_marker(owned.profile_root()?, marker)?;
                owned.marker = Some(marker.clone());
            }
            let (program, profile) = owned.program_and_profile()?;
            self.cli(
                generation,
                &program,
                "owner roots add",
                owner_args(
                    ["roots", "add"],
                    &profile,
                    Some(selected.as_os_str().to_os_string()),
                ),
            )
            .await?;
        }
        let status = self.owner_roots_status(owned, generation).await?;
        if status.roots.len() != 1 || !same_path(&status.roots[0].path, selected) {
            return Err(
                "The managed search profile could not be reconciled to the one approved folder."
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(super) async fn cleanup_owned_profile(
        &self,
        owned: &mut OwnedState,
        generation: Option<u64>,
        deadline: Option<MonotonicInstant>,
    ) -> Result<(), String> {
        if owned.child.is_none() || owned.marker.is_none() {
            return Ok(());
        }
        self.ensure_live_endpoint(owned)?;
        let mut marker = owned
            .marker
            .clone()
            .ok_or_else(|| "The launcher search ownership marker is unavailable.".to_string())?;
        let owned_grants = marker.grants.clone();
        for grant in owned_grants {
            self.revoke_grant_until(owned, &grant.token_digest, generation, deadline)
                .await?;
            remove_private_file(owned.profile_root()?, &grant.credential_file)?;
            marker
                .grants
                .retain(|known| known.token_digest != grant.token_digest);
            write_marker(owned.profile_root()?, &marker)?;
            owned.marker = Some(marker.clone());
        }
        let roots = marker.roots.clone();
        for root in roots {
            self.remove_root_until(owned, &root, generation, deadline)
                .await?;
            marker.roots.retain(|known| known != &root);
            write_marker(owned.profile_root()?, &marker)?;
            owned.marker = Some(marker.clone());
        }
        marker.pending_root = None;
        marker.pending_credential_file = None;
        write_marker(owned.profile_root()?, &marker)?;
        owned.marker = Some(marker);
        Ok(())
    }

    pub(super) async fn remove_root_until(
        &self,
        owned: &mut OwnedState,
        root: &Path,
        generation: Option<u64>,
        deadline: Option<MonotonicInstant>,
    ) -> Result<(), String> {
        self.ensure_live_endpoint(owned)?;
        let status = self
            .owner_roots_status_until(owned, generation, deadline)
            .await?;
        if !status
            .roots
            .iter()
            .any(|approved| same_path(&approved.path, root))
        {
            return Ok(());
        }
        let (program, profile) = owned.program_and_profile()?;
        let args = owner_args(
            ["roots", "remove"],
            &profile,
            Some(root.as_os_str().to_os_string()),
        );
        self.run_owned_cli(
            owned,
            generation,
            deadline,
            &program,
            "owner roots remove",
            args,
        )
        .await?;
        Ok(())
    }

    pub(super) async fn owner_roots_status_until(
        &self,
        owned: &mut OwnedState,
        generation: Option<u64>,
        deadline: Option<MonotonicInstant>,
    ) -> Result<RootsStatus, String> {
        self.ensure_live_endpoint(owned)?;
        let (program, profile) = owned.program_and_profile()?;
        let args = owner_args(["roots", "status"], &profile, None);
        let output = self
            .run_owned_cli(
                owned,
                generation,
                deadline,
                &program,
                "owner roots status",
                args,
            )
            .await?;
        serde_json::from_slice(&output.stdout).map_err(|_| {
            "The managed search service returned an invalid indexing status.".to_string()
        })
    }
}
