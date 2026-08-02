use crate::{ScopeId, Sensitivity, TrustZone};

/// Immutable, typed representation of the retrieval policy recorded in a search trace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetrievalPolicySnapshot {
    pub require_trust_zone: Option<TrustZone>,
    pub max_sensitivity: Option<Sensitivity>,
    pub require_read_allowed: bool,
    /// Complete effective scope set. `None` is global; `Some` is restricted.
    pub effective_scopes: Option<Vec<ScopeId>>,
    pub allow_unscoped_items: bool,
}

impl RetrievalPolicySnapshot {
    pub fn effective_scope_set(&self) -> Option<&[ScopeId]> {
        self.effective_scopes.as_deref()
    }

    pub fn global_default() -> Self {
        Self {
            require_trust_zone: None,
            max_sensitivity: None,
            require_read_allowed: true,
            effective_scopes: None,
            allow_unscoped_items: false,
        }
    }

    pub fn restricted(scopes: impl IntoIterator<Item = ScopeId>) -> Self {
        let mut effective_scopes = scopes.into_iter().collect::<Vec<_>>();
        effective_scopes.sort();
        effective_scopes.dedup();
        Self {
            require_trust_zone: None,
            max_sensitivity: None,
            require_read_allowed: true,
            effective_scopes: Some(effective_scopes),
            allow_unscoped_items: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrievalPolicySnapshotError {
    Empty,
    UnknownField(String),
    DuplicateField(String),
    InvalidValue { field: String, value: String },
    MissingField(String),
}

impl RetrievalPolicySnapshot {
    pub fn canonical_fingerprint(&self) -> String {
        format!(
            "trust={};sensitivity={};read_allowed={};scope={};unscoped={}",
            option_trust(self.require_trust_zone.as_ref()),
            option_sensitivity(self.max_sensitivity.as_ref()),
            self.require_read_allowed,
            option_scopes(self.effective_scopes.as_deref()),
            self.allow_unscoped_items,
        )
    }

    pub fn from_canonical(value: &str) -> Result<Self, RetrievalPolicySnapshotError> {
        if value.trim().is_empty() {
            return Err(RetrievalPolicySnapshotError::Empty);
        }
        let mut trust = None;
        let mut sensitivity = None;
        let mut read_allowed = None;
        let mut scope = None;
        let mut unscoped = None;
        let mut seen = std::collections::BTreeSet::new();
        for field in value.split(';') {
            let (name, raw) = field.split_once('=').ok_or_else(|| {
                RetrievalPolicySnapshotError::InvalidValue {
                    field: field.to_string(),
                    value: String::new(),
                }
            })?;
            if !seen.insert(name) {
                return Err(RetrievalPolicySnapshotError::DuplicateField(
                    name.to_string(),
                ));
            }
            match name {
                "trust" => trust = Some(parse_trust(raw)?),
                "sensitivity" => sensitivity = Some(parse_sensitivity(raw)?),
                "read_allowed" => read_allowed = Some(parse_bool(name, raw)?),
                // `scope=Restricted(...)` carries the complete ordered set.
                "scope" => scope = Some(parse_scopes(raw)?),
                "unscoped" => unscoped = Some(parse_bool(name, raw)?),
                other => {
                    return Err(RetrievalPolicySnapshotError::UnknownField(
                        other.to_string(),
                    ));
                }
            }
        }
        let effective_scopes = scope.ok_or_else(|| missing("scope"))?;
        Ok(Self {
            require_trust_zone: trust.ok_or_else(|| missing("trust"))?,
            max_sensitivity: sensitivity.ok_or_else(|| missing("sensitivity"))?,
            require_read_allowed: read_allowed.ok_or_else(|| missing("read_allowed"))?,
            effective_scopes,
            allow_unscoped_items: unscoped.ok_or_else(|| missing("unscoped"))?,
        })
    }
}

fn missing(field: &str) -> RetrievalPolicySnapshotError {
    RetrievalPolicySnapshotError::MissingField(field.to_string())
}
fn parse_bool(field: &str, value: &str) -> Result<bool, RetrievalPolicySnapshotError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(RetrievalPolicySnapshotError::InvalidValue {
            field: field.to_string(),
            value: value.to_string(),
        }),
    }
}
fn parse_trust(value: &str) -> Result<Option<TrustZone>, RetrievalPolicySnapshotError> {
    match value {
        "None" => Ok(None),
        "Some(System)" => Ok(Some(TrustZone::System)),
        "Some(Verified)" => Ok(Some(TrustZone::Verified)),
        "Some(Untrusted)" => Ok(Some(TrustZone::Untrusted)),
        "Some(Quarantined)" => Ok(Some(TrustZone::Quarantined)),
        _ => Err(RetrievalPolicySnapshotError::InvalidValue {
            field: "trust".to_string(),
            value: value.to_string(),
        }),
    }
}
fn parse_sensitivity(value: &str) -> Result<Option<Sensitivity>, RetrievalPolicySnapshotError> {
    match value {
        "None" => Ok(None),
        "Some(Public)" => Ok(Some(Sensitivity::Public)),
        "Some(Internal)" => Ok(Some(Sensitivity::Internal)),
        "Some(Confidential)" => Ok(Some(Sensitivity::Confidential)),
        "Some(Restricted)" => Ok(Some(Sensitivity::Restricted)),
        _ => Err(RetrievalPolicySnapshotError::InvalidValue {
            field: "sensitivity".to_string(),
            value: value.to_string(),
        }),
    }
}
fn option_trust(value: Option<&TrustZone>) -> String {
    value.map_or_else(|| "None".to_string(), |zone| format!("Some({zone:?})"))
}

fn option_sensitivity(value: Option<&Sensitivity>) -> String {
    value.map_or_else(
        || "None".to_string(),
        |sensitivity| format!("Some({sensitivity:?})"),
    )
}

fn parse_scopes(value: &str) -> Result<Option<Vec<ScopeId>>, RetrievalPolicySnapshotError> {
    if value == "None" {
        return Ok(None);
    }
    let values = value
        .strip_prefix("Restricted(")
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| RetrievalPolicySnapshotError::InvalidValue {
            field: "scope".to_string(),
            value: value.to_string(),
        })?
        .split(',')
        .map(|raw| {
            raw.strip_prefix("ScopeId(")
                .and_then(|value| value.strip_suffix(')'))
                .ok_or_else(|| RetrievalPolicySnapshotError::InvalidValue {
                    field: "scope".to_string(),
                    value: raw.to_string(),
                })?
                .parse::<u64>()
                .map(ScopeId::new)
                .map_err(|_| RetrievalPolicySnapshotError::InvalidValue {
                    field: "scope".to_string(),
                    value: raw.to_string(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() || values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(RetrievalPolicySnapshotError::InvalidValue {
            field: "scope".to_string(),
            value: value.to_string(),
        });
    }
    Ok(Some(values))
}
fn option_scopes(value: Option<&[ScopeId]>) -> String {
    match value {
        None => "None".to_string(),
        Some(scopes) => format!(
            "Restricted({})",
            scopes
                .iter()
                .map(|scope| format!("ScopeId({scope})"))
                .collect::<Vec<_>>()
                .join(","),
        ),
    }
}
