use anyhow::{Result, anyhow};
use maestria_domain::{
    CorpusScope, DomainInput, FederatedAccessRecord, FederatedReadOperation, GrantTokenDigest,
    RealmId, RecordFederatedAccessInput,
};
use maestria_governance::{FederatedGrantDecision, FederatedGrantDenial, authorize_federated_read};

use super::super::federation_previews::{self, RESPONSE_EVIDENCE_RESERVE_BYTES};
use super::super::protocol::SearchPathResultResponse;
use super::super::server::{ApiContext, InteractiveSearchControl, RequestPrincipal};
use super::super::{
    ClientOperation, ClientResponse, FederationCredential, FederationEvidenceResponse,
    FederationSearchResponse,
};
use super::federation_binding::{self, FederationBinding};

const MAX_SEARCH_LIMIT: usize = 100;

pub(super) async fn install_binding(
    context: &ApiContext,
    principal: &RequestPrincipal,
    provider_realm: RealmId,
    provider_socket_path: String,
    credential: FederationCredential,
) -> Result<ClientResponse> {
    require_instance(principal)?;
    let provider_socket_path = std::path::PathBuf::from(provider_socket_path);
    if !provider_socket_path.is_absolute() {
        return Err(anyhow!("provider socket path must be absolute"));
    }
    federation_binding::install(
        &context.layout,
        FederationBinding {
            provider_realm,
            provider_socket_path,
            credential,
        },
    )?;
    Ok(ClientResponse::FederationBindingInstalled)
}

pub(super) async fn search(
    context: &ApiContext,
    principal: &RequestPrincipal,
    provider_realm: RealmId,
    query: String,
    limit: usize,
) -> Result<ClientResponse> {
    match principal {
        RequestPrincipal::Instance => relay_search(context, provider_realm, query, limit).await,
        RequestPrincipal::Federation {
            consumer_realm,
            credential,
        } => {
            serve_search(
                context,
                consumer_realm,
                credential,
                provider_realm,
                query,
                limit,
                None,
            )
            .await
        }
    }
}

pub(super) async fn interactive_search(
    context: &ApiContext,
    principal: &RequestPrincipal,
    provider_realm: RealmId,
    query: String,
    limit: usize,
    control: InteractiveSearchControl,
) -> Result<ClientResponse> {
    let RequestPrincipal::Federation {
        consumer_realm,
        credential,
    } = principal
    else {
        return Err(anyhow!("interactive search requires a federation grant"));
    };
    serve_search(
        context,
        consumer_realm,
        credential,
        provider_realm,
        query,
        limit,
        Some(control),
    )
    .await
}

pub(super) async fn evidence(
    context: &ApiContext,
    principal: &RequestPrincipal,
    provider_realm: RealmId,
    evidence_id: u64,
) -> Result<ClientResponse> {
    match principal {
        RequestPrincipal::Instance => relay_evidence(context, provider_realm, evidence_id).await,
        RequestPrincipal::Federation {
            consumer_realm,
            credential,
        } => {
            serve_evidence(
                context,
                consumer_realm,
                credential,
                provider_realm,
                evidence_id,
            )
            .await
        }
    }
}

async fn relay_search(
    context: &ApiContext,
    provider_realm: RealmId,
    query: String,
    limit: usize,
) -> Result<ClientResponse> {
    let binding = federation_binding::load(&context.layout, &provider_realm)?;
    let client = super::super::protocol::DaemonClient::federation(
        binding.provider_socket_path,
        context.realm_id.clone(),
        binding.credential,
    );
    let response = client
        .request(ClientOperation::FederationSearch {
            provider_realm: provider_realm.clone(),
            query,
            limit,
        })
        .await?;
    let ClientResponse::FederationSearch(response) = response else {
        return Err(anyhow!(
            "provider returned unexpected federation search response"
        ));
    };
    if response.provider_realm != provider_realm {
        return Err(anyhow!(
            "provider response realm does not match requested realm"
        ));
    }
    Ok(ClientResponse::FederationSearch(response))
}

async fn relay_evidence(
    context: &ApiContext,
    provider_realm: RealmId,
    evidence_id: u64,
) -> Result<ClientResponse> {
    let binding = federation_binding::load(&context.layout, &provider_realm)?;
    let client = super::super::protocol::DaemonClient::federation(
        binding.provider_socket_path,
        context.realm_id.clone(),
        binding.credential,
    );
    let response = client
        .request(ClientOperation::FederationEvidence {
            provider_realm: provider_realm.clone(),
            evidence_id,
        })
        .await?;
    let ClientResponse::FederationEvidence(response) = response else {
        return Err(anyhow!(
            "provider returned unexpected federation evidence response"
        ));
    };
    if response.provider_realm != provider_realm {
        return Err(anyhow!(
            "provider response realm does not match requested realm"
        ));
    }
    Ok(ClientResponse::FederationEvidence(response))
}

async fn serve_search(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
    requested_provider_realm: RealmId,
    query: String,
    limit: usize,
    interactive: Option<InteractiveSearchControl>,
) -> Result<ClientResponse> {
    if requested_provider_realm != context.realm_id {
        return denied();
    }
    if query.trim().is_empty() || !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Err(anyhow!("federated search request is invalid"));
    }
    let grant = grant_for(context, consumer_realm, credential).await?;
    let now = unix_time_seconds()?;
    let (authorization, bounds) = match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::Search,
        &grant,
        now,
        &maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![maestria_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed {
            authorization,
            bounds,
        } => (authorization, bounds),
        FederatedGrantDecision::Denied(denial) => return Err(grant_denial(denial)),
    };
    let approved_roots = context.source_manifest.read().read_roots.clone();
    let executor = runtime(context)?
        .search_executor()
        .ok_or_else(|| anyhow!("daemon-owned search executor is unavailable"))?;
    let search_runtime = executor
        .as_any()
        .and_then(|runtime| runtime.downcast_ref::<crate::SearchRuntime>())
        .ok_or_else(|| anyhow!("daemon-owned search executor has an unexpected type"))?;
    let request_runtime = search_runtime
        .without_graph_expansion()
        .with_allowed_roots(grant.allowed_roots());
    let bounded_limit = limit.min(bounds.max_results());
    // Registration follows grant authentication. Supersession is scoped to this
    // consumer, while the shared worker semaphore bounds total daemon load.
    let interactive_request = interactive.as_ref().map(|control| {
        context
            .interactive_searches
            .begin(consumer_realm.clone(), control.clone())
    });
    let (plan, outcome) = match interactive.as_ref() {
        Some(control) => {
            request_runtime
                .execute_interactive(
                    query,
                    bounded_limit,
                    authorization.clone(),
                    control.cancellation.clone(),
                    control.signal.clone(),
                )
                .await?
        }
        None => {
            request_runtime
                .execute_pre_authorized(query, bounded_limit, authorization.clone())
                .await?
        }
    };
    let path_candidates = if let Some(control) = interactive.as_ref() {
        request_runtime
            .interactive_path_candidates(
                plan.original_query().to_string(),
                bounded_limit,
                authorization.clone(),
                control.cancellation.clone(),
                control.signal.clone(),
            )
            .await?
    } else {
        Vec::new()
    };
    let query_id = plan.query_id();
    let trace_id = outcome.trace;

    let preview_candidates = federation_previews::search_preview_candidates(&outcome.evidence);
    let mut response = FederationSearchResponse {
        provider_realm: context.realm_id.clone(),
        graph_degraded: true,
        search: super::search_services::search_response(
            plan.original_query().to_string(),
            plan.query_id().value(),
            outcome,
        ),
    };

    if !preview_candidates.is_empty() {
        federation_previews::open_and_attach_search_previews(
            &context.layout,
            &mut response,
            preview_candidates,
            authorization.clone(),
            bounds.max_evidence_bytes(),
            if interactive_request.is_some() {
                Some(search_runtime.interactive_current_sources()?)
            } else {
                None
            },
            grant
                .allowed_roots()
                .map(|roots| std::sync::Arc::from(roots.to_vec())),
        )
        .await;
    }
    if interactive_request.is_some() {
        let original_count = response.search.evidence.len();
        // The lexical prefilter enforces currently approved source roots. Only
        // a fresh scoped evidence reopen may release a passage: edits, deletes,
        // or a symlink/root change cannot leak stale cited metadata.
        response
            .search
            .evidence
            .retain(|evidence| evidence.preview.is_some());
        if response.search.evidence.len() != original_count {
            let valid_versions: std::collections::BTreeSet<_> = response
                .search
                .evidence
                .iter()
                .map(|evidence| evidence.artifact_version)
                .collect();
            response.search.coverage.percent_covered = 0;
            response.search.coverage.gaps.clear();
            response.search.coverage.distinct_sources = valid_versions.len();
            response.search.coverage.distinct_documents = valid_versions.len();
            response.search.coverage.distinct_sections = response.search.evidence.len();
            if response.search.evidence.is_empty() {
                response.search.status = "NoEvidenceFound".to_string();
            }
        }
    }
    record_access(
        context,
        grant.token_digest().clone(),
        consumer_realm.clone(),
        FederatedAccessRecord::Search { query_id, trace_id },
    )
    .await?;
    if let Some(control) = interactive.as_ref() {
        let path_result_limit = bounded_limit.saturating_sub(response.search.evidence.len());
        let verified_paths = request_runtime
            .validate_interactive_path_candidates(
                path_candidates,
                authorization.clone(),
                control.cancellation.clone(),
                control.signal.clone(),
            )
            .await?;
        response.search.path_results = verified_paths
            .into_iter()
            .take(path_result_limit)
            .map(|path| SearchPathResultResponse { path })
            .collect();
        federation_previews::fit_search_path_results(&mut response);
        if response.search.evidence.is_empty() && !response.search.path_results.is_empty() {
            response.search.status = "PathResultsFound".to_string();
        }
    }

    // Re-read the grant after source opens and audit persistence. A revocation
    // concurrent with any of those awaits must not release a preview response.
    let current_grant = grant_for(context, consumer_realm, credential).await?;
    match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::Search,
        &current_grant,
        unix_time_seconds()?,
        &maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![maestria_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed { .. } => {}
        FederatedGrantDecision::Denied(denial) => return Err(grant_denial(denial)),
    }
    if context.source_manifest.read().read_roots != approved_roots {
        return Err(anyhow!(
            "provider read roots changed during federated search"
        ));
    }
    Ok(ClientResponse::FederationSearch(response))
}

async fn serve_evidence(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
    requested_provider_realm: RealmId,
    evidence_id: u64,
) -> Result<ClientResponse> {
    if requested_provider_realm != context.realm_id {
        return denied();
    }
    let grant = grant_for(context, consumer_realm, credential).await?;
    let now = unix_time_seconds()?;
    let (authorization, bounds) = match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::OpenEvidence,
        &grant,
        now,
        &maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![maestria_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed {
            authorization,
            bounds,
        } => (authorization, bounds),
        FederatedGrantDecision::Denied(denial) => return Err(grant_denial(denial)),
    };
    let approved_roots = context.source_manifest.read().read_roots.clone();
    let max_evidence_bytes = bounds
        .max_evidence_bytes()
        .min(super::super::MAX_REQUEST_BYTES - RESPONSE_EVIDENCE_RESERVE_BYTES);
    let grant_digest = grant.token_digest().clone();
    let layout = context.layout.clone();
    let (mut output, pdf_source_path) = tokio::task::spawn_blocking(move || {
        crate::evidence_open::open_evidence_scoped_with_authorization_and_pdf_path(
            &layout,
            evidence_id,
            authorization,
            grant.allowed_roots(),
        )
    })
    .await
    .map_err(|error| anyhow!("federated evidence task failed: {error}"))??;
    truncate_utf8(&mut output.evidence.excerpt, max_evidence_bytes);
    let evidence = super::read_services::evidence_response_with_pdf_source_path(
        output,
        pdf_source_path.as_deref(),
    )?;
    record_access(
        context,
        grant_digest,
        consumer_realm.clone(),
        FederatedAccessRecord::Evidence {
            evidence_id: maestria_domain::EvidenceId::new(evidence_id),
        },
    )
    .await?;
    // The source open and durable audit both await. A grant revoked during
    // either operation must not release the reopened excerpt to its client.
    let current_grant = grant_for(context, consumer_realm, credential).await?;
    match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::OpenEvidence,
        &current_grant,
        unix_time_seconds()?,
        &maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![maestria_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed { .. } => {}
        FederatedGrantDecision::Denied(denial) => return Err(grant_denial(denial)),
    }
    if context.source_manifest.read().read_roots != approved_roots {
        return Err(anyhow!(
            "provider read roots changed during federated evidence open"
        ));
    }
    Ok(ClientResponse::FederationEvidence(
        FederationEvidenceResponse {
            provider_realm: context.realm_id.clone(),
            evidence,
        },
    ))
}
pub(super) async fn status(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<super::super::protocol::RetrievalStatusResponse> {
    authorized_status_grant(context, consumer_realm, credential).await?;
    super::search_services::retrieval_status(context).await
}

pub(super) async fn indexing_status(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<super::super::protocol::SearchRootsStatusResponse> {
    let grant = authorized_status_grant(context, consumer_realm, credential).await?;
    super::search_roots_services::status_for_roots(context, grant.allowed_roots()).await
}

async fn authorized_status_grant(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<maestria_domain::RealmReadGrant> {
    let grant = grant_for(context, consumer_realm, credential).await?;
    let now = unix_time_seconds()?;
    match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::Search,
        &grant,
        now,
        &maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![maestria_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed { .. } => Ok(grant),
        FederatedGrantDecision::Denied(denial) => Err(grant_denial(denial)),
    }
}

async fn grant_for(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<maestria_domain::RealmReadGrant> {
    let digest = GrantTokenDigest::derive(credential.as_str().as_bytes());
    let handle = runtime(context)?;
    let Some(projected) = handle
        .realm_read_grant_repository()
        .get(&digest)
        .map_err(|error| anyhow!(error))?
    else {
        return denied();
    };
    let state = handle.kernel_state().await;
    let Some(current) = state.realm_read_grants.get(&digest) else {
        return denied();
    };
    if current != &projected {
        return denied();
    }
    let grant = projected;
    if grant.consumer_realm() != consumer_realm || grant.provider_realm() != &context.realm_id {
        return denied();
    }
    Ok(grant)
}

async fn record_access(
    context: &ApiContext,
    token_digest: GrantTokenDigest,
    consumer_realm: RealmId,
    record: FederatedAccessRecord,
) -> Result<()> {
    runtime(context)?
        .submit_durable(DomainInput::RecordFederatedAccess(
            RecordFederatedAccessInput {
                token_digest,
                provider_realm: context.realm_id.clone(),
                consumer_realm,
                record,
            },
        ))
        .await
        .map_err(|error| anyhow!(error))?;
    Ok(())
}

fn runtime(context: &ApiContext) -> Result<&maestria_runtime::RuntimeHandle> {
    context
        .runtime
        .as_ref()
        .ok_or_else(|| anyhow!("realm federation requires a live daemon runtime"))
}

fn require_instance(principal: &RequestPrincipal) -> Result<()> {
    if matches!(principal, RequestPrincipal::Instance) {
        Ok(())
    } else {
        denied()
    }
}

fn denied<T>() -> Result<T> {
    Err(anyhow!("federation access denied"))
}
fn grant_denial(denial: FederatedGrantDenial) -> anyhow::Error {
    match denial {
        FederatedGrantDenial::GrantExpired => {
            anyhow!("federation grant expired; ask provider to reissue it")
        }
        _ => anyhow!("federation access denied"),
    }
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    *value = maestria_ports::truncate_at_char_boundary(value, max_bytes).to_owned();
}
fn unix_time_seconds() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow!("read grant authorization clock: {error}"))?
        .as_secs())
}
