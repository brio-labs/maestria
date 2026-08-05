use anyhow::{Result, anyhow};
use maestria_domain::{
    DomainInput, FederatedEvidenceBounds, FederatedReadAccess, GrantTokenDigest,
    IssueRealmReadGrantInput, RealmReadGrant, RealmReadGrantState, RevokeRealmReadGrantInput,
    Sensitivity,
};

use super::super::protocol::{
    ClientResponse, FederationCredential, RealmGrantAccess, RealmGrantCreatedResponse,
    RealmGrantListResponse, RealmGrantResponse, RealmGrantSensitivity,
};
use super::super::server::{ApiContext, RequestPrincipal};

pub(super) async fn create(
    context: &ApiContext,
    principal: &RequestPrincipal,
    consumer_realm: maestria_domain::RealmId,
    access: RealmGrantAccess,
    max_sensitivity: RealmGrantSensitivity,
    max_results: usize,
    max_evidence_bytes: usize,
) -> Result<ClientResponse> {
    require_instance(principal)?;
    let bounds = FederatedEvidenceBounds::try_new(max_results, max_evidence_bytes)
        .map_err(|error| anyhow!(error))?;
    let credential = generate_credential()?;
    let grant = RealmReadGrant::new(
        GrantTokenDigest::derive(credential.as_str().as_bytes()),
        context.realm_id.clone(),
        consumer_realm,
        domain_access(access),
        domain_sensitivity(max_sensitivity),
        bounds,
    );
    runtime(context)?
        .submit_durable(DomainInput::IssueRealmReadGrant(IssueRealmReadGrantInput {
            grant: grant.clone(),
        }))
        .await
        .map_err(|error| anyhow!(error))?;
    Ok(ClientResponse::RealmGrantCreated(
        RealmGrantCreatedResponse {
            grant: response_from_grant(&grant),
            credential,
        },
    ))
}

pub(super) fn list(context: &ApiContext, principal: &RequestPrincipal) -> Result<ClientResponse> {
    require_instance(principal)?;
    let mut grants = runtime(context)?
        .realm_read_grant_repository()
        .list()
        .map_err(|error| anyhow!(error))?;
    grants.sort_by(|left, right| {
        left.token_digest()
            .as_str()
            .cmp(right.token_digest().as_str())
    });
    Ok(ClientResponse::RealmGrantList(RealmGrantListResponse {
        grants: grants.iter().map(response_from_grant).collect(),
    }))
}

pub(super) async fn revoke(
    context: &ApiContext,
    principal: &RequestPrincipal,
    token_digest: String,
) -> Result<ClientResponse> {
    require_instance(principal)?;
    let token_digest = GrantTokenDigest::try_from(token_digest).map_err(|error| anyhow!(error))?;
    runtime(context)?
        .submit_durable(DomainInput::RevokeRealmReadGrant(
            RevokeRealmReadGrantInput { token_digest },
        ))
        .await
        .map_err(|error| anyhow!(error))?;
    Ok(ClientResponse::RealmGrantList(RealmGrantListResponse {
        grants: Vec::new(),
    }))
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
        Err(anyhow!(
            "realm grant administration requires instance authentication"
        ))
    }
}

fn generate_credential() -> Result<FederationCredential> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow!("generate federation credential: {error}"))?;
    let value = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    FederationCredential::try_from(value)
}

pub(super) fn response_from_grant(grant: &RealmReadGrant) -> RealmGrantResponse {
    RealmGrantResponse {
        token_digest: grant.token_digest().as_str().to_string(),
        provider_realm: grant.provider_realm().clone(),
        consumer_realm: grant.consumer_realm().clone(),
        access: protocol_access(grant.access()),
        max_sensitivity: protocol_sensitivity(grant.max_sensitivity()),
        max_results: grant.bounds().max_results(),
        max_evidence_bytes: grant.bounds().max_evidence_bytes(),
        state: match grant.state() {
            RealmReadGrantState::Active => "active".to_string(),
            RealmReadGrantState::Revoked => "revoked".to_string(),
        },
    }
}

fn domain_access(access: RealmGrantAccess) -> FederatedReadAccess {
    match access {
        RealmGrantAccess::SearchOnly => FederatedReadAccess::SearchOnly,
        RealmGrantAccess::SearchAndOpenEvidence => FederatedReadAccess::SearchAndOpenEvidence,
    }
}

fn protocol_access(access: FederatedReadAccess) -> RealmGrantAccess {
    match access {
        FederatedReadAccess::SearchOnly => RealmGrantAccess::SearchOnly,
        FederatedReadAccess::SearchAndOpenEvidence => RealmGrantAccess::SearchAndOpenEvidence,
    }
}

fn domain_sensitivity(sensitivity: RealmGrantSensitivity) -> Sensitivity {
    match sensitivity {
        RealmGrantSensitivity::Public => Sensitivity::Public,
        RealmGrantSensitivity::Internal => Sensitivity::Internal,
        RealmGrantSensitivity::Confidential => Sensitivity::Confidential,
        RealmGrantSensitivity::Restricted => Sensitivity::Restricted,
    }
}

fn protocol_sensitivity(sensitivity: &Sensitivity) -> RealmGrantSensitivity {
    match sensitivity {
        Sensitivity::Public => RealmGrantSensitivity::Public,
        Sensitivity::Internal => RealmGrantSensitivity::Internal,
        Sensitivity::Confidential => RealmGrantSensitivity::Confidential,
        Sensitivity::Restricted => RealmGrantSensitivity::Restricted,
    }
}
