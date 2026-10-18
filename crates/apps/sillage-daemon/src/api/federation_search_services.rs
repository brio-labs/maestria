use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow};
use sillage_domain::{
    CorpusScope, FederatedAccessRecord, FederatedEvidenceBounds, FederatedReadOperation, QueryId,
    RealmId, RealmReadGrant, SearchOutcome, SearchPlan, SearchTraceId,
};
use sillage_governance::{
    FederatedGrantDecision, RetrievalAuthorizationContext, authorize_federated_read,
};

use super::super::super::federation_previews;
use super::super::super::protocol::SearchPathResultResponse;
use super::super::super::server::{ApiContext, InteractiveSearchControl};
use super::super::super::{ClientResponse, FederationCredential, FederationSearchResponse};

struct AuthorizedSearch {
    grant: RealmReadGrant,
    authorization: RetrievalAuthorizationContext,
    bounds: FederatedEvidenceBounds,
}

pub(super) async fn serve_search(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
    requested_provider_realm: RealmId,
    query: String,
    limit: usize,
    interactive: Option<InteractiveSearchControl>,
) -> Result<ClientResponse> {
    let authorized = authorize_search(
        context,
        consumer_realm,
        credential,
        &requested_provider_realm,
        &query,
        limit,
    )
    .await?;
    let AuthorizedSearch {
        grant,
        authorization,
        bounds,
    } = &authorized;
    let approved_roots = context.source_manifest.read().read_roots.clone();
    let executor = super::runtime(context)?
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
    let (plan, outcome, path_candidates) = execute_search(
        &request_runtime,
        query,
        bounded_limit,
        authorization,
        interactive.as_ref(),
    )
    .await?;
    let (mut response, query_id, trace_id) = shape_search_response(
        context,
        search_runtime,
        plan,
        outcome,
        &authorized,
        interactive_request.is_some(),
    )
    .await?;
    super::record_access(
        context,
        grant.token_digest().clone(),
        consumer_realm.clone(),
        FederatedAccessRecord::Search { query_id, trace_id },
    )
    .await?;
    if let Some(control) = interactive.as_ref() {
        add_validated_path_results(
            &mut response,
            &request_runtime,
            path_candidates,
            authorization,
            control,
            bounded_limit,
        )
        .await?;
    }
    ensure_current_search_authorization(context, consumer_realm, credential, &approved_roots)
        .await?;
    Ok(ClientResponse::FederationSearch(response))
}

async fn authorize_search(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
    requested_provider_realm: &RealmId,
    query: &str,
    limit: usize,
) -> Result<AuthorizedSearch> {
    if requested_provider_realm != &context.realm_id {
        return super::denied();
    }
    if query.trim().is_empty()
        || !(1..=super::super::super::protocol::MAX_SEARCH_LIMIT).contains(&limit)
    {
        return Err(anyhow!("federated search request is invalid"));
    }
    let grant = super::grant_for(context, consumer_realm, credential).await?;
    let now = super::unix_time_seconds()?;
    let (authorization, bounds) = match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::Search,
        &grant,
        now,
        &sillage_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![sillage_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed {
            authorization,
            bounds,
        } => (authorization, bounds),
        FederatedGrantDecision::Denied(denial) => return Err(super::grant_denial(denial)),
    };
    Ok(AuthorizedSearch {
        grant,
        authorization,
        bounds,
    })
}

async fn execute_search(
    request_runtime: &crate::SearchRuntime,
    query: String,
    limit: usize,
    authorization: &RetrievalAuthorizationContext,
    interactive: Option<&InteractiveSearchControl>,
) -> Result<(
    SearchPlan,
    SearchOutcome,
    Vec<crate::search_executor::InteractivePathCandidate>,
)> {
    let (plan, outcome) = match interactive {
        Some(control) => {
            request_runtime
                .execute_interactive(
                    query,
                    limit,
                    authorization.clone(),
                    control.cancellation.clone(),
                    control.signal.clone(),
                )
                .await?
        }
        None => {
            request_runtime
                .execute_pre_authorized(query, limit, authorization.clone())
                .await?
        }
    };
    let path_candidates = match interactive {
        Some(control) => {
            request_runtime
                .interactive_path_candidates(
                    plan.original_query().to_string(),
                    limit,
                    authorization.clone(),
                    control.cancellation.clone(),
                    control.signal.clone(),
                )
                .await?
        }
        None => Vec::new(),
    };
    Ok((plan, outcome, path_candidates))
}

async fn shape_search_response(
    context: &ApiContext,
    search_runtime: &crate::SearchRuntime,
    plan: SearchPlan,
    outcome: SearchOutcome,
    authorized: &AuthorizedSearch,
    interactive: bool,
) -> Result<(FederationSearchResponse, QueryId, SearchTraceId)> {
    let query_id = plan.query_id();
    let trace_id = outcome.trace;
    let preview_candidates = federation_previews::search_preview_candidates(&outcome.evidence);
    let mut response = FederationSearchResponse {
        provider_realm: context.realm_id.clone(),
        graph_degraded: true,
        search: super::super::search_services::search_response(
            plan.original_query().to_string(),
            query_id.value(),
            outcome,
        ),
    };
    if !preview_candidates.is_empty() {
        federation_previews::open_and_attach_search_previews(
            &context.layout,
            &mut response,
            preview_candidates,
            authorized.authorization.clone(),
            authorized.bounds.max_evidence_bytes(),
            if interactive {
                Some(search_runtime.interactive_current_sources()?)
            } else {
                None
            },
            authorized
                .grant
                .allowed_roots()
                .map(|roots| Arc::from(roots.to_vec())),
        )
        .await;
    }
    if interactive {
        retain_reopened_evidence(&mut response);
    }
    Ok((response, query_id, trace_id))
}

fn retain_reopened_evidence(response: &mut FederationSearchResponse) {
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

async fn add_validated_path_results(
    response: &mut FederationSearchResponse,
    request_runtime: &crate::SearchRuntime,
    path_candidates: Vec<crate::search_executor::InteractivePathCandidate>,
    authorization: &RetrievalAuthorizationContext,
    control: &InteractiveSearchControl,
    bounded_limit: usize,
) -> Result<()> {
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
    federation_previews::fit_search_path_results(response);
    if response.search.evidence.is_empty() && !response.search.path_results.is_empty() {
        response.search.status = "PathResultsFound".to_string();
    }
    Ok(())
}

async fn ensure_current_search_authorization(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
    approved_roots: &[PathBuf],
) -> Result<()> {
    // Re-read the grant after source opens and audit persistence. A revocation
    // concurrent with any of those awaits must not release a preview response.
    let current_grant = super::grant_for(context, consumer_realm, credential).await?;
    match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::Search,
        &current_grant,
        super::unix_time_seconds()?,
        &sillage_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![sillage_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed { .. } => {}
        FederatedGrantDecision::Denied(denial) => return Err(super::grant_denial(denial)),
    }
    if context.source_manifest.read().read_roots != approved_roots {
        return Err(anyhow!(
            "provider read roots changed during federated search"
        ));
    }
    Ok(())
}
