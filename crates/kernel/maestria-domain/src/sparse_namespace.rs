use serde::{Deserialize, Serialize};

use crate::TrustZone;

const MAX_NAMESPACE_COMPONENT_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SparseNamespace {
    instance_id: String,
    trust_zone: TrustZone,
    projection: String,
}

impl SparseNamespace {
    pub fn new(
        instance_id: impl Into<String>,
        trust_zone: TrustZone,
        projection: impl Into<String>,
    ) -> Result<Self, SparseNamespaceError> {
        let instance_id = instance_id.into();
        let projection = projection.into();
        validate_component("instance_id", &instance_id)?;
        validate_component("projection", &projection)?;
        Ok(Self {
            instance_id,
            trust_zone,
            projection,
        })
    }
    pub fn validate(&self) -> Result<(), SparseNamespaceError> {
        validate_component("instance_id", &self.instance_id)?;
        validate_component("projection", &self.projection)
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn trust_zone(&self) -> TrustZone {
        self.trust_zone.clone()
    }

    pub fn projection(&self) -> &str {
        &self.projection
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SparseNamespaceError {
    EmptyComponent { field: &'static str },
    ComponentTooLong { field: &'static str },
    ControlCharacter { field: &'static str },
}

impl std::fmt::Display for SparseNamespaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyComponent { field } => write!(formatter, "{field} must not be empty"),
            Self::ComponentTooLong { field } => write!(
                formatter,
                "{field} exceeds {MAX_NAMESPACE_COMPONENT_CHARS} characters"
            ),
            Self::ControlCharacter { field } => {
                write!(formatter, "{field} contains a control character")
            }
        }
    }
}

impl std::error::Error for SparseNamespaceError {}

fn validate_component(field: &'static str, value: &str) -> Result<(), SparseNamespaceError> {
    if value.trim().is_empty() {
        return Err(SparseNamespaceError::EmptyComponent { field });
    }
    if value.chars().count() > MAX_NAMESPACE_COMPONENT_CHARS {
        return Err(SparseNamespaceError::ComponentTooLong { field });
    }
    if value.chars().any(char::is_control) {
        return Err(SparseNamespaceError::ControlCharacter { field });
    }
    Ok(())
}
