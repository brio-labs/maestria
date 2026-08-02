#[path = "model_agent_services.rs"]
mod model_agent_services;
#[path = "proposal_service.rs"]
mod proposal_service;
#[path = "read_services.rs"]
mod read_services;
#[path = "search_services.rs"]
mod search_services;
#[path = "support.rs"]
mod support;

use anyhow::{Result, anyhow};

use super::server::ApiContext;
use super::{ClientOperation, ClientResponse};

const MAX_SEARCH_LIMIT: usize = 100;

pub(crate) async fn dispatch(
    context: &ApiContext,
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
    }
}
