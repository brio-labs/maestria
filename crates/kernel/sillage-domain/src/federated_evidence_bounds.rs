use std::fmt;

pub const MIN_FEDERATED_RESULTS: usize = 1;
pub const MAX_FEDERATED_RESULTS: usize = 100;
pub const MIN_FEDERATED_EVIDENCE_BYTES: usize = 1;
pub const MAX_FEDERATED_EVIDENCE_BYTES: usize = 65_536;

/// Finite evidence limits carried by a realm-read grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FederatedEvidenceBounds {
    max_results: usize,
    max_evidence_bytes: usize,
}

impl FederatedEvidenceBounds {
    pub fn try_new(
        max_results: usize,
        max_evidence_bytes: usize,
    ) -> Result<Self, FederatedEvidenceBoundsError> {
        if !(MIN_FEDERATED_RESULTS..=MAX_FEDERATED_RESULTS).contains(&max_results) {
            return Err(FederatedEvidenceBoundsError::InvalidMaxResults { max_results });
        }
        if !(MIN_FEDERATED_EVIDENCE_BYTES..=MAX_FEDERATED_EVIDENCE_BYTES)
            .contains(&max_evidence_bytes)
        {
            return Err(FederatedEvidenceBoundsError::InvalidMaxEvidenceBytes {
                max_evidence_bytes,
            });
        }
        Ok(Self {
            max_results,
            max_evidence_bytes,
        })
    }

    pub const fn max_results(&self) -> usize {
        self.max_results
    }

    pub const fn max_evidence_bytes(&self) -> usize {
        self.max_evidence_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederatedEvidenceBoundsError {
    InvalidMaxResults { max_results: usize },
    InvalidMaxEvidenceBytes { max_evidence_bytes: usize },
}

impl fmt::Display for FederatedEvidenceBoundsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMaxResults { max_results } => write!(
                formatter,
                "federated maximum results must be {MIN_FEDERATED_RESULTS}..={MAX_FEDERATED_RESULTS}, got {max_results}"
            ),
            Self::InvalidMaxEvidenceBytes { max_evidence_bytes } => write!(
                formatter,
                "federated maximum evidence bytes must be {MIN_FEDERATED_EVIDENCE_BYTES}..={MAX_FEDERATED_EVIDENCE_BYTES}, got {max_evidence_bytes}"
            ),
        }
    }
}

impl std::error::Error for FederatedEvidenceBoundsError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range_bounds() {
        assert!(FederatedEvidenceBounds::try_new(0, 1).is_err());
        assert!(FederatedEvidenceBounds::try_new(101, 1).is_err());
        assert!(FederatedEvidenceBounds::try_new(1, 0).is_err());
        assert!(FederatedEvidenceBounds::try_new(1, 65_537).is_err());
    }
}
