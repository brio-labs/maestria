use anyhow::{Context, Result, anyhow};
use maestria_domain::{
    CorpusScope, DomainInput, FederatedAccessRecord, FederatedReadOperation, GrantTokenDigest,
    RealmId, RecordFederatedAccessInput,
};
use maestria_governance::{FederatedGrantDecision, authorize_federated_read};

use super::super::protocol::{
    ClientOperation, ClientResponse, FederationCredential, FederationEvidenceResponse,
    FederationSearchResponse,
};
use super::super::server::{ApiContext, RequestPrincipal};
use super::federation_binding::{self, FederationBinding};

const MAX_SEARCH_LIMIT: usize = 100;
const RESPONSE_EVIDENCE_RESERVE_BYTES: usize = 4 * 1024;

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
            )
            .await
        }
    }
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
) -> Result<ClientResponse> {
    if requested_provider_realm != context.realm_id {
        return denied();
    }
    if query.trim().is_empty() || !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Err(anyhow!("federated search request is invalid"));
    }
    let grant = grant_for(context, consumer_realm, credential).await?;
    let (authorization, bounds) = match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::Search,
        &grant,
        &maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![maestria_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed {
            authorization,
            bounds,
        } => (authorization, bounds),
        FederatedGrantDecision::Denied(_) => return denied(),
    };
    let state = runtime(context)?.kernel_state().await;
    let manifest = load_manifest(context)?;
    let layout = context.layout.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        crate::prepare_search_runtime_read_only_for_federation(
            &layout,
            &state,
            &manifest,
            maestria_governance::RetrievalSecurityPolicy::default()
                .require_read_allowed(true)
                .allow_unscoped_items(true),
        )
    })
    .await
    .map_err(|error| anyhow!("prepare federation search runtime task failed: {error}"))??
    .without_graph_expansion();
    let (plan, outcome) = runtime
        .execute_pre_authorized(query, limit.min(bounds.max_results()), authorization)
        .await?;
    record_access(
        context,
        grant.token_digest().clone(),
        consumer_realm.clone(),
        FederatedAccessRecord::Search {
            query_id: plan.query_id(),
            trace_id: outcome.trace,
        },
    )
    .await?;
    Ok(ClientResponse::FederationSearch(FederationSearchResponse {
        provider_realm: context.realm_id.clone(),
        graph_degraded: true,
        search: super::search_services::search_response(
            plan.original_query().to_string(),
            plan.query_id().value(),
            outcome,
        ),
    }))
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
    let (authorization, bounds) = match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::OpenEvidence,
        &grant,
        &maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![maestria_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed {
            authorization,
            bounds,
        } => (authorization, bounds),
        FederatedGrantDecision::Denied(_) => return denied(),
    };
    let max_evidence_bytes = bounds
        .max_evidence_bytes()
        .min(super::super::MAX_REQUEST_BYTES - RESPONSE_EVIDENCE_RESERVE_BYTES);
    let layout = context.layout.clone();
    let mut output = tokio::task::spawn_blocking(move || {
        crate::evidence_open::open_evidence_scoped_with_authorization(
            &layout,
            evidence_id,
            authorization,
        )
    })
    .await
    .map_err(|error| anyhow!("federated evidence task failed: {error}"))??;
    truncate_utf8(&mut output.evidence.excerpt, max_evidence_bytes);
    let evidence = super::read_services::evidence_response(output)?;
    record_access(
        context,
        grant.token_digest().clone(),
        consumer_realm.clone(),
        FederatedAccessRecord::Evidence {
            evidence_id: maestria_domain::EvidenceId::new(evidence_id),
        },
    )
    .await?;
    Ok(ClientResponse::FederationEvidence(
        FederationEvidenceResponse {
            provider_realm: context.realm_id.clone(),
            evidence,
        },
    ))
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

fn load_manifest(context: &ApiContext) -> Result<maestria_core::InstanceManifest> {
    let content = std::fs::read_to_string(&context.layout.manifest_path).with_context(|| {
        format!(
            "read instance manifest {}",
            context.layout.manifest_path.display()
        )
    })?;
    maestria_core::InstanceManifest::decode(&content).context("decode instance manifest")
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

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    *value = maestria_ports::truncate_at_char_boundary(value, max_bytes).to_owned();
}
