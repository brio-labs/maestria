use maestria_domain::{
    CorpusScope, FederatedEvidenceBounds, FederatedReadOperation, RealmId, RealmReadGrant,
    RealmReadGrantState,
};

use crate::{RetrievalAuthorizationContext, RetrievalAuthorizationError, RetrievalSecurityPolicy};

/// Result of the complete provider-side gate, before any candidate retrieval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FederatedGrantDecision {
    Allowed {
        authorization: RetrievalAuthorizationContext,
        bounds: FederatedEvidenceBounds,
    },
    Denied(FederatedGrantDenial),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FederatedGrantDenial {
    ProviderRealmMismatch,
    ConsumerRealmMismatch,
    GrantRevoked,
    UnsupportedAccess,
    InvalidBounds,
    ProviderPolicy(RetrievalAuthorizationError),
}

/// Authorizes a federated request strictly before retrieval/scoring.
///
/// The successful context retains every provider policy predicate and adds the
/// grant sensitivity ceiling by intersection. It must be supplied unchanged to
/// each enabled retrieval lane.
pub fn authorize_federated_read(
    provider_realm: &RealmId,
    consumer_realm: &RealmId,
    operation: FederatedReadOperation,
    grant: &RealmReadGrant,
    provider_policy: &RetrievalSecurityPolicy,
    corpus: &CorpusScope,
) -> FederatedGrantDecision {
    if grant.provider_realm() != provider_realm {
        return FederatedGrantDecision::Denied(FederatedGrantDenial::ProviderRealmMismatch);
    }
    if grant.consumer_realm() != consumer_realm {
        return FederatedGrantDecision::Denied(FederatedGrantDenial::ConsumerRealmMismatch);
    }
    if grant.state() != RealmReadGrantState::Active {
        return FederatedGrantDecision::Denied(FederatedGrantDenial::GrantRevoked);
    }
    if matches!(operation, FederatedReadOperation::OpenEvidence)
        && !grant.access().allows_evidence_open()
    {
        return FederatedGrantDecision::Denied(FederatedGrantDenial::UnsupportedAccess);
    }
    let bounds = match FederatedEvidenceBounds::try_new(
        grant.bounds().max_results(),
        grant.bounds().max_evidence_bytes(),
    ) {
        Ok(bounds) => bounds,
        Err(_) => return FederatedGrantDecision::Denied(FederatedGrantDenial::InvalidBounds),
    };
    let authorization = match provider_policy.authorization_context(corpus) {
        Ok(context) => context.with_sensitivity_ceiling(grant.max_sensitivity().clone()),
        Err(error) => {
            return FederatedGrantDecision::Denied(FederatedGrantDenial::ProviderPolicy(error));
        }
    };
    FederatedGrantDecision::Allowed {
        authorization,
        bounds,
    }
}
