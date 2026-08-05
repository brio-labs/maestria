use crate::{
    EvidenceId, FederatedEvidenceBounds, GrantTokenDigest, QueryId, RealmId, SearchTraceId,
    Sensitivity,
};

/// The provider-authorized read surface for a consumer realm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederatedReadAccess {
    SearchOnly,
    SearchAndOpenEvidence,
}

impl FederatedReadAccess {
    pub const fn allows_evidence_open(self) -> bool {
        matches!(self, Self::SearchAndOpenEvidence)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmReadGrantState {
    Active,
    Revoked,
}

/// Records a completed federated operation without carrying query or evidence
/// content. The append-only event stream remains the audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederatedAccessRecord {
    Search {
        query_id: QueryId,
        trace_id: SearchTraceId,
    },
    Evidence {
        evidence_id: EvidenceId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederatedReadOperation {
    Search,
    OpenEvidence,
}

/// Current provider grant state, rebuilt from the append-only domain log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmReadGrant {
    token_digest: GrantTokenDigest,
    provider_realm: RealmId,
    consumer_realm: RealmId,
    access: FederatedReadAccess,
    max_sensitivity: Sensitivity,
    bounds: FederatedEvidenceBounds,
    state: RealmReadGrantState,
}

impl RealmReadGrant {
    pub fn new(
        token_digest: GrantTokenDigest,
        provider_realm: RealmId,
        consumer_realm: RealmId,
        access: FederatedReadAccess,
        max_sensitivity: Sensitivity,
        bounds: FederatedEvidenceBounds,
    ) -> Self {
        Self {
            token_digest,
            provider_realm,
            consumer_realm,
            access,
            max_sensitivity,
            bounds,
            state: RealmReadGrantState::Active,
        }
    }

    /// Reconstructs a validated current-state projection. Event issuance uses
    /// [`Self::new`] and is constrained to `Active`; this constructor exists
    /// only for rebuildable repository adapters.
    pub fn from_current_state(
        token_digest: GrantTokenDigest,
        provider_realm: RealmId,
        consumer_realm: RealmId,
        access: FederatedReadAccess,
        max_sensitivity: Sensitivity,
        bounds: FederatedEvidenceBounds,
        state: RealmReadGrantState,
    ) -> Self {
        Self {
            token_digest,
            provider_realm,
            consumer_realm,
            access,
            max_sensitivity,
            bounds,
            state,
        }
    }

    pub fn token_digest(&self) -> &GrantTokenDigest {
        &self.token_digest
    }

    pub fn provider_realm(&self) -> &RealmId {
        &self.provider_realm
    }

    pub fn consumer_realm(&self) -> &RealmId {
        &self.consumer_realm
    }

    pub const fn access(&self) -> FederatedReadAccess {
        self.access
    }

    pub fn max_sensitivity(&self) -> &Sensitivity {
        &self.max_sensitivity
    }

    pub const fn bounds(&self) -> FederatedEvidenceBounds {
        self.bounds
    }

    pub const fn state(&self) -> RealmReadGrantState {
        self.state
    }

    pub(crate) fn revoke(&mut self) {
        self.state = RealmReadGrantState::Revoked;
    }
}
