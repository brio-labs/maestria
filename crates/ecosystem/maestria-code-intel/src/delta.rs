//! Validated changed-file delta record.
//!
//! [`RepositoryChangeDelta`] owns the files/symbols consistency invariant:
//! the symbol list is derived from the file set and the index symbols by
//! [`RepositoryChangeDelta::from_parts`], and persisted deltas are
//! re-validated on load through the DTO conversion (every symbol must belong
//! to a listed file).

use crate::types::SymbolRecord;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Files and symbols that changed between the build-time baseline and the
/// current repository state, computed from porcelain status and git history
/// metadata only (zero content reads).
///
/// The symbol list is derived from the file set and the index symbols by
/// [`RepositoryChangeDelta::from_parts`], so files/symbols consistency is
/// by construction; persisted deltas are re-validated on load through the
/// DTO conversion (every symbol must belong to a listed file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RepositoryChangeDeltaDto")]
pub struct RepositoryChangeDelta {
    /// Relative repository paths in the changed set, sorted.
    files: Vec<String>,
    /// `record_id`s of symbols whose file is in `files`, ordered by file
    /// then qualified name.
    symbols: Vec<String>,
}

impl RepositoryChangeDelta {
    /// Build the delta from a changed file set and the indexed symbols. The
    /// symbol list is derived here, so an inconsistent files/symbols pair
    /// cannot be constructed.
    pub fn from_parts(files: BTreeSet<String>, symbols: &[SymbolRecord]) -> Self {
        Self {
            files: files.iter().cloned().collect(),
            symbols: derive_delta_symbols(&files, symbols),
        }
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }
}

/// Failure while decoding a persisted [`RepositoryChangeDelta`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryChangeDeltaError {
    EmptyFile,
    EmptySymbol,
    SymbolWithoutFile,
}

impl std::fmt::Display for RepositoryChangeDeltaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyFile => write!(f, "changed delta lists an empty file path"),
            Self::EmptySymbol => write!(f, "changed delta lists an empty symbol id"),
            Self::SymbolWithoutFile => {
                write!(
                    f,
                    "changed delta lists a symbol whose file is not in the file set"
                )
            }
        }
    }
}

impl std::error::Error for RepositoryChangeDeltaError {}

#[derive(Deserialize)]
struct RepositoryChangeDeltaDto {
    files: Vec<String>,
    symbols: Vec<String>,
}

impl TryFrom<RepositoryChangeDeltaDto> for RepositoryChangeDelta {
    type Error = RepositoryChangeDeltaError;

    fn try_from(dto: RepositoryChangeDeltaDto) -> Result<Self, Self::Error> {
        for file in &dto.files {
            if file.is_empty() {
                return Err(RepositoryChangeDeltaError::EmptyFile);
            }
        }
        for symbol in &dto.symbols {
            if symbol.is_empty() {
                return Err(RepositoryChangeDeltaError::EmptySymbol);
            }
            // record_id format is "{file_path}:{kind}:{name}:{start}-{end}";
            // the file prefix match keeps paths containing ':' unambiguous.
            let in_files = dto
                .files
                .iter()
                .any(|file| symbol.starts_with(&format!("{file}:")));
            if !in_files {
                return Err(RepositoryChangeDeltaError::SymbolWithoutFile);
            }
        }
        Ok(Self {
            files: dto.files,
            symbols: dto.symbols,
        })
    }
}

/// `record_id`s of symbols whose file is in `files`, ordered by file then
/// qualified name (deterministic regardless of input symbol order).
pub(crate) fn derive_delta_symbols(
    files: &BTreeSet<String>,
    symbols: &[SymbolRecord],
) -> Vec<String> {
    let mut by_name: Vec<(&str, &str, &str)> = symbols
        .iter()
        .filter(|symbol| files.contains(&symbol.provenance.file_path))
        .map(|symbol| {
            (
                symbol.provenance.file_path.as_str(),
                symbol.qualified_name.as_str(),
                symbol.record_id.as_str(),
            )
        })
        .collect();
    by_name.sort_by(|left, right| left.0.cmp(right.0).then_with(|| left.1.cmp(right.1)));
    by_name
        .into_iter()
        .map(|(_, _, record_id)| record_id.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CommitSha, ParserGeneration, RecordProvenance, SourceRange, SourceRangeError, SymbolKind,
        SymbolMarkers, SymbolRecord, Visibility, WorktreeIdentity,
    };
    use std::collections::BTreeSet;
    use std::error::Error;

    #[test]
    fn change_delta_from_parts_derives_consistent_symbols() -> Result<(), Box<dyn Error>> {
        let files = BTreeSet::from(["src/a.rs".to_string(), "src/b.rs".to_string()]);
        let symbols = vec![
            symbol_with_file("src/a.rs", "a_one")?,
            symbol_with_file("src/b.rs", "b_one")?,
            symbol_with_file("src/a.rs", "a_two")?,
            symbol_with_file("src/c.rs", "c_one")?,
        ];
        let delta = RepositoryChangeDelta::from_parts(files, &symbols);
        assert_eq!(
            delta.files(),
            &["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
        // Only symbols of changed files, ordered by file then qualified name.
        assert_eq!(delta.symbols().len(), 3);
        assert!(
            delta
                .symbols()
                .iter()
                .all(|symbol| symbol.starts_with("src/a.rs:") || symbol.starts_with("src/b.rs:"))
        );
        assert!(delta.symbols()[0].contains("a_one"));
        Ok(())
    }

    #[test]
    fn change_delta_rejects_persisted_symbol_without_file() {
        let dto = RepositoryChangeDeltaDto {
            files: vec!["src/a.rs".to_string()],
            symbols: vec!["src/b.rs:function:x:1-2".to_string()],
        };
        assert_eq!(
            RepositoryChangeDelta::try_from(dto),
            Err(RepositoryChangeDeltaError::SymbolWithoutFile)
        );
        let empty_file = RepositoryChangeDeltaDto {
            files: vec![String::new()],
            symbols: Vec::new(),
        };
        assert_eq!(
            RepositoryChangeDelta::try_from(empty_file),
            Err(RepositoryChangeDeltaError::EmptyFile)
        );
    }

    fn symbol_with_file(file: &str, name: &str) -> Result<SymbolRecord, SourceRangeError> {
        Ok(SymbolRecord {
            record_id: format!("{file}:function:{name}:1-2"),
            package: "p".to_string(),
            target: "t".to_string(),
            kind: SymbolKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            visibility: Visibility::Public,
            is_public_api: true,
            is_async: false,
            is_unsafe: false,
            is_test: false,
            is_bench: false,
            signature: None,
            imports: Vec::new(),
            doc_comment: None,
            markers: SymbolMarkers::default(),
            provenance: RecordProvenance {
                repository_root: "/work".to_string(),
                commit_sha: CommitSha::new("c"),
                worktree_identity: WorktreeIdentity::new("w"),
                content_hash: format!("sha256:{}", "0".repeat(64)),
                file_path: file.to_string(),
                source_range: SourceRange::new(1, 2)?,
                parser_generation: ParserGeneration::new("g"),
            },
        })
    }
}
