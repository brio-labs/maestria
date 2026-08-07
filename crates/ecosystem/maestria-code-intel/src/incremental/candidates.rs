//! Relation candidate sidecar persistence (atomic tmp + rename).

use crate::CodeIntelError;
use crate::symbols::RelationCandidate;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Sidecar payload: the full relation candidate set for one parser generation.
/// Candidates live outside the index so the daemon/query path never parses them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RepositoryRelationCandidates {
    pub parser_generation: String,
    pub candidates: Vec<RelationCandidate>,
}

/// Write the relation candidates sidecar atomically (tmp + rename).
pub(crate) fn write_relation_candidates(
    path: &Path,
    generation: &str,
    candidates: &[RelationCandidate],
) -> Result<(), CodeIntelError> {
    let payload = RepositoryRelationCandidates {
        parser_generation: generation.to_string(),
        candidates: candidates.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&payload).map_err(|error| CodeIntelError::Persist {
        context: "serialize relation candidates".to_string(),
        details: error.to_string(),
    })?;
    let temporary = path.with_extension("candidates.tmp");
    fs::write(&temporary, bytes).map_err(|error| CodeIntelError::Persist {
        context: "write temporary relation candidates file".to_string(),
        details: format!("{temporary:?}: {error}"),
    })?;
    fs::rename(&temporary, path).map_err(|error| CodeIntelError::Persist {
        context: "atomically replace relation candidates file".to_string(),
        details: format!("{path:?}: {error}"),
    })
}

/// Load the relation candidates sidecar; `None` when missing or built under a
/// different parser generation, error on corrupt JSON.
pub(crate) fn load_relation_candidates(
    path: &Path,
    expected_generation: &str,
) -> Result<Option<Vec<RelationCandidate>>, CodeIntelError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CodeIntelError::Persist {
                context: "read relation candidates file".to_string(),
                details: format!("{path:?}: {error}"),
            });
        }
    };
    let payload: RepositoryRelationCandidates =
        serde_json::from_slice(&bytes).map_err(|error| CodeIntelError::Persist {
            context: "deserialize relation candidates".to_string(),
            details: error.to_string(),
        })?;
    if payload.parser_generation != expected_generation {
        return Ok(None);
    }
    Ok(Some(payload.candidates))
}
