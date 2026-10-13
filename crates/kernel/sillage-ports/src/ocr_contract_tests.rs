use std::collections::BTreeSet;

use crate::contract_tests::{contract_assert, contract_assert_eq};
use crate::{OcrProvider, OcrRequest};

/// Shared behavioral contract every concrete `OcrProvider` must satisfy.
///
/// Concrete adapters execute this suite in their own test modules alongside
/// adapter-specific boundary tests (Rule 25). The contract covers
/// identity/disclosure stability, request-scoped page responses, and
/// duplicate-page rejection.
pub fn assert_ocr_provider_contract(
    provider: &impl OcrProvider,
    request: OcrRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    let identity = provider.identity();
    let disclosure = provider.disclosure();
    let response = provider.recognize(request.clone())?;

    contract_assert_eq!(
        response.identity,
        identity,
        "OCR provider returned a different identity than it advertises"
    );
    contract_assert_eq!(
        response.disclosure,
        disclosure,
        "OCR provider returned a different disclosure than it advertises"
    );
    contract_assert!(
        !response.pages.is_empty(),
        "OCR provider returned no pages for a requested page set"
    );
    let mut seen = BTreeSet::new();
    for page in &response.pages {
        contract_assert!(
            request.pages.contains(&page.page),
            "OCR provider returned page {} outside the requested set {:?}",
            page.page,
            request.pages
        );
        contract_assert!(
            seen.insert(page.page),
            "OCR provider returned page {} more than once",
            page.page
        );
    }

    // Identity and disclosure stay stable across calls.
    let second = provider.recognize(request)?;
    contract_assert_eq!(second.identity, identity);
    contract_assert_eq!(second.disclosure, disclosure);
    Ok(())
}
