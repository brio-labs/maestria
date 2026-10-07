use std::path::PathBuf;

use crate::{
    EvidenceId, FederatedEvidenceBounds, GrantTokenDigest, QueryId, RealmId, SearchTraceId,
    Sensitivity,
};
/// Maximum number of roots frozen into a provider-issued read grant.
pub const MAX_REALM_GRANT_ROOTS: usize = 64;
/// Maximum combined byte length of those roots, keeping protocol responses bounded.
pub const MAX_REALM_GRANT_ROOT_BYTES: usize = 8 * 1024;

/// Absolute UTC expiry for a provider-issued read grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RealmReadGrantExpiry(u64);

impl RealmReadGrantExpiry {
    pub fn new(unix_seconds: u64) -> Result<Self, RealmReadGrantExpiryError> {
        if unix_seconds == 0 {
            return Err(RealmReadGrantExpiryError::Zero);
        }
        Ok(Self(unix_seconds))
    }

    pub const fn unix_seconds(self) -> u64 {
        self.0
    }

    pub const fn is_expired_at(self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmReadGrantExpiryError {
    Zero,
}

impl std::fmt::Display for RealmReadGrantExpiryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Zero => formatter.write_str("realm read grant expiry must be non-zero"),
        }
    }
}

impl std::error::Error for RealmReadGrantExpiryError {}

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
    expires_at: RealmReadGrantExpiry,
    allowed_roots: Option<Vec<PathBuf>>,
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
        expires_at: RealmReadGrantExpiry,
    ) -> Self {
        Self {
            token_digest,
            provider_realm,
            consumer_realm,
            access,
            max_sensitivity,
            bounds,
            expires_at,
            allowed_roots: None,
            state: RealmReadGrantState::Active,
        }
    }

    /// Restricts this grant to a frozen set of provider-approved roots.
    ///
    /// `None` remains reserved for legacy grants that predate root scoping.
    pub fn with_allowed_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.allowed_roots = Some(roots);
        self
    }

    /// Reconstructs a validated current-state projection. Event issuance uses
    /// [`Self::new`] and is constrained to `Active`; this constructor exists
    /// only for rebuildable repository adapters.
    pub fn from_current_state(mut grant: Self, state: RealmReadGrantState) -> Self {
        grant.state = state;
        grant
    }
}
impl RealmReadGrant {
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

    pub const fn expires_at(&self) -> RealmReadGrantExpiry {
        self.expires_at
    }

    /// Returns `None` for legacy grants or the frozen explicit root scope.
    pub fn allowed_roots(&self) -> Option<&[PathBuf]> {
        self.allowed_roots.as_deref()
    }

    pub const fn state(&self) -> RealmReadGrantState {
        self.state
    }

    pub(crate) fn revoke(&mut self) {
        self.state = RealmReadGrantState::Revoked;
    }
}
