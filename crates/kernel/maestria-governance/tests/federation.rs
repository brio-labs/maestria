use maestria_domain::{
    Authority, CorpusScope, FederatedEvidenceBounds, FederatedReadAccess, FederatedReadOperation,
    GrantTokenDigest, IntegrityState, RealmId, RealmReadGrant, ReviewStatus, ScopeId,
    SecurityMetadata, Sensitivity, TrustZone,
};
use maestria_governance::{
    FederatedGrantDecision, RetrievalDecision, RetrievalSecurityPolicy, authorize_federated_read,
};

fn realm(byte: char) -> Result<RealmId, Box<dyn std::error::Error>> {
    Ok(RealmId::try_from(byte.to_string().repeat(64))?)
}

fn grant(access: FederatedReadAccess) -> Result<RealmReadGrant, Box<dyn std::error::Error>> {
    Ok(RealmReadGrant::new(
        GrantTokenDigest::derive(b"grant"),
        realm('a')?,
        realm('b')?,
        access,
        Sensitivity::Confidential,
        FederatedEvidenceBounds::try_new(3, 128)?,
    ))
}

fn metadata() -> SecurityMetadata {
    SecurityMetadata {
        trust_zone: TrustZone::Verified,
        authority: Authority::User,
        integrity: IntegrityState::Verified,
        sensitivity: Sensitivity::Internal,
        review_status: ReviewStatus::Approved,
        prompt_injection_risk: false,
        poisoning_flags: Vec::new(),
        read_allowed: true,
        write_allowed: false,
        scope_id: Some(ScopeId::new(1)),
    }
}

fn authorization(
    policy: RetrievalSecurityPolicy,
    value: &RealmReadGrant,
) -> Result<maestria_governance::RetrievalAuthorizationContext, Box<dyn std::error::Error>> {
    match authorize_federated_read(
        &realm('a')?,
        &realm('b')?,
        FederatedReadOperation::Search,
        value,
        &policy,
        &CorpusScope::Global,
    ) {
        FederatedGrantDecision::Allowed { authorization, .. } => Ok(authorization),
        FederatedGrantDecision::Denied(_) => Err("expected a grant authorization decision".into()),
    }
}

#[test]
fn grant_sensitivity_cap_intersects_provider_policy() -> Result<(), Box<dyn std::error::Error>> {
    let grant = grant(FederatedReadAccess::SearchOnly)?;
    let authorization = authorization(
        RetrievalSecurityPolicy::new()
            .max_sensitivity(Sensitivity::Restricted)
            .allow_unscoped_items(true),
        &grant,
    )?;
    let mut restricted = metadata();
    restricted.sensitivity = Sensitivity::Restricted;
    assert!(matches!(
        authorization.evaluate(&restricted),
        RetrievalDecision::Denied(_)
    ));
    let mut confidential = metadata();
    confidential.sensitivity = Sensitivity::Confidential;
    assert!(matches!(
        authorization.evaluate(&confidential),
        RetrievalDecision::Allowed
    ));
    Ok(())
}

#[test]
fn existing_retrieval_predicates_deny_before_scoring() -> Result<(), Box<dyn std::error::Error>> {
    let grant = grant(FederatedReadAccess::SearchOnly)?;
    let policy = RetrievalSecurityPolicy::new()
        .require_trust_zone(TrustZone::Verified)
        .with_instance_scopes([ScopeId::new(1)])
        .allow_unscoped_items(false);
    let authorization = authorization(policy, &grant)?;

    let mut acl = metadata();
    acl.read_allowed = false;
    let mut trust = metadata();
    trust.trust_zone = TrustZone::Untrusted;
    let mut scope = metadata();
    scope.scope_id = Some(ScopeId::new(2));
    let mut quarantine = metadata();
    quarantine.trust_zone = TrustZone::Quarantined;
    let mut injection = metadata();
    injection.prompt_injection_risk = true;
    let mut poisoning = metadata();
    poisoning.poisoning_flags.push("tainted".to_string());

    for candidate in [acl, trust, scope, quarantine, injection, poisoning] {
        assert!(matches!(
            authorization.evaluate(&candidate),
            RetrievalDecision::Denied(_)
        ));
    }
    Ok(())
}

#[test]
fn rejects_wrong_realm_and_unsupported_evidence_access() -> Result<(), Box<dyn std::error::Error>> {
    let grant = grant(FederatedReadAccess::SearchOnly)?;
    assert!(matches!(
        authorize_federated_read(
            &realm('c')?,
            &realm('b')?,
            FederatedReadOperation::Search,
            &grant,
            &RetrievalSecurityPolicy::new(),
            &CorpusScope::Global,
        ),
        FederatedGrantDecision::Denied(_)
    ));
    assert!(matches!(
        authorize_federated_read(
            &realm('a')?,
            &realm('b')?,
            FederatedReadOperation::OpenEvidence,
            &grant,
            &RetrievalSecurityPolicy::new(),
            &CorpusScope::Global,
        ),
        FederatedGrantDecision::Denied(_)
    ));
    Ok(())
}
