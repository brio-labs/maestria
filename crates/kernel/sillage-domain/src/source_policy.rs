//! Pure per-source policy values with validating constructors and deserialization.
//!
//! Scope is lexical policy data: it does not inspect the filesystem or authorize access.
//! Choices describe processing and disclosure intent; they do not execute effects or create grants.

#[path = "source_policy_budget.rs"]
mod budget;
#[cfg(test)]
#[path = "source_policy_tests.rs"]
mod tests;
pub use budget::SourceResourceBudget;

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SourceScopeDto")]
pub struct SourceScope {
    path: PathBuf,
    selection: SourceSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceSelection {
    File,
    Folder { recursive: bool },
}

#[derive(Deserialize)]
struct SourceScopeDto {
    path: PathBuf,
    selection: SourceSelection,
}

impl SourceScope {
    pub fn file(path: PathBuf) -> Result<Self, SourceChoiceError> {
        Self::try_new(path, SourceSelection::File)
    }

    pub fn folder(path: PathBuf, recursive: bool) -> Result<Self, SourceChoiceError> {
        Self::try_new(path, SourceSelection::Folder { recursive })
    }

    pub fn try_new(path: PathBuf, selection: SourceSelection) -> Result<Self, SourceChoiceError> {
        if !valid_absolute_path(&path) {
            return Err(SourceChoiceError::InvalidScope);
        }
        Ok(Self { path, selection })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn selection(&self) -> SourceSelection {
        self.selection
    }
}

impl TryFrom<SourceScopeDto> for SourceScope {
    type Error = SourceChoiceError;

    fn try_from(dto: SourceScopeDto) -> Result<Self, Self::Error> {
        Self::try_new(dto.path, dto.selection)
    }
}

fn valid_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && !path.as_os_str().is_empty()
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputationTiming {
    OnDemand,
    BackgroundWhenIdle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceLocality {
    LocalOnly,
    LocalWithNamedRemoteProvider { provider_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SourceRepresentation {
    OriginalBytes,
    ExtractedText,
    LexicalIndex,
    OcrText,
    Embedding,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteRetentionChoice {
    NoRetention,
    ProviderRetentionAccepted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDisclosureConsent {
    pub provider_id: String,
    pub representation: SourceRepresentation,
    pub retention: RemoteRetentionChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentaryRetentionChoice {
    NoDocumentaryPersistence,
    RetainUntilExplicitlyPurged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlPersistenceChoice {
    SessionOnly,
    PersistExplicitly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceAvailability {
    Enabled,
    Paused,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SourceChoiceParameters")]
pub struct SourceChoice {
    scope: SourceScope,
    timing: ComputationTiming,
    locality: SourceLocality,
    documentary_retention: DocumentaryRetentionChoice,
    control_persistence: ControlPersistenceChoice,
    availability: SourceAvailability,
    representations: Vec<SourceRepresentation>,
    remote_disclosures: Vec<RemoteDisclosureConsent>,
    budget: SourceResourceBudget,
}

#[derive(Deserialize)]
pub struct SourceChoiceParameters {
    pub scope: SourceScope,
    pub timing: ComputationTiming,
    pub locality: SourceLocality,
    pub documentary_retention: DocumentaryRetentionChoice,
    pub control_persistence: ControlPersistenceChoice,
    pub availability: SourceAvailability,
    pub representations: Vec<SourceRepresentation>,
    pub remote_disclosures: Vec<RemoteDisclosureConsent>,
    pub budget: SourceResourceBudget,
}

impl SourceChoice {
    pub fn try_new(parameters: SourceChoiceParameters) -> Result<Self, SourceChoiceError> {
        let SourceChoiceParameters {
            scope,
            timing,
            locality,
            documentary_retention,
            control_persistence,
            availability,
            representations,
            remote_disclosures,
            budget,
        } = parameters;
        if representations.is_empty() || representations.len() > 6 {
            return Err(SourceChoiceError::InvalidRepresentations);
        }
        if has_duplicates(&representations) {
            return Err(SourceChoiceError::InvalidRepresentations);
        }
        let remote_provider = match &locality {
            SourceLocality::LocalOnly if remote_disclosures.is_empty() => None,
            SourceLocality::LocalOnly => return Err(SourceChoiceError::DisclosureWithoutProvider),
            SourceLocality::LocalWithNamedRemoteProvider { provider_id }
                if !provider_id.trim().is_empty() =>
            {
                Some(provider_id)
            }
            SourceLocality::LocalWithNamedRemoteProvider { .. } => {
                return Err(SourceChoiceError::InvalidProvider);
            }
        };
        for consent in &remote_disclosures {
            if !representations.contains(&consent.representation)
                || remote_provider.is_none_or(|provider| provider != &consent.provider_id)
                || consent.provider_id.trim().is_empty()
                || !matches!(
                    consent.representation,
                    SourceRepresentation::OcrText | SourceRepresentation::Embedding
                )
            {
                return Err(SourceChoiceError::InvalidDisclosure);
            }
        }
        if has_duplicate_disclosures(&remote_disclosures) {
            return Err(SourceChoiceError::InvalidDisclosure);
        }
        Ok(Self {
            scope,
            timing,
            locality,
            documentary_retention,
            control_persistence,
            availability,
            representations,
            remote_disclosures,
            budget,
        })
    }

    pub fn scope(&self) -> &SourceScope {
        &self.scope
    }

    pub const fn timing(&self) -> ComputationTiming {
        self.timing
    }

    pub fn locality(&self) -> &SourceLocality {
        &self.locality
    }

    pub const fn documentary_retention(&self) -> DocumentaryRetentionChoice {
        self.documentary_retention
    }

    pub const fn control_persistence(&self) -> ControlPersistenceChoice {
        self.control_persistence
    }

    pub const fn availability(&self) -> SourceAvailability {
        self.availability
    }

    pub fn set_availability(&mut self, availability: SourceAvailability) {
        self.availability = availability;
    }

    pub fn representations(&self) -> &[SourceRepresentation] {
        &self.representations
    }

    pub fn remote_disclosures(&self) -> &[RemoteDisclosureConsent] {
        &self.remote_disclosures
    }

    pub const fn budget(&self) -> SourceResourceBudget {
        self.budget
    }
}

impl TryFrom<SourceChoiceParameters> for SourceChoice {
    type Error = SourceChoiceError;

    fn try_from(parameters: SourceChoiceParameters) -> Result<Self, Self::Error> {
        Self::try_new(parameters)
    }
}

fn has_duplicates<T: PartialEq>(items: &[T]) -> bool {
    items
        .iter()
        .enumerate()
        .any(|(index, item)| items[..index].contains(item))
}

fn has_duplicate_disclosures(items: &[RemoteDisclosureConsent]) -> bool {
    items.iter().enumerate().any(|(index, item)| {
        items[..index].iter().any(|previous| {
            previous.provider_id == item.provider_id
                && previous.representation == item.representation
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceChoiceError {
    InvalidScope,
    InvalidBudget,
    InvalidRepresentations,
    InvalidProvider,
    DisclosureWithoutProvider,
    InvalidDisclosure,
}

impl std::fmt::Display for SourceChoiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidScope => "source scope must be absolute and normalized",
            Self::InvalidBudget => "source resource budget is outside supported limits",
            Self::InvalidRepresentations => "source representations must be nonempty and unique",
            Self::InvalidProvider => "remote provider identity must not be empty",
            Self::DisclosureWithoutProvider => "remote disclosure requires a named provider choice",
            Self::InvalidDisclosure => "remote disclosure does not match an explicit source choice",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for SourceChoiceError {}
