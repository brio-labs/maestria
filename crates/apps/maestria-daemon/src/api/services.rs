#[path = "federation_binding.rs"]
mod federation_binding;
#[path = "federation_services.rs"]
mod federation_services;
#[path = "model_agent_services.rs"]
mod model_agent_services;
#[path = "proposal_service.rs"]
mod proposal_service;
#[path = "read_services.rs"]
mod read_services;
#[path = "realm_grant_services.rs"]
mod realm_grant_services;
#[path = "search_services.rs"]
mod search_services;
#[path = "support.rs"]
mod support;

use anyhow::{Result, anyhow};

use super::server::{ApiContext, RequestPrincipal};
use super::{ClientOperation, ClientResponse};

const MAX_SEARCH_LIMIT: usize = 100;

pub(crate) async fn dispatch(
    context: &ApiContext,
    principal: RequestPrincipal,
    operation: ClientOperation,
) -> Result<ClientResponse> {
    match operation {
        ClientOperation::Status => {
            let layout = context.layout.clone();
            let socket_path = context.socket_path.clone();
            let response = support::run_database_retry("status", move || {
                read_services::status(&layout, &socket_path)
            })
            .await?;
            Ok(ClientResponse::Status(response))
        }
        ClientOperation::Task { task_id } => {
            let layout = context.layout.clone();
            let response =
                support::run_database_retry("task", move || read_services::task(&layout, task_id))
                    .await?;
            Ok(ClientResponse::Task(response))
        }
        ClientOperation::Evidence { evidence_id } => {
            let layout = context.layout.clone();
            let response = support::run_database_retry("evidence", move || {
                read_services::open_evidence(&layout, evidence_id)
            })
            .await?;
            Ok(ClientResponse::Evidence(response))
        }
        ClientOperation::Search { query, limit } => {
            if query.trim().is_empty() {
                return Err(anyhow!("search query must not be empty"));
            }
            if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
                return Err(anyhow!(
                    "search limit must be between 1 and {MAX_SEARCH_LIMIT}"
                ));
            }
            Ok(ClientResponse::Search(
                search_services::search_with_retry(context, query, limit).await?,
            ))
        }
        ClientOperation::ModelAgentPropose { proposal } => {
            model_agent_services::propose(context, proposal).await
        }
        ClientOperation::ModelAgentStatus { run_id } => {
            let layout = context.layout.clone();
            let response = support::run_database_retry("model-agent status", move || {
                proposal_service::status(&layout, run_id)
            })
            .await?;
            Ok(ClientResponse::ModelAgentStatus(response))
        }
        ClientOperation::ModelAgentResolve {
            run_id,
            approval_id,
            approved,
        } => model_agent_services::resolve(context, run_id, approval_id, approved).await,
        operation @ (ClientOperation::RealmGrantCreate { .. }
        | ClientOperation::RealmGrantList
        | ClientOperation::RealmGrantRevoke { .. }) => {
            dispatch_realm_grant(context, &principal, operation).await
        }
        ClientOperation::InstallFederationBinding {
            provider_realm,
            provider_socket_path,
            credential,
        } => {
            federation_services::install_binding(
                context,
                &principal,
                provider_realm,
                provider_socket_path,
                credential,
            )
            .await
        }
        ClientOperation::FederationSearch {
            provider_realm,
            query,
            limit,
        } => federation_services::search(context, &principal, provider_realm, query, limit).await,
        ClientOperation::FederationEvidence {
            provider_realm,
            evidence_id,
        } => federation_services::evidence(context, &principal, provider_realm, evidence_id).await,
    }
}

async fn dispatch_realm_grant(
    context: &ApiContext,
    principal: &RequestPrincipal,
    operation: ClientOperation,
) -> Result<ClientResponse> {
    match operation {
        ClientOperation::RealmGrantCreate {
            consumer_realm,
            access,
            max_sensitivity,
            max_results,
            max_evidence_bytes,
        } => {
            realm_grant_services::create(
                context,
                principal,
                consumer_realm,
                access,
                max_sensitivity,
                max_results,
                max_evidence_bytes,
            )
            .await
        }
        ClientOperation::RealmGrantList => realm_grant_services::list(context, principal),
        ClientOperation::RealmGrantRevoke { token_digest } => {
            realm_grant_services::revoke(context, principal, token_digest).await
        }
        _ => Err(anyhow!("invalid realm grant operation")),
    }
}
