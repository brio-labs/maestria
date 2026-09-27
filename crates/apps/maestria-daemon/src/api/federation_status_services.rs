use anyhow::Result;
use maestria_domain::{CorpusScope, FederatedReadOperation, RealmId, RealmReadGrant};
use maestria_governance::{FederatedGrantDecision, authorize_federated_read};

use super::super::super::FederationCredential;
use super::super::super::protocol::{RetrievalStatusResponse, SearchRootsStatusResponse};
use super::super::super::server::ApiContext;

pub(super) async fn status(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<RetrievalStatusResponse> {
    authorized_status_grant(context, consumer_realm, credential).await?;
    super::super::search_services::retrieval_status(context).await
}

pub(super) async fn indexing_status(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<SearchRootsStatusResponse> {
    let grant = authorized_status_grant(context, consumer_realm, credential).await?;
    super::super::search_roots_services::status_for_roots(context, grant.allowed_roots()).await
}

async fn authorized_status_grant(
    context: &ApiContext,
    consumer_realm: &RealmId,
    credential: &FederationCredential,
) -> Result<RealmReadGrant> {
    let grant = super::grant_for(context, consumer_realm, credential).await?;
    match authorize_federated_read(
        &context.realm_id,
        consumer_realm,
        FederatedReadOperation::Search,
        &grant,
        super::unix_time_seconds()?,
        &maestria_governance::RetrievalSecurityPolicy::default()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        &CorpusScope::Restricted(vec![maestria_domain::DEFAULT_INSTANCE_SCOPE_ID]),
    ) {
        FederatedGrantDecision::Allowed { .. } => Ok(grant),
        FederatedGrantDecision::Denied(denial) => Err(super::grant_denial(denial)),
    }
}
