use anyhow::{Result, anyhow, bail};

use maestria_domain::{DomainInput, RetrievalEventsRetired};

use super::super::protocol::{ClientResponse, RetrievalEventsRetiredResponse};
use super::super::server::{ApiContext, RequestPrincipal};

pub(super) async fn retire(
    context: &ApiContext,
    principal: &RequestPrincipal,
    before_sequence: u64,
    reason: String,
) -> Result<ClientResponse> {
    require_instance(principal)?;
    if before_sequence == 0 {
        bail!("--before-sequence must be greater than zero");
    }
    if reason.trim().is_empty() {
        bail!(
            "--reason is required: the retirement marker must record why the audit trail narrows"
        );
    }
    runtime(context)?
        .submit_durable(DomainInput::RetrievalEventsRetired(
            RetrievalEventsRetired {
                before_sequence,
                reason: reason.clone(),
            },
        ))
        .await
        .map_err(|error| anyhow!(error))?;
    // The runtime state handle is a startup snapshot; the durable high-water
    // is read back from the recorded markers instead.
    let database_path = context.layout.database_path.clone();
    let retired_through =
        super::support::run_database_retry("retire retrieval events", move || {
            let store = maestria_storage_sqlite::SqliteStore::open_read_only(&database_path)
                .map_err(|error| anyhow!(error))?;
            store
                .retrieval_retired_through()
                .map_err(|error| anyhow!(error))
        })
        .await?;
    Ok(ClientResponse::RetrievalEventsRetired(
        RetrievalEventsRetiredResponse { retired_through },
    ))
}

fn runtime(context: &ApiContext) -> Result<&maestria_runtime::RuntimeHandle> {
    context
        .runtime
        .as_ref()
        .ok_or_else(|| anyhow!("retrieval event retirement requires a live daemon runtime"))
}

fn require_instance(principal: &RequestPrincipal) -> Result<()> {
    if matches!(principal, RequestPrincipal::Instance) {
        Ok(())
    } else {
        Err(anyhow!(
            "retrieval event retirement requires instance authentication"
        ))
    }
}
#[cfg(test)]
#[path = "retention_services_tests.rs"]
mod tests;
