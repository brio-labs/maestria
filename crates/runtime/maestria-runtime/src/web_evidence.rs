use crate::config::EffectExecutionContext;
use maestria_domain::{
    ArtifactDetected, Authority, ContentHash, DomainInput, EvidenceKind, FetchWebRequest,
    IntegrityState, LogicalTick, RecordEvidenceInput, RegisterArtifactInput, ReviewStatus,
    SecurityMetadata, SnapshotRef, TrustZone, content_hash, web_artifact_id_for,
    web_evidence_id_for,
};
use maestria_governance::{contains_prompt_injection_risk, scan_secrets};
use maestria_ports::WebFetchOptions;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

impl EffectExecutionContext {
    pub(crate) async fn handle_fetch_web(&self, request: FetchWebRequest) -> bool {
        if request.max_bytes == 0 || request.max_requests == 0 || request.max_latency_ms == 0 {
            tracing::warn!("web fetch rejected because its budget is zero");
            return false;
        }
        let options = WebFetchOptions {
            max_bytes: request.max_bytes,
            max_latency_ms: request.max_latency_ms,
            allowed_domains: request.allowed_domains.clone(),
            allowed_content_types: request.allowed_content_types.clone(),
        };
        let fetcher = std::sync::Arc::clone(&self.adapters.web_fetcher);
        let url = request.url.clone();
        let fetch = tokio::task::spawn_blocking(move || fetcher.fetch_with_options(&url, &options));
        let mut snapshot = match tokio::time::timeout(
            Duration::from_millis(u64::from(request.max_latency_ms)),
            fetch,
        )
        .await
        {
            Ok(Ok(Ok(snapshot))) => snapshot,
            Ok(Ok(Err(error))) => {
                tracing::error!(url = %request.url, %error, "web fetch failed");
                return false;
            }
            Ok(Err(error)) => {
                tracing::error!(url = %request.url, %error, "web fetch worker failed");
                return false;
            }
            Err(_) => {
                tracing::warn!(url = %request.url, "web fetch exceeded latency budget");
                return false;
            }
        };
        if !domain_allowed(&snapshot.url, &request.allowed_domains) {
            tracing::warn!(url = %request.url, "web response is outside the allowed domain budget");
            return false;
        }
        if !content_type_allowed(
            snapshot.metadata.content_type.as_deref(),
            &request.allowed_content_types,
        ) {
            tracing::warn!(url = %request.url, "web response content type is outside the budget");
            return false;
        }
        if snapshot.html.len() > request.max_bytes {
            tracing::warn!(url = %request.url, "web response exceeded byte budget");
            return false;
        }
        let computed_hash = content_hash(snapshot.html.as_bytes());
        if snapshot.content_hash != computed_hash {
            tracing::warn!(url = %request.url, "web adapter returned an invalid content hash");
            return false;
        }
        let observed_at = {
            let state = self.state.read().await;
            state.event_log.last().map_or(0, |entry| entry.id.value())
        };
        let accessed_at = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(error) => {
                // Rule 24: a clock failure must not fabricate an epoch
                // timestamp into the evidence record; drop the snapshot
                // instead of persisting a misleading accessed_at.
                tracing::error!(url = %request.url, %error, "system clock before unix epoch; web evidence rejected");
                return false;
            }
        };
        let mut metadata = std::mem::take(&mut snapshot.metadata);
        metadata.accessed_at = Some(accessed_at.to_string());
        let security = self.security_metadata_for_web(&snapshot.html);
        self.persist_web_evidence(snapshot, metadata, security, observed_at)
            .await
    }

    fn security_metadata_for_web(&self, html: &str) -> SecurityMetadata {
        let secret_scan = scan_secrets(html);
        let lower = Self::normalized_web_text(html);
        let prompt_injection_risk = contains_prompt_injection_risk(&lower);
        let mut security = SecurityMetadata {
            trust_zone: TrustZone::Untrusted,
            authority: Authority::External,
            integrity: IntegrityState::Verified,
            sensitivity: maestria_domain::Sensitivity::Internal,
            review_status: ReviewStatus::Unreviewed,
            prompt_injection_risk,
            poisoning_flags: Vec::new(),
            read_allowed: true,
            write_allowed: false,
            scope_id: Some(self.scope_id),
        };
        if !secret_scan.findings.is_empty() {
            security.poisoning_flags.push("secret_signal".to_string());
        }
        if prompt_injection_risk {
            security
                .poisoning_flags
                .push("prompt_injection_signal".to_string());
        }
        if !security.poisoning_flags.is_empty() {
            security.trust_zone = TrustZone::Quarantined;
            security.review_status = ReviewStatus::Pending;
        }
        security
    }

    fn normalized_web_text(html: &str) -> String {
        let mut out = String::with_capacity(html.len());
        let mut in_tag = false;
        let mut prev_was_space = true;
        let mut idx = 0;
        while idx < html.len() {
            let Some(ch) = html[idx..].chars().next() else {
                break;
            };
            let char_len = ch.len_utf8();
            let lc = ch.to_ascii_lowercase();
            if !in_tag && lc == '<' {
                in_tag = true;
                if !prev_was_space && !out.is_empty() {
                    out.push(' ');
                    prev_was_space = true;
                }
                idx += char_len;
                continue;
            }
            if in_tag && lc == '>' {
                in_tag = false;
                if !prev_was_space && !out.is_empty() {
                    out.push(' ');
                    prev_was_space = true;
                }
                idx += char_len;
                continue;
            }
            if in_tag {
                idx += char_len;
                continue;
            }
            if lc == '&' {
                let rem = &html[idx..];
                if rem.len() >= 5 && rem[..5].eq_ignore_ascii_case("&#32;") {
                    if !prev_was_space && !out.is_empty() {
                        out.push(' ');
                        prev_was_space = true;
                    } else if out.is_empty() {
                        prev_was_space = true;
                    }
                    idx += 5;
                    continue;
                }
                if rem.len() >= 6 && rem[..6].eq_ignore_ascii_case("&#x20;") {
                    if !prev_was_space && !out.is_empty() {
                        out.push(' ');
                        prev_was_space = true;
                    } else if out.is_empty() {
                        prev_was_space = true;
                    }
                    idx += 6;
                    continue;
                }
                if rem.len() >= 6 && rem[..6].eq_ignore_ascii_case("&nbsp;") {
                    if !prev_was_space && !out.is_empty() {
                        out.push(' ');
                        prev_was_space = true;
                    } else if out.is_empty() {
                        prev_was_space = true;
                    }
                    idx += 6;
                    continue;
                }
                if rem.len() >= 5 && rem[..5].eq_ignore_ascii_case("&amp;") {
                    if prev_was_space && out.is_empty() {
                        // leading '&' not preceded by space, keep
                    }
                    out.push('&');
                    prev_was_space = false;
                    idx += 5;
                    continue;
                }
            }
            if lc.is_ascii_whitespace() || lc.is_whitespace() {
                if !prev_was_space && !out.is_empty() {
                    out.push(' ');
                    prev_was_space = true;
                }
                idx += char_len;
                continue;
            }
            out.push(lc);
            prev_was_space = false;
            idx += char_len;
        }
        if prev_was_space && out.ends_with(' ') {
            out.pop();
        }
        out
    }
    async fn persist_web_evidence(
        &self,
        snapshot: maestria_ports::WebSnapshotData,
        metadata: maestria_domain::WebEvidenceMetadata,
        security: SecurityMetadata,
        observed_at: u64,
    ) -> bool {
        let blob_id = match self
            .adapters
            .blob_store
            .put(snapshot.html.as_bytes().to_vec())
        {
            Ok(blob_id) => blob_id,
            Err(error) => {
                tracing::error!(url = %snapshot.url, %error, "web snapshot persistence failed");
                return false;
            }
        };
        let source_bytes = snapshot.html.as_bytes().to_vec();
        let snapshot_hash = snapshot.content_hash.clone();
        let snapshot_hash_typed = match ContentHash::new(snapshot_hash.clone()) {
            Ok(hash) => hash,
            Err(error) => {
                tracing::error!(url = %snapshot.url, %error, "web snapshot hash is invalid");
                return false;
            }
        };
        let snapshot_ref = SnapshotRef::new(blob_id, snapshot_hash_typed.clone());
        let artifact_id = web_artifact_id_for(&snapshot.url, &snapshot.content_hash);
        let evidence_id = web_evidence_id_for(artifact_id);
        if self
            .input_tx
            .send(DomainInput::RegisterArtifact(RegisterArtifactInput {
                artifact_id,
                title: snapshot.url.clone(),
                security: Some(security.clone()),
            }))
            .await
            .is_err()
        {
            tracing::error!(url = %snapshot.url, "web artifact registration failed");
            return false;
        }
        if self
            .input_tx
            .send(DomainInput::RecordEvidence(RecordEvidenceInput {
                evidence_id,
                artifact_id,
                claim_id: None,
                kind: EvidenceKind::WebSnapshot {
                    url: snapshot.url.clone(),
                    snapshot: snapshot_ref,
                    fetched_at: LogicalTick::new(observed_at),
                    metadata,
                },
                excerpt: snapshot.html,
                observed_at: LogicalTick::new(observed_at),
                security: Some(security),
            }))
            .await
            .is_err()
        {
            tracing::error!(url = %snapshot.url, "web evidence recording failed");
            return false;
        }
        if self
            .input_tx
            .send(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id,
                title: snapshot.url.clone(),
                source_path: snapshot.url.clone(),
                source_bytes,
                content_hash: snapshot_hash_typed.clone(),
            }))
            .await
            .is_err()
        {
            tracing::error!(url = %snapshot.url, "web artifact indexing request failed");
            return false;
        }
        true
    }
}

fn domain_allowed(url: &str, allowed_domains: &[String]) -> bool {
    if allowed_domains.is_empty() {
        return true;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allowed_domains.iter().any(|domain| {
        let domain = domain.trim_end_matches('.').to_ascii_lowercase();
        host == domain
            || host
                .strip_suffix(&domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

fn content_type_allowed(content_type: Option<&str>, allowed_types: &[String]) -> bool {
    allowed_types.is_empty()
        || content_type.is_some_and(|actual| {
            allowed_types
                .iter()
                .any(|allowed| actual.starts_with(allowed))
        })
}
