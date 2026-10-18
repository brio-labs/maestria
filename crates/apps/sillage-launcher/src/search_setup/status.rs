use super::*;
use std::fmt::Write as _;

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
        let detail = owner_status_detail(root, &status, has_indexing_error);
        let label = owner_status_label(root, &status, has_indexing_error);
        let error = has_indexing_error.then(|| "Sillage reported an indexing issue.".to_string());
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

const MAX_STATUS_DETAIL_BYTES: usize = 768;
const MAX_DISPLAYED_SUPPORTED_FORMATS: usize = 4;
const MAX_SUPPORTED_FORMATS_TO_INSPECT: usize = 32;

fn owner_status_label(
    root: &RootStatus,
    status: &RootsStatus,
    has_indexing_error: bool,
) -> &'static str {
    if has_indexing_error {
        "Indexing issue"
    } else if status.indexing.admission_oversized_file_count > 0 {
        "Partial coverage"
    } else if status.indexing.scanning
        || status.indexing.pending_file_count > 0
        || status.indexing.last_scan_unix_ms.is_none()
    {
        "Indexing"
    } else if root.excluded_file_count > 0
        || status.ocr_needed_file_count > 0
        || root.exclusion_scan_truncated
        || status.exclusion_scan_truncated
    {
        "Partial coverage"
    } else {
        "Ready"
    }
}

fn owner_status_detail(
    root: &RootStatus,
    status: &RootsStatus,
    has_indexing_error: bool,
) -> String {
    let ocr_needed = status.ocr_needed_file_count.max(
        root.exclusions_by_reason
            .get("needs_ocr")
            .copied()
            .unwrap_or_default(),
    );
    let mut detail = format!(
        "{} indexed · {} pending · {} excluded · {} OCR needed",
        root.indexed_file_count,
        status.indexing.pending_file_count,
        root.excluded_file_count,
        ocr_needed
    );
    let admission_limit_mib = status.indexing.admission_budget_bytes / (1024 * 1024);
    let oversized_count = status.indexing.admission_oversized_file_count;
    let deferred_count = status
        .indexing
        .admission_deferred_file_count
        .saturating_sub(oversized_count);
    if deferred_count > 0 {
        write!(
            detail,
            " · Deferred source files: {deferred_count}; automatically rechecked on later scans under the {admission_limit_mib} MiB source-read budget."
        )
        .expect("writing to a String cannot fail");
    }
    if oversized_count > 0 {
        write!(
            detail,
            " · Oversized source files not read: {oversized_count}; per-file limit {admission_limit_mib} MiB."
        )
        .expect("writing to a String cannot fail");
    }

    if root.excluded_file_count > 0 {
        append_exclusion_reasons(&mut detail, root, ocr_needed);
    }

    let formats = supported_format_summary(&status.supported_formats);
    write!(detail, " · Supported formats: {formats}").expect("writing to a String cannot fail");

    if status.indexing.scanning || status.indexing.pending_file_count > 0 {
        detail.push_str(" · Scanning continues while the folder stays available.");
    }
    if root.exclusion_scan_truncated || status.exclusion_scan_truncated {
        detail.push_str(" · Exclusion counts may be incomplete; choose a smaller folder for a more complete count.");
    }
    if ocr_needed > 0 {
        detail.push_str(" · For OCR-needed files, use text-based PDFs or OCR them first.");
    }
    if root.excluded_file_count > ocr_needed {
        detail.push_str(
            " · Excluded sources are not searchable; review folder ignore and exclusion rules.",
        );
    }
    if has_indexing_error {
        detail.push_str(
            " · Scan issue: check folder availability and restart the launcher to retry.",
        );
    }
    bound_status_detail(detail)
}

fn append_exclusion_reasons(detail: &mut String, root: &RootStatus, ocr_needed: usize) {
    if root.exclusions_by_reason.is_empty() {
        if root.excluded_file_count > ocr_needed {
            detail.push_str(" · Other exclusion reasons not reported");
        }
        return;
    }

    const REASONS: [(&str, &str); 7] = [
        ("unsupported_format", "unsupported format"),
        ("hidden", "hidden"),
        ("ignore_rule", "ignore rule"),
        ("privacy_exclusion", "privacy exclusion"),
        ("symbolic_link", "symbolic link"),
        ("instance_internal", "Sillage internal"),
        ("source_changed_or_missing", "changed/missing"),
    ];

    let mut classified = ocr_needed;
    let mut has_reason = false;
    for (reason, label) in REASONS {
        let count = root
            .exclusions_by_reason
            .get(reason)
            .copied()
            .unwrap_or_default();
        if count == 0 {
            continue;
        }
        classified = classified.saturating_add(count);
        append_exclusion_reason(detail, label, count, &mut has_reason);
    }

    let other_count = root.excluded_file_count.saturating_sub(classified);
    if other_count > 0 {
        append_exclusion_reason(detail, "other/unreported", other_count, &mut has_reason);
    }
}

fn append_exclusion_reason(detail: &mut String, label: &str, count: usize, has_reason: &mut bool) {
    if !*has_reason {
        detail.push_str(" · Exclusions: ");
        *has_reason = true;
    } else {
        detail.push_str(", ");
    }
    write!(detail, "{count} {label}").expect("writing to a String cannot fail");
}

fn supported_format_summary(formats: &[String]) -> String {
    let mut summary = String::new();
    let mut displayed = 0;
    for format in formats.iter().take(MAX_SUPPORTED_FORMATS_TO_INSPECT) {
        let Some(label) = displayable_supported_format(format) else {
            continue;
        };
        if displayed == MAX_DISPLAYED_SUPPORTED_FORMATS {
            break;
        }
        if displayed > 0 {
            summary.push_str(", ");
        }
        summary.push_str(label);
        displayed += 1;
    }

    if displayed == 0 {
        return "not reported".to_string();
    }
    let omitted = formats.len().saturating_sub(displayed);
    if omitted > 0 {
        write!(summary, " + {omitted} more").expect("writing to a String cannot fail");
    }
    summary
}

fn displayable_supported_format(format: &str) -> Option<&str> {
    if format == "Cargo.toml (exact filename)" {
        return Some("Cargo.toml");
    }
    let bytes = format.as_bytes();
    (bytes.len() > 1
        && bytes.len() <= 16
        && bytes[0] == b'.'
        && bytes[1..].iter().all(u8::is_ascii_alphanumeric))
    .then_some(format)
}

fn bound_status_detail(mut detail: String) -> String {
    if detail.len() > MAX_STATUS_DETAIL_BYTES {
        let mut end = MAX_STATUS_DETAIL_BYTES - "…".len();
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
        detail.push('…');
    }
    detail
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_status(indexing: serde_json::Value) -> RootsStatus {
        serde_json::from_value(json!({
            "roots": [{
                "path": "/private/approved",
                "indexed_file_count": 8,
                "excluded_file_count": 0
            }],
            "indexing": indexing
        }))
        .expect("indexing status should be readable")
    }

    #[test]
    fn legacy_status_defaults_admission_fields() {
        let status = parse_status(json!({
            "scanning": false,
            "pending_file_count": 0,
            "last_scan_unix_ms": 12,
            "last_error": null
        }));
        assert_eq!(status.indexing.admission_budget_bytes, 0);
        assert_eq!(status.indexing.admission_deferred_file_count, 0);
        assert_eq!(status.indexing.admission_oversized_file_count, 0);
        assert_eq!(
            owner_status_label(&status.roots[0], &status, false),
            "Ready"
        );
    }

    #[test]
    fn oversized_sources_are_partial_and_raw_errors_stay_private() {
        let mut status = parse_status(json!({
            "scanning": false,
            "pending_file_count": 2,
            "admission_budget_bytes": 67108864,
            "admission_deferred_file_count": 2,
            "admission_oversized_file_count": 1,
            "last_scan_unix_ms": 24,
            "last_error": "/private/approved/report.pdf: sensitive raw error content"
        }));
        let root = &status.roots[0];
        let detail = owner_status_detail(root, &status, true);
        assert_eq!(owner_status_label(root, &status, true), "Indexing issue");
        assert!(!detail.contains("/private/"));
        assert!(!detail.contains("sensitive raw error content"));
        assert!(detail.len() <= MAX_STATUS_DETAIL_BYTES);

        status.indexing.last_error = None;
        assert_eq!(
            owner_status_label(&status.roots[0], &status, false),
            "Partial coverage"
        );
    }

    #[test]
    fn temporary_admission_deferrals_remain_indexing_until_retried() {
        let mut status = parse_status(json!({
            "scanning": true,
            "pending_file_count": 1,
            "admission_budget_bytes": 67108864,
            "admission_deferred_file_count": 1,
            "admission_oversized_file_count": 0,
            "last_scan_unix_ms": 24,
            "last_error": null
        }));
        assert_eq!(
            owner_status_label(&status.roots[0], &status, false),
            "Indexing"
        );

        status.indexing.scanning = false;
        status.indexing.pending_file_count = 0;
        status.indexing.admission_deferred_file_count = 0;
        assert_eq!(
            owner_status_label(&status.roots[0], &status, false),
            "Ready"
        );
    }
}

