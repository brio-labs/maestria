use super::*;

fn fixture_ocr_intent(
    disclosure: maestria_domain::OcrDisclosure,
) -> Result<maestria_domain::OcrIntent, Box<dyn std::error::Error>> {
    let identity = maestria_domain::OcrProviderIdentity::new(
        "fixture",
        "ocr",
        "v1",
        "sha256:provider",
        "prep-v1",
    )?;
    let source_hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(b"pdf"))?;
    Ok(maestria_domain::OcrIntent::new(
        maestria_domain::ArtifactId::new(1),
        maestria_domain::BlobId::new(1),
        source_hash,
        [1],
        identity,
        disclosure,
    )?)
}

#[test]
fn ocr_risk_requires_low_for_local_no_retention_and_governs_other_disclosures()
-> Result<(), Box<dyn std::error::Error>> {
    let scope = Scope::new(
        vec![std::path::PathBuf::from("/data")],
        vec![std::path::PathBuf::from("/data")],
        vec![],
        vec![],
        false,
    );
    let classifier = DefaultRiskClassifier;

    let local_no_retention = classifier.classify(
        &maestria_domain::MaestriaEffect::Ocr(fixture_ocr_intent(
            maestria_domain::OcrDisclosure::new(
                false,
                maestria_domain::OcrRetentionPolicy::NoRetention,
            ),
        )?),
        &scope,
    );
    assert_eq!(local_no_retention, RiskClass::Low);

    let local_provider_defined = classifier.classify(
        &maestria_domain::MaestriaEffect::Ocr(fixture_ocr_intent(
            maestria_domain::OcrDisclosure::new(
                false,
                maestria_domain::OcrRetentionPolicy::ProviderDefined,
            ),
        )?),
        &scope,
    );
    assert_eq!(local_provider_defined, RiskClass::Medium);

    let remote_no_retention = classifier.classify(
        &maestria_domain::MaestriaEffect::Ocr(fixture_ocr_intent(
            maestria_domain::OcrDisclosure::new(
                true,
                maestria_domain::OcrRetentionPolicy::NoRetention,
            ),
        )?),
        &scope,
    );
    assert_eq!(remote_no_retention, RiskClass::High);
    Ok(())
}
