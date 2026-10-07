use crate::{ScopeId, Sensitivity, TrustZone};

/// Immutable, validated representation of the retrieval policy recorded in a
/// search trace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "RetrievalPolicySnapshotDto")]
pub struct RetrievalPolicySnapshot {
    require_trust_zone: Option<TrustZone>,
    max_sensitivity: Option<Sensitivity>,
    require_read_allowed: bool,
    /// Complete effective scope set. `None` is global; `Some` is restricted.
    effective_scopes: Option<Vec<ScopeId>>,
    allow_unscoped_items: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrievalPolicySnapshotDto {
    require_trust_zone: Option<TrustZone>,
    max_sensitivity: Option<Sensitivity>,
    require_read_allowed: bool,
    effective_scopes: Option<Vec<ScopeId>>,
    allow_unscoped_items: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrievalPolicySnapshotError {
    Empty,
    UnknownField(String),
    DuplicateField(String),
    InvalidValue { field: String, value: String },
    MissingField(String),
    InvalidScopeSet,
}

impl std::fmt::Display for RetrievalPolicySnapshotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("policy snapshot is empty"),
            Self::UnknownField(field) => write!(formatter, "unknown policy field: {field}"),
            Self::DuplicateField(field) => write!(formatter, "duplicate policy field: {field}"),
            Self::InvalidValue { field, value } => {
                write!(formatter, "invalid value for policy field {field}: {value}")
            }
            Self::MissingField(field) => write!(formatter, "missing policy field: {field}"),
            Self::InvalidScopeSet => formatter.write_str(
                "effective scopes must be absent or a strictly increasing non-empty list",
            ),
        }
    }
}

impl std::error::Error for RetrievalPolicySnapshotError {}

impl TryFrom<RetrievalPolicySnapshotDto> for RetrievalPolicySnapshot {
    type Error = RetrievalPolicySnapshotError;

    fn try_from(dto: RetrievalPolicySnapshotDto) -> Result<Self, Self::Error> {
        Self::try_new(
            dto.require_trust_zone,
            dto.max_sensitivity,
            dto.require_read_allowed,
            dto.effective_scopes,
            dto.allow_unscoped_items,
        )
    }
}

impl RetrievalPolicySnapshot {
    pub fn try_new(
        require_trust_zone: Option<TrustZone>,
        max_sensitivity: Option<Sensitivity>,
        require_read_allowed: bool,
        effective_scopes: Option<Vec<ScopeId>>,
        allow_unscoped_items: bool,
    ) -> Result<Self, RetrievalPolicySnapshotError> {
        if let Some(scopes) = &effective_scopes
            && (scopes.is_empty() || scopes.windows(2).any(|pair| pair[0] >= pair[1]))
        {
            return Err(RetrievalPolicySnapshotError::InvalidScopeSet);
        }
        Ok(Self {
            require_trust_zone,
            max_sensitivity,
            require_read_allowed,
            effective_scopes,
            allow_unscoped_items,
        })
    }

    pub fn require_trust_zone(&self) -> Option<&TrustZone> {
        self.require_trust_zone.as_ref()
    }

    pub fn max_sensitivity(&self) -> Option<&Sensitivity> {
        self.max_sensitivity.as_ref()
    }

    pub const fn requires_read_allowed(&self) -> bool {
        self.require_read_allowed
    }

    pub fn effective_scopes(&self) -> Option<&[ScopeId]> {
        self.effective_scopes.as_deref()
    }

    pub const fn allows_unscoped_items(&self) -> bool {
        self.allow_unscoped_items
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

    pub fn restricted(
        scopes: impl IntoIterator<Item = ScopeId>,
    ) -> Result<Self, RetrievalPolicySnapshotError> {
        let mut effective_scopes = scopes.into_iter().collect::<Vec<_>>();
        effective_scopes.sort();
        effective_scopes.dedup();
        Self::try_new(None, None, true, Some(effective_scopes), false)
    }

    pub fn with_allow_unscoped_items(
        self,
        allow_unscoped_items: bool,
    ) -> Result<Self, RetrievalPolicySnapshotError> {
        Self::try_new(
            self.require_trust_zone,
            self.max_sensitivity,
            self.require_read_allowed,
            self.effective_scopes,
            allow_unscoped_items,
        )
    }
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
        Self::try_new(
            trust.ok_or_else(|| missing("trust"))?,
            sensitivity.ok_or_else(|| missing("sensitivity"))?,
            read_allowed.ok_or_else(|| missing("read_allowed"))?,
            effective_scopes,
            unscoped.ok_or_else(|| missing("unscoped"))?,
        )
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
