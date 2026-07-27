use serde::{Deserialize, Serialize};

use crate::search::SearchCompatibilityError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EvidenceCoverageDto")]
pub struct EvidenceCoverage {
    pub percent_covered: u8,
    pub gaps_identified: Vec<String>,
    #[serde(default)]
    pub required_claims: Vec<String>,
    #[serde(default)]
    pub required_subquestions: Vec<String>,
    #[serde(default)]
    pub distinct_sources: usize,
    #[serde(default)]
    pub distinct_documents: usize,
    #[serde(default)]
    pub distinct_sections: usize,
    #[serde(default)]
    pub candidate_coverage_keys: Vec<String>,
}

#[derive(Deserialize)]
struct EvidenceCoverageDto {
    percent_covered: u8,
    gaps_identified: Vec<String>,
    #[serde(default)]
    required_claims: Vec<String>,
    #[serde(default)]
    required_subquestions: Vec<String>,
    #[serde(default)]
    distinct_sources: usize,
    #[serde(default)]
    distinct_documents: usize,
    #[serde(default)]
    distinct_sections: usize,
    #[serde(default)]
    candidate_coverage_keys: Vec<String>,
}

impl TryFrom<EvidenceCoverageDto> for EvidenceCoverage {
    type Error = SearchCompatibilityError;
    fn try_from(dto: EvidenceCoverageDto) -> Result<Self, Self::Error> {
        if dto.percent_covered > 100 {
            return Err(SearchCompatibilityError::InvalidCoverage(
                "percent_covered must be between 0 and 100",
            ));
        }
        Ok(EvidenceCoverage {
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
}
