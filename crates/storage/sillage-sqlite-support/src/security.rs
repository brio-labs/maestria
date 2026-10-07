/// The default `security_json` column value shared by every storage schema.
///
/// Mirrors `SecurityMetadata::default()` serialization; DDL sites interpolate
/// it as `DEFAULT '{DEFAULT_SECURITY_JSON}'` so the literal cannot drift
/// between projections.
pub const DEFAULT_SECURITY_JSON: &str = r#"{"trust_zone":"Untrusted","authority":"External","integrity":"Unverified","sensitivity":"Internal","review_status":"Unreviewed","prompt_injection_risk":false,"poisoning_flags":[],"read_allowed":true,"write_allowed":false,"scope_id":null}"#;
