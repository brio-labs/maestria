use maestria_domain::{
    Authority, IntegrityState, ReviewStatus, ScopeId, SecurityMetadata, Sensitivity, TrustZone,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredTrustZone {
    System,
    Verified,
    Untrusted,
    Quarantined,
}

impl StoredTrustZone {
    pub(crate) fn from_domain(value: &TrustZone) -> Self {
        match value {
            TrustZone::System => Self::System,
            TrustZone::Verified => Self::Verified,
            TrustZone::Untrusted => Self::Untrusted,
            TrustZone::Quarantined => Self::Quarantined,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<TrustZone, maestria_ports::PortError> {
        Ok(match self {
            Self::System => TrustZone::System,
            Self::Verified => TrustZone::Verified,
            Self::Untrusted => TrustZone::Untrusted,
            Self::Quarantined => TrustZone::Quarantined,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredAuthority {
    System,
    User,
    Agent,
    External,
}

impl StoredAuthority {
    pub(crate) fn from_domain(value: &Authority) -> Self {
        match value {
            Authority::System => Self::System,
            Authority::User => Self::User,
            Authority::Agent => Self::Agent,
            Authority::External => Self::External,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<Authority, maestria_ports::PortError> {
        Ok(match self {
            Self::System => Authority::System,
            Self::User => Authority::User,
            Self::Agent => Authority::Agent,
            Self::External => Authority::External,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredIntegrityState {
    Verified,
    Unverified,
    Compromised,
}

impl StoredIntegrityState {
    pub(crate) fn from_domain(value: &IntegrityState) -> Self {
        match value {
            IntegrityState::Verified => Self::Verified,
            IntegrityState::Unverified => Self::Unverified,
            IntegrityState::Compromised => Self::Compromised,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<IntegrityState, maestria_ports::PortError> {
        Ok(match self {
            Self::Verified => IntegrityState::Verified,
            Self::Unverified => IntegrityState::Unverified,
            Self::Compromised => IntegrityState::Compromised,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

impl StoredSensitivity {
    pub(crate) fn from_domain(value: &Sensitivity) -> Self {
        match value {
            Sensitivity::Public => Self::Public,
            Sensitivity::Internal => Self::Internal,
            Sensitivity::Confidential => Self::Confidential,
            Sensitivity::Restricted => Self::Restricted,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<Sensitivity, maestria_ports::PortError> {
        Ok(match self {
            Self::Public => Sensitivity::Public,
            Self::Internal => Sensitivity::Internal,
            Self::Confidential => Sensitivity::Confidential,
            Self::Restricted => Sensitivity::Restricted,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredReviewStatus {
    Approved,
    Unreviewed,
    Pending,
    Rejected,
}

impl StoredReviewStatus {
    pub(crate) fn from_domain(value: &ReviewStatus) -> Self {
        match value {
            ReviewStatus::Approved => Self::Approved,
            ReviewStatus::Unreviewed => Self::Unreviewed,
            ReviewStatus::Pending => Self::Pending,
            ReviewStatus::Rejected => Self::Rejected,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<ReviewStatus, maestria_ports::PortError> {
        Ok(match self {
            Self::Approved => ReviewStatus::Approved,
            Self::Unreviewed => ReviewStatus::Unreviewed,
            Self::Pending => ReviewStatus::Pending,
            Self::Rejected => ReviewStatus::Rejected,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSecurityMetadata {
    trust_zone: StoredTrustZone,
    authority: StoredAuthority,
    integrity: StoredIntegrityState,
    sensitivity: StoredSensitivity,
    review_status: StoredReviewStatus,
    prompt_injection_risk: bool,
    poisoning_flags: Vec<String>,
    read_allowed: bool,
    write_allowed: bool,
    scope_id: Option<u64>,
}

impl StoredSecurityMetadata {
    pub(crate) fn from_domain(value: &SecurityMetadata) -> Self {
        Self {
            trust_zone: StoredTrustZone::from_domain(&value.trust_zone),
            authority: StoredAuthority::from_domain(&value.authority),
            integrity: StoredIntegrityState::from_domain(&value.integrity),
            sensitivity: StoredSensitivity::from_domain(&value.sensitivity),
            review_status: StoredReviewStatus::from_domain(&value.review_status),
            prompt_injection_risk: value.prompt_injection_risk,
            poisoning_flags: value.poisoning_flags.clone(),
            read_allowed: value.read_allowed,
            write_allowed: value.write_allowed,
            scope_id: value.scope_id.map(|scope_id| scope_id.value()),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SecurityMetadata, maestria_ports::PortError> {
        Ok(SecurityMetadata {
            trust_zone: self.trust_zone.try_into_domain()?,
            authority: self.authority.try_into_domain()?,
            integrity: self.integrity.try_into_domain()?,
            sensitivity: self.sensitivity.try_into_domain()?,
            review_status: self.review_status.try_into_domain()?,
            prompt_injection_risk: self.prompt_injection_risk,
            poisoning_flags: self.poisoning_flags,
            read_allowed: self.read_allowed,
            write_allowed: self.write_allowed,
            scope_id: self.scope_id.map(ScopeId::new),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metadata() -> SecurityMetadata {
        SecurityMetadata {
            trust_zone: TrustZone::System,
            authority: Authority::Agent,
            integrity: IntegrityState::Verified,
            sensitivity: Sensitivity::Confidential,
            review_status: ReviewStatus::Approved,
            prompt_injection_risk: true,
            poisoning_flags: vec!["prompt-injection:v1".to_owned()],
            read_allowed: true,
            write_allowed: true,
            scope_id: Some(ScopeId::new(7)),
        }
    }

    #[test]
    fn security_metadata_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = sample_metadata();
        let stored = StoredSecurityMetadata::from_domain(&original);
        let restored = stored.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn security_metadata_round_trip_without_scope() -> Result<(), Box<dyn std::error::Error>> {
        let original = SecurityMetadata::default();
        let stored = StoredSecurityMetadata::from_domain(&original);
        let restored = stored.try_into_domain()?;
        assert_eq!(restored, original);
        assert_eq!(restored.scope_id, None);
        Ok(())
    }

    #[test]
    fn every_enum_variant_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let zones = [
            TrustZone::System,
            TrustZone::Verified,
            TrustZone::Untrusted,
            TrustZone::Quarantined,
        ];
        for zone in zones {
            assert_eq!(StoredTrustZone::from_domain(&zone).try_into_domain()?, zone);
        }
        let authorities = [
            Authority::System,
            Authority::User,
            Authority::Agent,
            Authority::External,
        ];
        for authority in authorities {
            assert_eq!(
                StoredAuthority::from_domain(&authority).try_into_domain()?,
                authority
            );
        }
        let states = [
            IntegrityState::Verified,
            IntegrityState::Unverified,
            IntegrityState::Compromised,
        ];
        for state in states {
            assert_eq!(
                StoredIntegrityState::from_domain(&state).try_into_domain()?,
                state
            );
        }
        let sensitivities = [
            Sensitivity::Public,
            Sensitivity::Internal,
            Sensitivity::Confidential,
            Sensitivity::Restricted,
        ];
        for sensitivity in sensitivities {
            assert_eq!(
                StoredSensitivity::from_domain(&sensitivity).try_into_domain()?,
                sensitivity
            );
        }
        let statuses = [
            ReviewStatus::Approved,
            ReviewStatus::Unreviewed,
            ReviewStatus::Pending,
            ReviewStatus::Rejected,
        ];
        for status in statuses {
            assert_eq!(
                StoredReviewStatus::from_domain(&status).try_into_domain()?,
                status
            );
        }
        Ok(())
    }

    #[test]
    fn scope_id_is_flattened_to_raw_u64() {
        let original = sample_metadata();
        let stored = StoredSecurityMetadata::from_domain(&original);
        assert_eq!(stored.scope_id, Some(7));
    }
}
