use std::error::Error;

use maestria_domain::{RetrievalPolicySnapshot, ScopeId, Sensitivity, TrustZone};

#[test]
fn try_new_rejects_invalid_effective_scope_sets() -> Result<(), Box<dyn Error>> {
    for scopes in [
        Vec::new(),
        vec![ScopeId::new(2), ScopeId::new(1)],
        vec![ScopeId::new(1), ScopeId::new(1)],
    ] {
        if RetrievalPolicySnapshot::try_new(None, None, true, Some(scopes), false).is_ok() {
            return Err("invalid effective scopes were accepted".into());
        }
    }
    Ok(())
}

#[test]
fn serde_deserialization_validates_effective_scope_sets() -> Result<(), Box<dyn Error>> {
    let invalid = r#"{
        "require_trust_zone": null,
        "max_sensitivity": null,
        "require_read_allowed": true,
        "effective_scopes": [],
        "allow_unscoped_items": false
    }"#;
    let decoded = serde_json::from_str::<RetrievalPolicySnapshot>(invalid);
    if decoded.is_ok() {
        return Err("serde accepted an empty effective scope set".into());
    }
    Ok(())
}

#[test]
fn valid_snapshot_round_trips_through_serde() -> Result<(), Box<dyn Error>> {
    let snapshot = RetrievalPolicySnapshot::try_new(
        Some(TrustZone::Verified),
        Some(Sensitivity::Internal),
        true,
        Some(vec![ScopeId::new(1), ScopeId::new(3)]),
        false,
    )?;
    let encoded = serde_json::to_string(&snapshot)?;
    let decoded = serde_json::from_str::<RetrievalPolicySnapshot>(&encoded)?;
    if decoded != snapshot {
        return Err("validated policy snapshot changed during serde round-trip".into());
    }
    Ok(())
}
