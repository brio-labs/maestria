//! Validated code-marker record types and the marker query surface.
//!
//! `CodeMarker` is the persisted marker shape carried by `SymbolMarkers`
//! (todo/fixme/hack comments with validated, one-based inclusive source
//! ranges). `MarkerQueryKind` is the query-side kind selector: it may also
//! ask for `unsafe`, which persisted markers never carry — the matcher maps
//! that kind to `UnsafeBlock` symbols and unsafe-bearing declarations.

use crate::types::{SymbolKind, SymbolRecord};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Kind of a raw-text comment marker (serde snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeMarkerKind {
    Todo,
    Fixme,
    Hack,
}

/// One todo/fixme/hack comment with a validated, one-based inclusive source
/// range (mirrors the `SourceRange` boundary pattern: private fields, a
/// fallible constructor, and a serde DTO conversion so persisted markers can
/// never bypass validation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CodeMarkerDto")]
pub struct CodeMarker {
    kind: CodeMarkerKind,
    start_line: usize,
    end_line: usize,
}

impl CodeMarker {
    /// Builds a comment marker spanning `start_line..=end_line`.
    pub fn new(
        kind: CodeMarkerKind,
        start_line: usize,
        end_line: usize,
    ) -> Result<Self, CodeMarkerError> {
        if start_line == 0 {
            return Err(CodeMarkerError::StartMustBePositive);
        }
        if start_line > end_line {
            return Err(CodeMarkerError::StartAfterEnd {
                start_line,
                end_line,
            });
        }
        Ok(Self {
            kind,
            start_line,
            end_line,
        })
    }

    pub const fn kind(&self) -> CodeMarkerKind {
        self.kind
    }

    pub const fn start_line(&self) -> usize {
        self.start_line
    }

    pub const fn end_line(&self) -> usize {
        self.end_line
    }
}

/// Failure while building a validated [`CodeMarker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeMarkerError {
    StartMustBePositive,
    StartAfterEnd { start_line: usize, end_line: usize },
}

impl std::fmt::Display for CodeMarkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartMustBePositive => {
                write!(f, "code marker start line must be positive")
            }
            Self::StartAfterEnd {
                start_line,
                end_line,
            } => write!(
                f,
                "code marker start line {start_line} must not exceed end line {end_line}"
            ),
        }
    }
}

impl std::error::Error for CodeMarkerError {}

#[derive(Deserialize)]
struct CodeMarkerDto {
    kind: CodeMarkerKind,
    start_line: usize,
    end_line: usize,
}

impl TryFrom<CodeMarkerDto> for CodeMarker {
    type Error = CodeMarkerError;

    fn try_from(dto: CodeMarkerDto) -> Result<Self, Self::Error> {
        Self::new(dto.kind, dto.start_line, dto.end_line)
    }
}

/// Query-side marker kind selector. `Unsafe` is not a persisted
/// [`CodeMarkerKind`]; the matcher resolves it to `UnsafeBlock` symbols and
/// unsafe-bearing declarations (`is_unsafe`) instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerQueryKind {
    Todo,
    Fixme,
    Hack,
    Unsafe,
}

/// Failure while parsing a [`MarkerQueryKind`] from user input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerQueryKindParseError {
    input: String,
}

impl std::fmt::Display for MarkerQueryKindParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown marker kind {:?}; expected one of todo, fixme, hack, unsafe",
            self.input
        )
    }
}

impl std::error::Error for MarkerQueryKindParseError {}

impl FromStr for MarkerQueryKind {
    type Err = MarkerQueryKindParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "todo" => Ok(Self::Todo),
            "fixme" => Ok(Self::Fixme),
            "hack" => Ok(Self::Hack),
            "unsafe" => Ok(Self::Unsafe),
            _ => Err(MarkerQueryKindParseError {
                input: value.to_string(),
            }),
        }
    }
}

impl SymbolRecord {
    /// Whether this symbol carries a marker of `kind` under the marker-query
    /// semantics shared by `CodeQuery::Markers` and the benchmark runner:
    /// Todo/Fixme/Hack match by presence in `markers.code_markers`; Unsafe
    /// matches `UnsafeBlock` symbols and unsafe-bearing declarations.
    pub fn has_marker(&self, kind: MarkerQueryKind) -> bool {
        match kind {
            MarkerQueryKind::Todo => self
                .markers
                .code_markers
                .iter()
                .any(|marker| marker.kind() == CodeMarkerKind::Todo),
            MarkerQueryKind::Fixme => self
                .markers
                .code_markers
                .iter()
                .any(|marker| marker.kind() == CodeMarkerKind::Fixme),
            MarkerQueryKind::Hack => self
                .markers
                .code_markers
                .iter()
                .any(|marker| marker.kind() == CodeMarkerKind::Hack),
            MarkerQueryKind::Unsafe => self.kind == SymbolKind::UnsafeBlock || self.is_unsafe,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_marker_rejects_invalid_lines() {
        assert_eq!(
            CodeMarker::new(CodeMarkerKind::Todo, 0, 1),
            Err(CodeMarkerError::StartMustBePositive)
        );
        assert_eq!(
            CodeMarker::new(CodeMarkerKind::Fixme, 4, 2),
            Err(CodeMarkerError::StartAfterEnd {
                start_line: 4,
                end_line: 2
            })
        );
        assert!(CodeMarker::new(CodeMarkerKind::Hack, 7, 9).is_ok());
        if let Ok(marker) = CodeMarker::new(CodeMarkerKind::Hack, 7, 9) {
            assert_eq!(marker.kind(), CodeMarkerKind::Hack);
            assert_eq!(marker.start_line(), 7);
            assert_eq!(marker.end_line(), 9);
        }
    }

    #[test]
    fn marker_query_kind_parses_case_insensitively() {
        assert_eq!("todo".parse(), Ok(MarkerQueryKind::Todo));
        assert_eq!("fixme".parse(), Ok(MarkerQueryKind::Fixme));
        assert_eq!("Hack".parse(), Ok(MarkerQueryKind::Hack));
        assert_eq!("UNSAFE".parse(), Ok(MarkerQueryKind::Unsafe));
        assert!(matches!(
            "marker".parse::<MarkerQueryKind>(),
            Err(MarkerQueryKindParseError { .. })
        ));
        assert!(matches!(
            "".parse::<MarkerQueryKind>(),
            Err(MarkerQueryKindParseError { .. })
        ));
    }
}
