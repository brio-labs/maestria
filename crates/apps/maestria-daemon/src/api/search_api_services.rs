use anyhow::{Result, anyhow};
use maestria_domain::RealmId;

use super::super::protocol::{
    ClientResponse, FederationCredential, FederationEvidenceResponse, FederationSearchResponse,
};
use super::super::protocol_search_api::{SearchApiOperation, SearchApiResponse};
use super::super::server::{ApiContext, RequestPrincipal};
use super::federation_services;

pub(super) async fn dispatch(
    context: &ApiContext,
    consumer_realm: RealmId,
    credential: FederationCredential,
    operation: SearchApiOperation,
    interactive: Option<super::super::server::InteractiveSearchControl>,
) -> Result<SearchApiResponse> {
    let principal = RequestPrincipal::Federation {
        consumer_realm: consumer_realm.clone(),
        credential: credential.clone(),
    };
    match operation {
        SearchApiOperation::Search { query, limit } => {
            match federation_services::search(
                context,
                &principal,
                context.realm_id.clone(),
                query,
                limit,
            )
            .await?
            {
                ClientResponse::FederationSearch(FederationSearchResponse { search, .. }) => {
                    Ok(SearchApiResponse::Search(search))
                }
                _ => Err(anyhow!(
                    "federated search returned an incompatible response"
                )),
            }
        }
        SearchApiOperation::InteractiveSearch { query, limit } => {
            let control = interactive
                .ok_or_else(|| anyhow!("interactive search cancellation control is unavailable"))?;
            match federation_services::interactive_search(
                context,
                &principal,
                context.realm_id.clone(),
                query,
                limit,
                control,
            )
            .await?
            {
                ClientResponse::FederationSearch(FederationSearchResponse { search, .. }) => {
                    Ok(SearchApiResponse::Search(search))
                }
                _ => Err(anyhow!(
                    "federated interactive search returned an incompatible response"
                )),
            }
        }
        SearchApiOperation::Status => Ok(SearchApiResponse::Status(Box::new(
            federation_services::status(context, &consumer_realm, &credential).await?,
        ))),
        SearchApiOperation::IndexingStatus => {
            let status =
                federation_services::indexing_status(context, &consumer_realm, &credential).await?;
            let mut exclusions = std::collections::BTreeMap::new();
            for root in &status.roots {
                for (reason, count) in &root.exclusions_by_reason {
                    *exclusions.entry(reason.clone()).or_default() += *count;
                }
            }
            Ok(SearchApiResponse::IndexingStatus(Box::new(
                super::super::protocol_search_api::SearchApiIndexingStatusResponse {
                    approved_root_count: status.approved_root_count,
                    indexed_file_count: status
                        .roots
                        .iter()
                        .map(|root| root.indexed_file_count)
                        .sum(),
                    inventory_truncated: status.exclusion_scan_truncated,
                    ocr_needed_file_count: status.ocr_needed_file_count,
                    excluded_file_count: status
                        .roots
                        .iter()
                        .map(|root| root.excluded_file_count)
                        .sum(),
                    exclusions_by_reason: exclusions,
                    supported_formats: status.supported_formats,
                    ignored_by_default: status.ignored_by_default,
                    scanning: status.indexing.scanning,
                    pending_file_count: status.indexing.pending_file_count,
                    last_scan_unix_ms: status.indexing.last_scan_unix_ms,
                    last_scan_error: status.indexing.last_error.is_some(),
                },
            )))
        }
        SearchApiOperation::Evidence { evidence_id } => {
            match federation_services::evidence(
                context,
                &principal,
                context.realm_id.clone(),
                evidence_id,
            )
            .await?
            {
                ClientResponse::FederationEvidence(FederationEvidenceResponse {
                    evidence, ..
                }) => Ok(SearchApiResponse::Evidence(evidence)),
                _ => Err(anyhow!(
                    "federated evidence returned an incompatible response"
                )),
            }
        }
    }
}
