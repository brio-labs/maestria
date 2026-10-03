//! Validation checks for search plan and outcome reproducibility.

use super::types::{ValidationCheck, ValidationContext, Validator};

#[path = "search_validator_checks.rs"]
mod checks;
#[path = "search_validator_rules.rs"]
mod rules;

#[derive(Debug, Clone, Copy)]
pub struct SearchCheck {
    pub name: &'static str,
    pub check: fn(&ValidationContext<'_>) -> Result<String, String>,
}

impl Validator for SearchCheck {
    fn name(&self) -> &str {
        self.name
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        match (self.check)(context) {
            Ok(message) => super::types::passed_check(self.name, message),
            Err(message) => super::types::failed_check(self.name, message),
        }
    }
}

pub const SEARCH_CHECKS: [SearchCheck; 8] = [
    SearchCheck {
        name: "search_plan",
        check: checks::check_search_plan,
    },
    SearchCheck {
        name: "coverage",
        check: checks::check_coverage,
    },
    SearchCheck {
        name: "conflict",
        check: checks::check_conflict,
    },
    SearchCheck {
        name: "freshness",
        check: checks::check_freshness,
    },
    SearchCheck {
        name: "citation_alignment",
        check: checks::check_citation_alignment,
    },
    SearchCheck {
        name: "retrieval_security",
        check: checks::check_retrieval_security,
    },
    SearchCheck {
        name: "search_regression",
        check: checks::check_search_regression,
    },
    SearchCheck {
        name: "candidate_provenance",
        check: checks::check_candidate_provenance,
    },
];
