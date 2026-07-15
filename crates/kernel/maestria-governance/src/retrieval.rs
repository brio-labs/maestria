use maestria_domain::{ScopeId, SecurityMetadata, Sensitivity, TrustZone};

/// Policy decision on whether an item should be allowed in retrieval results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrievalDecision {
    Allowed,
    Denied(String),
}

/// A policy that determines if a piece of retrieved evidence is allowed.
#[derive(Debug, Clone, Default)]
pub struct RetrievalSecurityPolicy {
    pub require_trust_zone: Option<TrustZone>,
    pub max_sensitivity: Option<Sensitivity>,
    pub require_read_allowed: bool,
    pub required_scope_id: Option<ScopeId>,
    /// Permit legacy records without scope metadata in an instance-local store.
    pub allow_unscoped_items: bool,
}

impl RetrievalSecurityPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn require_trust_zone(mut self, zone: TrustZone) -> Self {
        self.require_trust_zone = Some(zone);
        self
    }

    pub fn max_sensitivity(mut self, sensitivity: Sensitivity) -> Self {
        self.max_sensitivity = Some(sensitivity);
        self
    }

    pub fn require_read_allowed(mut self, require: bool) -> Self {
        self.require_read_allowed = require;
        self
    }

    pub fn required_scope(mut self, scope: ScopeId) -> Self {
        self.required_scope_id = Some(scope);
        self
    }

    pub fn allow_unscoped_items(mut self, allow: bool) -> Self {
        self.allow_unscoped_items = allow;
        self
    }
    /// Evaluates the metadata against the policy constraints.
    pub fn evaluate(&self, metadata: &SecurityMetadata) -> RetrievalDecision {
        // Core retrieval rule: always reject quarantined or rejected items
        if !metadata.retrieval_allowed() {
            return RetrievalDecision::Denied("Item is quarantined or rejected".into());
        }

        // Trust zone constraint
        if let Some(required_zone) = &self.require_trust_zone
            && &metadata.trust_zone != required_zone
        {
            return RetrievalDecision::Denied(format!(
                "Trust zone mismatch: expected {:?}, found {:?}",
                required_zone, metadata.trust_zone
            ));
        }

        // Sensitivity constraint
        if let Some(max_sens) = &self.max_sensitivity {
            let metadata_level = sensitivity_level(&metadata.sensitivity);
            let max_level = sensitivity_level(max_sens);
            if metadata_level > max_level {
                return RetrievalDecision::Denied(format!(
                    "Sensitivity too high: {:?}",
                    metadata.sensitivity
                ));
            }
        }

        // ACL and Scope constraints
        if self.require_read_allowed && !metadata.read_allowed {
            return RetrievalDecision::Denied("Read not allowed by ACL".into());
        }

        if let Some(req_scope) = &self.required_scope_id {
            if let Some(item_scope) = &metadata.scope_id {
                if item_scope != req_scope {
                    return RetrievalDecision::Denied(format!(
                        "Scope mismatch: expected {}, found {}",
                        req_scope, item_scope
                    ));
                }
            } else if !self.allow_unscoped_items {
                return RetrievalDecision::Denied("Item has no scope_id".into());
            }
        }

        // Prompt injection and poisoning
        if metadata.prompt_injection_risk {
            return RetrievalDecision::Denied("Prompt injection risk detected".into());
        }
        if !metadata.poisoning_flags.is_empty() {
            return RetrievalDecision::Denied("Poisoning flags detected".into());
        }

        RetrievalDecision::Allowed
    }
}

fn sensitivity_level(s: &Sensitivity) -> u8 {
    match s {
        Sensitivity::Public => 0,
        Sensitivity::Internal => 1,
        Sensitivity::Confidential => 2,
        Sensitivity::Restricted => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{Authority, IntegrityState, ReviewStatus, ScopeId};

    fn default_metadata() -> SecurityMetadata {
        SecurityMetadata {
            trust_zone: TrustZone::Verified,
            authority: Authority::User,
            integrity: IntegrityState::Verified,
            sensitivity: Sensitivity::Internal,
            review_status: ReviewStatus::Approved,
            quarantined: false,
            prompt_injection_risk: false,
            poisoning_flags: vec![],
            read_allowed: true,
            write_allowed: true,
            scope_id: Some(ScopeId::new(1)),
        }
    }

    #[test]
    fn test_quarantine_rejection() {
        let policy = RetrievalSecurityPolicy::new();
        let mut meta = default_metadata();
        meta.quarantined = true;

        assert!(matches!(
            policy.evaluate(&meta),
            RetrievalDecision::Denied(_)
        ));

        meta.quarantined = false;
        meta.trust_zone = TrustZone::Quarantined;
        assert!(matches!(
            policy.evaluate(&meta),
            RetrievalDecision::Denied(_)
        ));

        meta.trust_zone = TrustZone::Verified;
        meta.review_status = ReviewStatus::Rejected;
        assert!(matches!(
            policy.evaluate(&meta),
            RetrievalDecision::Denied(_)
        ));
    }

    #[test]
    fn test_acl_denial() {
        let policy = RetrievalSecurityPolicy::new().require_read_allowed(true);
        let mut meta = default_metadata();
        meta.read_allowed = false;
        assert!(matches!(
            policy.evaluate(&meta),
            RetrievalDecision::Denied(_)
        ));

        meta.read_allowed = true;
        assert_eq!(policy.evaluate(&meta), RetrievalDecision::Allowed);
    }

    #[test]
    fn test_prompt_injection_and_poisoning() {
        let policy = RetrievalSecurityPolicy::new();

        let mut meta = default_metadata();
        meta.prompt_injection_risk = true;
        assert!(matches!(
            policy.evaluate(&meta),
            RetrievalDecision::Denied(_)
        ));

        let mut meta2 = default_metadata();
        meta2.poisoning_flags = vec!["bad_data".into()];
        assert!(matches!(
            policy.evaluate(&meta2),
            RetrievalDecision::Denied(_)
        ));
    }

    #[test]
    fn test_sensitivity_levels() {
        let policy = RetrievalSecurityPolicy::new().max_sensitivity(Sensitivity::Internal);

        let mut meta = default_metadata();
        meta.sensitivity = Sensitivity::Public;
        assert_eq!(policy.evaluate(&meta), RetrievalDecision::Allowed);

        meta.sensitivity = Sensitivity::Internal;
        assert_eq!(policy.evaluate(&meta), RetrievalDecision::Allowed);

        meta.sensitivity = Sensitivity::Confidential;
        assert!(matches!(
            policy.evaluate(&meta),
            RetrievalDecision::Denied(_)
        ));
    }
}
