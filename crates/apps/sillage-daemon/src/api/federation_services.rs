use anyhow::{Result, anyhow};
use sillage_domain::{
    CorpusScope, DomainInput, FederatedAccessRecord, FederatedReadOperation, GrantTokenDigest,
    RealmId, RecordFederatedAccessInput,
};
use sillage_governance::{FederatedGrantDecision, FederatedGrantDenial, authorize_federated_read};

use super::super::federation_previews::RESPONSE_EVIDENCE_RESERVE_BYTES;
use super::super::{
    ClientOperation, ClientResponse, FederationCredential, FederationEvidenceResponse,
};
use super::federation_binding::{self, FederationBinding};

use super::super::server::{ApiContext, InteractiveSearchControl, RequestPrincipal};

#[path = "federation_search_services.rs"]
mod search;
#[path = "federation_status_services.rs"]
mod status_services;

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
            search::serve_search(
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
    search::serve_search(
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
        &sillage_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![sillage_domain::DEFAULT_INSTANCE_SCOPE_ID]),
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
            evidence_id: sillage_domain::EvidenceId::new(evidence_id),
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
        &sillage_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![sillage_domain::DEFAULT_INSTANCE_SCOPE_ID]),
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
    status_services::status(context, consumer_realm, credential).await
}

pub(super) async fn indexing_status(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<super::super::protocol::SearchRootsStatusResponse> {
    status_services::indexing_status(context, consumer_realm, credential).await
}
pub(super) async fn source_revision(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<i64> {
    status_services::source_revision(context, consumer_realm, credential).await
}

async fn grant_for(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<sillage_domain::RealmReadGrant> {
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

fn runtime(context: &ApiContext) -> Result<&sillage_runtime::RuntimeHandle> {
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
    *value = sillage_ports::truncate_at_char_boundary(value, max_bytes).to_owned();
}
fn unix_time_seconds() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow!("read grant authorization clock: {error}"))?
        .as_secs())
}
