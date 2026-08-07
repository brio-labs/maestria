#[path = "federation_binding.rs"]
mod federation_binding;
#[path = "federation_services.rs"]
mod federation_services;
#[path = "model_agent_services.rs"]
mod model_agent_services;
#[path = "notebook_services.rs"]
mod notebook_services;
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
        operation @ (ClientOperation::Status
        | ClientOperation::Task { .. }
        | ClientOperation::Evidence { .. }) => dispatch_read(context, operation).await,
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
        operation @ (ClientOperation::NotebookList
        | ClientOperation::NotebookCreate { .. }
        | ClientOperation::NotebookGet { .. }
        | ClientOperation::NotebookRename { .. }
        | ClientOperation::NotebookDelete { .. }
        | ClientOperation::NotebookSourceCatalog { .. }
        | ClientOperation::NotebookSourceAttach { .. }
        | ClientOperation::NotebookSourceDetach { .. }
        | ClientOperation::NotebookContext { .. }
        | ClientOperation::NotebookEvidence { .. }
        | ClientOperation::NotebookDraftList { .. }
        | ClientOperation::NotebookDraftGet { .. }
        | ClientOperation::NotebookDraftSave { .. }
        | ClientOperation::NotebookDraftDelete { .. }) => {
            dispatch_notebook(context, operation).await
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

async fn dispatch_read(context: &ApiContext, operation: ClientOperation) -> Result<ClientResponse> {
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
        _ => Err(anyhow!("invalid read operation")),
    }
}

async fn dispatch_notebook(
    context: &ApiContext,
    operation: ClientOperation,
) -> Result<ClientResponse> {
    match operation {
        ClientOperation::NotebookList => Ok(ClientResponse::NotebookList(
            notebook_services::list(context).await?,
        )),
        ClientOperation::NotebookCreate { title } => Ok(ClientResponse::Notebook(
            notebook_services::create(context, title).await?,
        )),
        ClientOperation::NotebookGet { notebook_id } => Ok(ClientResponse::Notebook(
            notebook_services::get(context, notebook_id).await?,
        )),
        ClientOperation::NotebookRename { notebook_id, title } => Ok(ClientResponse::Notebook(
            notebook_services::rename(context, notebook_id, title).await?,
        )),
        ClientOperation::NotebookDelete { notebook_id } => {
            notebook_services::delete(context, notebook_id).await?;
            Ok(ClientResponse::NotebookDeleted)
        }
        ClientOperation::NotebookSourceCatalog {
            query,
            offset,
            limit,
        } => Ok(ClientResponse::NotebookSources(
            notebook_services::source_catalog(context, query, offset, limit).await?,
        )),
        ClientOperation::NotebookSourceAttach {
            notebook_id,
            source_key,
        } => Ok(ClientResponse::Notebook(
            notebook_services::attach(context, notebook_id, source_key).await?,
        )),
        ClientOperation::NotebookSourceDetach {
            notebook_id,
            source_key,
        } => Ok(ClientResponse::Notebook(
            notebook_services::detach(context, notebook_id, source_key).await?,
        )),
        ClientOperation::NotebookContext {
            notebook_id,
            query,
            limit,
            max_context_bytes,
        } => Ok(ClientResponse::NotebookContext(
            notebook_services::context(context, notebook_id, query, limit, max_context_bytes)
                .await?,
        )),
        ClientOperation::NotebookEvidence {
            notebook_id,
            evidence_id,
        } => Ok(ClientResponse::NotebookEvidence(
            notebook_services::evidence(context, notebook_id, evidence_id).await?,
        )),
        ClientOperation::NotebookDraftList { notebook_id } => Ok(ClientResponse::NotebookDrafts(
            notebook_services::draft_list(context, notebook_id).await?,
        )),
        ClientOperation::NotebookDraftGet {
            notebook_id,
            draft_id,
        } => Ok(ClientResponse::NotebookDraft(
            notebook_services::draft_get(context, notebook_id, draft_id).await?,
        )),
        ClientOperation::NotebookDraftSave {
            notebook_id,
            draft_id,
            expected_revision,
            title,
            markdown,
            evidence_ids,
        } => Ok(ClientResponse::NotebookDraftSaved(
            notebook_services::draft_save(
                context,
                notebook_id,
                draft_id,
                expected_revision,
                title,
                markdown,
                evidence_ids,
            )
            .await?,
        )),
        ClientOperation::NotebookDraftDelete {
            notebook_id,
            draft_id,
            expected_revision,
        } => Ok(ClientResponse::NotebookDraftDeleted(
            notebook_services::draft_delete(context, notebook_id, draft_id, expected_revision)
                .await?,
        )),
        _ => Err(anyhow!("invalid notebook operation")),
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
