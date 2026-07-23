use maestria_domain::{EvidenceId, SearchTraceFilter, TrustZone};

use super::RetrievalSecurityValidator;
use super::Validator;

use crate::search_validator_fixtures::*;

#[test]
fn retrieval_security_validator_requires_required_filters() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    if let Some(trace) = fixture.outcome.trace_data.as_mut() {
        trace.filters = vec![SearchTraceFilter::Acl, SearchTraceFilter::Quarantine];
    }
    let check = RetrievalSecurityValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("required filter"));
    Ok(())
}

type SecurityMutation = fn(&mut maestria_domain::Evidence);
#[test]
fn security_validator_blocks_poisoning_prompt_injection_secret_acl_and_quarantine()
-> Result<(), Box<dyn std::error::Error>> {
    fn poison(evidence: &mut maestria_domain::Evidence) {
        evidence
            .security
            .poisoning_flags
            .push("graph_poisoning".to_string());
    }
    fn prompt_injection(evidence: &mut maestria_domain::Evidence) {
        evidence.security.prompt_injection_risk = true;
    }
    fn secret(evidence: &mut maestria_domain::Evidence) {
        evidence
            .security
            .poisoning_flags
            .push("secret_signal".to_string());
    }
    fn acl(evidence: &mut maestria_domain::Evidence) {
        evidence.security.read_allowed = false;
    }
    fn quarantine(evidence: &mut maestria_domain::Evidence) {
        evidence.security.quarantined = true;
        evidence.security.trust_zone = TrustZone::Quarantined;
    }

    let cases: [(&str, SecurityMutation); 5] = [
        ("poisoning", poison),
        ("prompt injection", prompt_injection),
        ("secret", secret),
        ("acl", acl),
        ("quarantine", quarantine),
    ];
    for (label, mutate) in cases {
        let mut fixture = fixture()?;
        let Some(evidence) = fixture.evidences.get_mut(&EvidenceId::new(10)) else {
            return Err(format!("fixture lost evidence for {label}").into());
        };
        mutate(evidence);
        let check = RetrievalSecurityValidator.validate(&fixture.context());
        assert!(!check.passed, "security case should fail: {label}");
        assert!(check.message.contains("1 denied candidate(s)"));
    }
    Ok(())
}

#[test]
fn security_validator_enforces_typed_policy_values() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture()?;
    if let Some(trace) = fixture.outcome.trace_data.as_mut() {
        trace.policy_fingerprint = Some(
            "trust=Some(Verified);sensitivity=Some(Public);read_allowed=true;scope=Some(ScopeId(999));unscoped=false"
                .to_string(),
        );
        trace.filters = vec![
            SearchTraceFilter::Acl,
            SearchTraceFilter::Trust,
            SearchTraceFilter::Sensitivity,
            SearchTraceFilter::Scope,
            SearchTraceFilter::Quarantine,
            SearchTraceFilter::PromptInjection,
            SearchTraceFilter::Freshness,
        ];
    }
    let check = RetrievalSecurityValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("denied candidate"));
    assert!(check.message.contains("1 denied candidate(s)"));
    Ok(())
}
