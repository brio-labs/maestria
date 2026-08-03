use serde::{Deserialize, Serialize};

use crate::search::SearchCompatibilityError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EvidenceCoverageDto")]
pub struct EvidenceCoverage {
    percent_covered: u8,
    gaps_identified: Vec<String>,
    required_claims: Vec<String>,
    required_subquestions: Vec<String>,
    distinct_sources: usize,
    distinct_documents: usize,
    distinct_sections: usize,
    candidate_coverage_keys: Vec<String>,
}

impl EvidenceCoverage {
    /// Validate and construct evidence coverage from its boundary input;
    /// `percent_covered` must be in `0..=100` (R56: the field set is
    /// private so the invariant cannot be bypassed by a struct literal).
    pub fn new(dto: EvidenceCoverageDto) -> Result<Self, SearchCompatibilityError> {
        if dto.percent_covered > 100 {
            return Err(SearchCompatibilityError::InvalidCoverage(
                "percent_covered must be between 0 and 100",
            ));
        }
        Ok(Self {
            percent_covered: dto.percent_covered,
            gaps_identified: dto.gaps_identified,
            required_claims: dto.required_claims,
            required_subquestions: dto.required_subquestions,
            distinct_sources: dto.distinct_sources,
            distinct_documents: dto.distinct_documents,
            distinct_sections: dto.distinct_sections,
            candidate_coverage_keys: dto.candidate_coverage_keys,
        })
    }

    pub fn percent_covered(&self) -> u8 {
        self.percent_covered
    }

    pub fn gaps_identified(&self) -> &[String] {
        &self.gaps_identified
    }

    pub fn required_claims(&self) -> &[String] {
        &self.required_claims
    }

    pub fn required_subquestions(&self) -> &[String] {
        &self.required_subquestions
    }

    pub fn distinct_sources(&self) -> usize {
        self.distinct_sources
    }

    pub fn distinct_documents(&self) -> usize {
        self.distinct_documents
    }

    pub fn distinct_sections(&self) -> usize {
        self.distinct_sections
    }

    pub fn candidate_coverage_keys(&self) -> &[String] {
        &self.candidate_coverage_keys
    }
}

/// Boundary input for [`EvidenceCoverage`] (R37).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCoverageDto {
    pub percent_covered: u8,
    pub gaps_identified: Vec<String>,
    pub required_claims: Vec<String>,
    pub required_subquestions: Vec<String>,
    pub distinct_sources: usize,
    pub distinct_documents: usize,
    pub distinct_sections: usize,
    pub candidate_coverage_keys: Vec<String>,
}

impl TryFrom<EvidenceCoverageDto> for EvidenceCoverage {
    type Error = SearchCompatibilityError;
    fn try_from(dto: EvidenceCoverageDto) -> Result<Self, Self::Error> {
        Self::new(dto)
    }
}
