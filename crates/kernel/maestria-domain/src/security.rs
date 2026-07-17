use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustZone {
    System,
    Verified,
    Untrusted,
    Quarantined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authority {
    System,
    User,
    Agent,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityState {
    Verified,
    Unverified,
    Compromised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewStatus {
    Approved,
    Unreviewed,
    Pending,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityMetadata {
    pub trust_zone: TrustZone,
    pub authority: Authority,
    pub integrity: IntegrityState,
    pub sensitivity: Sensitivity,
    pub review_status: ReviewStatus,
    pub quarantined: bool,
    pub prompt_injection_risk: bool,
    pub poisoning_flags: Vec<String>,
    pub read_allowed: bool,
    pub write_allowed: bool,
    pub scope_id: Option<crate::ids::ScopeId>,
}

impl Default for SecurityMetadata {
    fn default() -> Self {
        // Safe legacy default that does not lose information but doesn't assume high trust
        Self {
            trust_zone: TrustZone::Untrusted,
            authority: Authority::External,
            integrity: IntegrityState::Unverified,
            sensitivity: Sensitivity::Internal,
            review_status: ReviewStatus::Unreviewed,
            quarantined: false,
            prompt_injection_risk: false,
            poisoning_flags: Vec::new(),
            read_allowed: true,
            write_allowed: false,
            scope_id: None,
        }
    }
}

impl SecurityMetadata {
    pub fn from_optional(value: Option<Self>) -> Self {
        let mut sec = Self::default();
        if let Some(v) = value {
            sec = v;
        }
        sec
    }
    /// Merges two security metadatas, applying worst-case taint propagation.
    pub fn taint_from(&self, other: &Self) -> Self {
        let trust_zone = match (&self.trust_zone, &other.trust_zone) {
            (TrustZone::Quarantined, _) | (_, TrustZone::Quarantined) => TrustZone::Quarantined,
            (TrustZone::Untrusted, _) | (_, TrustZone::Untrusted) => TrustZone::Untrusted,
            (TrustZone::Verified, _) | (_, TrustZone::Verified) => TrustZone::Verified,
            _ => TrustZone::System,
        };

        // For authority, worst-case is External
        let authority = match (&self.authority, &other.authority) {
            (Authority::External, _) | (_, Authority::External) => Authority::External,
            (Authority::Agent, _) | (_, Authority::Agent) => Authority::Agent,
            (Authority::User, _) | (_, Authority::User) => Authority::User,
            _ => Authority::System,
        };

        let integrity = match (&self.integrity, &other.integrity) {
            (IntegrityState::Compromised, _) | (_, IntegrityState::Compromised) => {
                IntegrityState::Compromised
            }
            (IntegrityState::Unverified, _) | (_, IntegrityState::Unverified) => {
                IntegrityState::Unverified
            }
            _ => IntegrityState::Verified,
        };

        // For sensitivity, highest classification propagates
        let sensitivity = match (&self.sensitivity, &other.sensitivity) {
            (Sensitivity::Restricted, _) | (_, Sensitivity::Restricted) => Sensitivity::Restricted,
            (Sensitivity::Confidential, _) | (_, Sensitivity::Confidential) => {
                Sensitivity::Confidential
            }
            (Sensitivity::Internal, _) | (_, Sensitivity::Internal) => Sensitivity::Internal,
            _ => Sensitivity::Public,
        };

        let review_status = match (&self.review_status, &other.review_status) {
            (ReviewStatus::Rejected, _) | (_, ReviewStatus::Rejected) => ReviewStatus::Rejected,
            (ReviewStatus::Pending, _) | (_, ReviewStatus::Pending) => ReviewStatus::Pending,
            (ReviewStatus::Unreviewed, _) | (_, ReviewStatus::Unreviewed) => {
                ReviewStatus::Unreviewed
            }
            _ => ReviewStatus::Approved,
        };

        let mut poisoning_flags = self.poisoning_flags.clone();
        for flag in &other.poisoning_flags {
            if !poisoning_flags.contains(flag) {
                poisoning_flags.push(flag.clone());
            }
        }

        let scope_id = match (self.scope_id, other.scope_id) {
            (Some(left), Some(right)) if left == right => Some(left),
            (Some(_), Some(_)) => None,
            (Some(scope), None) | (None, Some(scope)) => Some(scope),
            (None, None) => None,
        };

        let quarantined =
            self.quarantined || other.quarantined || trust_zone == TrustZone::Quarantined;
        Self {
            trust_zone,
            authority,
            integrity,
            sensitivity,
            review_status,
            quarantined,
            prompt_injection_risk: self.prompt_injection_risk || other.prompt_injection_risk,
            poisoning_flags,
            read_allowed: self.read_allowed && other.read_allowed,
            write_allowed: self.write_allowed && other.write_allowed,
            scope_id,
        }
    }

    /// Denied/quarantined sources are not retrievable.
    pub fn retrieval_allowed(&self) -> bool {
        if !self.read_allowed {
            return false;
        }
        if self.quarantined || self.trust_zone == TrustZone::Quarantined {
            return false;
        }
        if self.review_status == ReviewStatus::Rejected {
            return false;
        }
        if self.integrity == IntegrityState::Compromised {
            return false;
        }
        true
    }

    /// Denied/quarantined/injection sources are not promotion-safe.
    pub fn memory_promotion_allowed(&self) -> bool {
        if !self.retrieval_allowed() {
            return false;
        }
        if matches!(
            self.trust_zone,
            TrustZone::Untrusted | TrustZone::Quarantined
        ) || self.authority == Authority::External
        {
            return false;
        }
        if self.integrity == IntegrityState::Compromised {
            return false;
        }
        if self.prompt_injection_risk {
            return false;
        }
        if !self.poisoning_flags.is_empty() {
            return false;
        }
        true
    }

    /// Action is authorized if not quarantined or rejected.
    pub fn action_authorized(&self) -> bool {
        if matches!(
            self.trust_zone,
            TrustZone::Untrusted | TrustZone::Quarantined
        ) || self.authority == Authority::External
        {
            return false;
        }
        self.write_allowed
            && self.retrieval_allowed()
            && self.integrity != IntegrityState::Compromised
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScopeId;

    #[test]
    fn test_legacy_default() {
        let sec = SecurityMetadata::default();
        assert_eq!(sec.trust_zone, TrustZone::Untrusted);
        assert_eq!(sec.authority, Authority::External);
        assert!(sec.read_allowed);
        assert!(!sec.write_allowed);
    }

    #[test]
    fn test_taint_propagation() {
        let sec1 = SecurityMetadata {
            trust_zone: TrustZone::Verified,
            authority: Authority::User,
            read_allowed: true,
            write_allowed: true,
            ..SecurityMetadata::default()
        };
        let sec2 = SecurityMetadata {
            trust_zone: TrustZone::Quarantined,
            read_allowed: false,
            ..SecurityMetadata::default()
        };

        let tainted = sec1.taint_from(&sec2);
        assert_eq!(tainted.trust_zone, TrustZone::Quarantined);
        assert_eq!(tainted.authority, Authority::External); // worst-case
        assert!(!tainted.read_allowed);
        assert!(!tainted.write_allowed);
        assert!(tainted.quarantined);
    }

    #[test]
    fn taint_inherits_unspecified_scope() {
        let source = SecurityMetadata {
            scope_id: Some(ScopeId::new(7)),
            ..SecurityMetadata::default()
        };
        assert_eq!(
            SecurityMetadata::default().taint_from(&source).scope_id,
            source.scope_id
        );
    }

    #[test]
    fn test_predicates() {
        let mut sec = SecurityMetadata::default();
        assert!(sec.retrieval_allowed());
        assert!(!sec.action_authorized()); // default has no write_allowed

        sec.write_allowed = true;
        sec.trust_zone = TrustZone::Verified;
        sec.authority = Authority::User;
        assert!(sec.action_authorized());

        sec.quarantined = true;
        assert!(!sec.retrieval_allowed());
        assert!(!sec.action_authorized());
        assert!(!sec.memory_promotion_allowed());
    }
}
