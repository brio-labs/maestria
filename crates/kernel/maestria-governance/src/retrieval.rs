use std::collections::BTreeSet;

use maestria_domain::{
    CorpusScope, RetrievalPolicySnapshot, ScopeId, SecurityMetadata, Sensitivity, TrustZone,
};

/// Policy decision on whether an item should be allowed in retrieval results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrievalDecision {
    Allowed,
    Denied(String),
}

/// Immutable authorization context bound to one validated search request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalAuthorizationContext {
    effective_scopes: Option<BTreeSet<ScopeId>>,
    require_read_allowed: bool,
    require_trust_zone: Option<TrustZone>,
    max_sensitivity: Option<Sensitivity>,
    allow_unscoped_items: bool,
    allow_quarantined: bool,
    allow_prompt_injection: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrievalAuthorizationError {
    ScopeDenied,
    InvalidPolicy(String),
}

impl std::fmt::Display for RetrievalAuthorizationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScopeDenied => formatter.write_str("retrieval scope denied"),
            Self::InvalidPolicy(reason) => write!(formatter, "invalid retrieval policy: {reason}"),
        }
    }
}

impl std::error::Error for RetrievalAuthorizationError {}

impl RetrievalAuthorizationContext {
    pub fn evaluate(&self, metadata: &SecurityMetadata) -> RetrievalDecision {
        if self.require_read_allowed && !metadata.read_allowed {
            return RetrievalDecision::Denied("Read not allowed by ACL".to_string());
        }
        if !self.allow_quarantined && metadata.quarantined() {
            return RetrievalDecision::Denied("Item is quarantined or rejected".to_string());
        }
        if metadata.review_status == maestria_domain::ReviewStatus::Rejected
            || metadata.integrity == maestria_domain::IntegrityState::Compromised
        {
            return RetrievalDecision::Denied("Item is quarantined or rejected".to_string());
        }
        if let Some(required) = &self.require_trust_zone
            && &metadata.trust_zone != required
        {
            return RetrievalDecision::Denied(format!(
                "Trust zone mismatch: expected {required:?}, found {:?}",
                metadata.trust_zone
            ));
        }
        if let Some(maximum) = &self.max_sensitivity
            && sensitivity_level(&metadata.sensitivity) > sensitivity_level(maximum)
        {
            return RetrievalDecision::Denied(format!(
                "Sensitivity too high: {:?}",
                metadata.sensitivity
            ));
        }
        if let Some(scopes) = &self.effective_scopes {
            match metadata.scope_id {
                Some(item_scope) if !scopes.contains(&item_scope) => {
                    return RetrievalDecision::Denied(format!(
                        "Scope mismatch: item scope {item_scope} is outside request scope"
                    ));
                }
                None if !self.allow_unscoped_items => {
                    return RetrievalDecision::Denied("Item has no scope_id".to_string());
                }
                _ => {}
            }
        }
        if !self.allow_prompt_injection && metadata.prompt_injection_risk {
            return RetrievalDecision::Denied("Prompt injection risk detected".to_string());
        }
        if !metadata.poisoning_flags.is_empty() {
            return RetrievalDecision::Denied("Poisoning flags detected".to_string());
        }
        RetrievalDecision::Allowed
    }

    pub fn effective_scopes(&self) -> Option<&BTreeSet<ScopeId>> {
        self.effective_scopes.as_ref()
    }

    pub fn policy_snapshot(&self) -> RetrievalPolicySnapshot {
        let effective_scopes = self
            .effective_scopes
            .as_ref()
            .map(|scopes| scopes.iter().copied().collect::<Vec<_>>());
        RetrievalPolicySnapshot {
            require_trust_zone: self.require_trust_zone.clone(),
            max_sensitivity: self.max_sensitivity.clone(),
            require_read_allowed: self.require_read_allowed,
            required_scope_id: effective_scopes
                .as_ref()
                .and_then(|scopes| (scopes.len() == 1).then_some(scopes[0])),
            effective_scopes,
            allow_unscoped_items: self.allow_unscoped_items,
        }
    }
}

/// Configuration used to derive a request-bound authorization context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalSecurityPolicy {
    pub require_trust_zone: Option<TrustZone>,
    pub max_sensitivity: Option<Sensitivity>,
    pub require_read_allowed: bool,
    pub required_scope_id: Option<ScopeId>,
    pub allow_unscoped_items: bool,
    pub instance_scope_ids: Option<BTreeSet<ScopeId>>,
}

impl Default for RetrievalSecurityPolicy {
    fn default() -> Self {
        Self {
            require_trust_zone: None,
            max_sensitivity: None,
            require_read_allowed: true,
            required_scope_id: None,
            allow_unscoped_items: false,
            instance_scope_ids: None,
        }
    }
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
    pub fn with_instance_scopes(mut self, scopes: impl IntoIterator<Item = ScopeId>) -> Self {
        self.instance_scope_ids = Some(scopes.into_iter().collect());
        self
    }

    pub fn authorization_context(
        &self,
        scope: &CorpusScope,
    ) -> Result<RetrievalAuthorizationContext, RetrievalAuthorizationError> {
        let plan_scopes: Option<BTreeSet<ScopeId>> = match scope {
            CorpusScope::Global => None,
            CorpusScope::Restricted(scopes) if scopes.is_empty() => {
                return Err(RetrievalAuthorizationError::ScopeDenied);
            }
            CorpusScope::Restricted(scopes) => Some(scopes.iter().copied().collect()),
        };
        let mut effective = match (plan_scopes, self.instance_scope_ids.clone()) {
            (Some(plan), Some(instance)) => Some(plan.intersection(&instance).copied().collect()),
            (Some(plan), None) => Some(plan),
            (None, Some(instance)) => Some(instance),
            (None, None) => None,
        };
        if let Some(required) = self.required_scope_id {
            let required_set = BTreeSet::from([required]);
            effective = Some(match effective {
                Some(existing) => existing.intersection(&required_set).copied().collect(),
                None => required_set,
            });
        }
        if effective.as_ref().is_some_and(BTreeSet::is_empty) {
            return Err(RetrievalAuthorizationError::ScopeDenied);
        }
        if effective.is_none() && !self.allow_unscoped_items && self.required_scope_id.is_some() {
            return Err(RetrievalAuthorizationError::ScopeDenied);
        }
        Ok(RetrievalAuthorizationContext {
            effective_scopes: effective,
            require_read_allowed: self.require_read_allowed,
            require_trust_zone: self.require_trust_zone.clone(),
            max_sensitivity: self.max_sensitivity.clone(),
            allow_unscoped_items: self.allow_unscoped_items,
            allow_quarantined: false,
            allow_prompt_injection: false,
        })
    }

    pub fn evaluate(&self, metadata: &SecurityMetadata) -> RetrievalDecision {
        let scope = self.required_scope_id.map_or(CorpusScope::Global, |scope| {
            CorpusScope::Restricted(vec![scope])
        });
        match self.authorization_context(&scope) {
            Ok(context) => context.evaluate(metadata),
            Err(error) => {
                RetrievalDecision::Denied(format!("retrieval scope is not authorized: {error}"))
            }
        }
    }

    pub fn policy_snapshot(&self) -> RetrievalPolicySnapshot {
        let effective_scopes = match (&self.instance_scope_ids, self.required_scope_id) {
            (Some(scopes), Some(required)) => Some(
                scopes
                    .intersection(&BTreeSet::from([required]))
                    .copied()
                    .collect::<Vec<_>>(),
            ),
            (Some(scopes), None) => Some(scopes.iter().copied().collect()),
            (None, Some(required)) => Some(vec![required]),
            (None, None) => None,
        };
        RetrievalPolicySnapshot {
            require_trust_zone: self.require_trust_zone.clone(),
            max_sensitivity: self.max_sensitivity.clone(),
            require_read_allowed: self.require_read_allowed,
            required_scope_id: effective_scopes
                .as_ref()
                .and_then(|scopes| (scopes.len() == 1).then_some(scopes[0])),
            effective_scopes,
            allow_unscoped_items: self.allow_unscoped_items,
        }
    }

    pub fn canonical_fingerprint(&self) -> String {
        self.policy_snapshot().canonical_fingerprint()
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
    use maestria_domain::{Authority, IntegrityState, ReviewStatus};

    fn metadata() -> SecurityMetadata {
        SecurityMetadata {
            trust_zone: TrustZone::Verified,
            authority: Authority::User,
            integrity: IntegrityState::Verified,
            sensitivity: Sensitivity::Internal,
            review_status: ReviewStatus::Approved,
            prompt_injection_risk: false,
            poisoning_flags: vec![],
            read_allowed: true,
            write_allowed: true,
            scope_id: Some(ScopeId::new(1)),
        }
    }

    #[test]
    fn default_is_acl_fail_closed() {
        let policy = RetrievalSecurityPolicy::default();
        let mut value = metadata();
        value.read_allowed = false;
        assert!(matches!(
            policy.evaluate(&value),
            RetrievalDecision::Denied(_)
        ));
    }

    #[test]
    fn mixed_scopes_intersect() {
        let policy = RetrievalSecurityPolicy::default()
            .with_instance_scopes([ScopeId::new(2), ScopeId::new(3)]);
        let result = policy.authorization_context(&CorpusScope::Restricted(vec![
            ScopeId::new(1),
            ScopeId::new(2),
        ]));
        assert!(result.is_ok());
        let Some(context) = result.ok() else {
            return;
        };
        assert_eq!(
            context.effective_scopes(),
            Some(&BTreeSet::from([ScopeId::new(2)]))
        );
    }
}
