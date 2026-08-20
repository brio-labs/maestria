use crate::evidence_source::EvidenceKind;
use crate::ids::{ArtifactId, BlobId, EvidenceId, LogicalTick, NotebookDraftId, NotebookId};
use crate::search::ContentHash;
use std::collections::BTreeSet;
use std::fmt;

const MAX_KEY_BYTES: usize = 4096;
const MAX_TITLE_BYTES: usize = 200;
const MAX_CITATION_EXCERPT_BYTES: usize = 2048;
const MAX_CITATIONS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotebookValueError {
    Empty(&'static str),
    TooLong {
        field: &'static str,
        max_bytes: usize,
    },
    ZeroRevision,
    RevisionOverflow,
    ExcerptTooLong,
    TooManyCitations,
}

impl fmt::Display for NotebookValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(field) => write!(f, "{field} must not be empty"),
            Self::TooLong { field, max_bytes } => {
                write!(f, "{field} must be at most {max_bytes} bytes")
            }
            Self::ZeroRevision => f.write_str("draft revision must start at one"),
            Self::RevisionOverflow => f.write_str("draft revision overflowed"),
            Self::ExcerptTooLong => write!(
                f,
                "citation excerpt must be at most {MAX_CITATION_EXCERPT_BYTES} bytes"
            ),
            Self::TooManyCitations => write!(f, "at most {MAX_CITATIONS} citations are supported"),
        }
    }
}

impl std::error::Error for NotebookValueError {}

macro_rules! validated_text {
    ($name:ident, $field:literal, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl TryFrom<String> for $name {
            type Error = NotebookValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err(NotebookValueError::Empty($field));
                }
                if trimmed.len() > $max {
                    return Err(NotebookValueError::TooLong {
                        field: $field,
                        max_bytes: $max,
                    });
                }
                Ok(Self(trimmed.to_owned()))
            }
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

validated_text!(SourceIdentityKey, "source identity key", MAX_KEY_BYTES);
validated_text!(NotebookTitle, "notebook title", MAX_TITLE_BYTES);
validated_text!(NotebookDraftTitle, "notebook draft title", MAX_TITLE_BYTES);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NotebookDraftRevision(u64);

impl NotebookDraftRevision {
    pub const fn initial() -> Self {
        Self(1)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub fn increment(self) -> Result<Self, NotebookValueError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(NotebookValueError::RevisionOverflow)
    }
}

impl TryFrom<u64> for NotebookDraftRevision {
    type Error = NotebookValueError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(NotebookValueError::ZeroRevision)
        } else {
            Ok(Self(value))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notebook {
    pub id: NotebookId,
    pub title: NotebookTitle,
    pub source_keys: BTreeSet<SourceIdentityKey>,
    pub created_at: LogicalTick,
    pub updated_at: LogicalTick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenNotebookCitation {
    pub evidence_id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub artifact_title: String,
    pub artifact_content_hash: ContentHash,
    pub source: EvidenceKind,
    pub excerpt: String,
    pub observed_at: LogicalTick,
}

impl FrozenNotebookCitation {
    pub fn validate(&self) -> Result<(), NotebookValueError> {
        if self.excerpt.len() > MAX_CITATION_EXCERPT_BYTES {
            return Err(NotebookValueError::ExcerptTooLong);
        }
        Ok(())
    }
}

pub fn validate_frozen_citations(
    citations: &[FrozenNotebookCitation],
) -> Result<(), NotebookValueError> {
    if citations.len() > MAX_CITATIONS {
        return Err(NotebookValueError::TooManyCitations);
    }
    for citation in citations {
        citation.validate()?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookDraft {
    pub id: NotebookDraftId,
    pub notebook_id: NotebookId,
    pub title: NotebookDraftTitle,
    pub body_blob: BlobId,
    pub body_hash: ContentHash,
    pub revision: NotebookDraftRevision,
    pub citations: Vec<FrozenNotebookCitation>,
    pub created_at: LogicalTick,
    pub updated_at: LogicalTick,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_values_trim_and_enforce_byte_limits() -> Result<(), NotebookValueError> {
        assert_eq!(
            NotebookTitle::try_from("  title  ".to_owned())?.as_str(),
            "title"
        );
        assert!(NotebookTitle::try_from(" ".to_owned()).is_err());
        assert!(SourceIdentityKey::try_from("x".repeat(MAX_KEY_BYTES + 1)).is_err());
        Ok(())
    }

    #[test]
    fn revisions_start_at_one_and_increment_checked() -> Result<(), NotebookValueError> {
        assert_eq!(NotebookDraftRevision::initial().value(), 1);
        assert_eq!(
            NotebookDraftRevision::try_from(u64::MAX)?.increment(),
            Err(NotebookValueError::RevisionOverflow)
        );
        assert!(NotebookDraftRevision::try_from(0).is_err());
        Ok(())
    }
}
