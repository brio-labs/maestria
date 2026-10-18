use super::*;

fn fixture_ocr_intent(
    disclosure: sillage_domain::OcrDisclosure,
) -> Result<sillage_domain::OcrIntent, Box<dyn std::error::Error>> {
    let identity = sillage_domain::OcrProviderIdentity::new(
        "fixture",
        "ocr",
        "v1",
        "sha256:provider",
        "prep-v1",
    )?;
    let source_hash = sillage_domain::ContentHash::new(sillage_domain::content_hash(b"pdf"))?;
    Ok(sillage_domain::OcrIntent::new(
        sillage_domain::ArtifactId::new(1),
        sillage_domain::BlobId::new(1),
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
        &sillage_domain::SillageEffect::Ocr(fixture_ocr_intent(
            sillage_domain::OcrDisclosure::new(
                false,
                sillage_domain::OcrRetentionPolicy::NoRetention,
            ),
        )?),
        &scope,
    );
    assert_eq!(local_no_retention, RiskClass::Low);

    let local_provider_defined = classifier.classify(
        &sillage_domain::SillageEffect::Ocr(fixture_ocr_intent(
            sillage_domain::OcrDisclosure::new(
                false,
                sillage_domain::OcrRetentionPolicy::ProviderDefined,
            ),
        )?),
        &scope,
    );
    assert_eq!(local_provider_defined, RiskClass::Medium);

    let remote_no_retention = classifier.classify(
        &sillage_domain::SillageEffect::Ocr(fixture_ocr_intent(
            sillage_domain::OcrDisclosure::new(
                true,
                sillage_domain::OcrRetentionPolicy::NoRetention,
            ),
        )?),
        &scope,
    );
    assert_eq!(remote_no_retention, RiskClass::High);
    Ok(())
}
