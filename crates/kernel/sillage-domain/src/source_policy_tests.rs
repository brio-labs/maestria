use super::*;

type BudgetLimits = (u32, u64, u64, u64, u32, u64, u32);

fn base_parameters() -> Result<SourceChoiceParameters, SourceChoiceError> {
    Ok(SourceChoiceParameters {
        scope: SourceScope::file(PathBuf::from("/policy/file"))?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::ExtractedText],
        remote_disclosures: Vec::new(),
        budget: try_budget((1, 1, 1, 1, 1, 1, 1))?,
    })
}

fn try_budget(limits: BudgetLimits) -> Result<SourceResourceBudget, SourceChoiceError> {
    let (
        max_entries,
        max_file_bytes,
        max_total_source_bytes,
        max_in_memory_bytes,
        max_results,
        max_work_units,
        max_duration_ms,
    ) = limits;
    SourceResourceBudget::try_new(
        max_entries,
        max_file_bytes,
        max_total_source_bytes,
        max_in_memory_bytes,
        max_results,
        max_work_units,
        max_duration_ms,
    )
}

#[test]
fn source_scope_requires_absolute_paths_without_parent_components() -> Result<(), SourceChoiceError>
{
    assert_eq!(
        SourceScope::file(PathBuf::from("relative/file")),
        Err(SourceChoiceError::InvalidScope)
    );
    assert_eq!(
        SourceScope::file(PathBuf::from("/policy/../outside")),
        Err(SourceChoiceError::InvalidScope)
    );
    let file = SourceScope::file(PathBuf::from("/policy/file"))?;
    let folder = SourceScope::folder(PathBuf::from("/policy/folder"), true)?;
    assert_eq!(file.selection(), SourceSelection::File);
    assert_eq!(
        folder.selection(),
        SourceSelection::Folder { recursive: true }
    );
    Ok(())
}

#[test]
fn source_choice_keeps_retention_and_control_persistence_independent()
-> Result<(), SourceChoiceError> {
    for documentary_retention in [
        DocumentaryRetentionChoice::NoDocumentaryPersistence,
        DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
    ] {
        for control_persistence in [
            ControlPersistenceChoice::SessionOnly,
            ControlPersistenceChoice::PersistExplicitly,
        ] {
            let mut parameters = base_parameters()?;
            parameters.documentary_retention = documentary_retention;
            parameters.control_persistence = control_persistence;
            parameters.timing = ComputationTiming::BackgroundWhenIdle;
            parameters.availability = SourceAvailability::Paused;
            let mut choice = SourceChoice::try_new(parameters)?;
            assert_eq!(choice.documentary_retention(), documentary_retention);
            assert_eq!(choice.control_persistence(), control_persistence);
            assert_eq!(choice.timing(), ComputationTiming::BackgroundWhenIdle);
            assert_eq!(choice.availability(), SourceAvailability::Paused);
            choice.set_availability(SourceAvailability::Disabled);
            assert_eq!(choice.availability(), SourceAvailability::Disabled);
        }
    }
    Ok(())
}

#[test]
fn named_provider_locality_does_not_imply_disclosure_consent() -> Result<(), SourceChoiceError> {
    let mut parameters = base_parameters()?;
    parameters.locality = SourceLocality::LocalWithNamedRemoteProvider {
        provider_id: "provider-a".to_owned(),
    };
    parameters.representations = vec![SourceRepresentation::Embedding];
    let choice = SourceChoice::try_new(parameters)?;
    assert!(choice.remote_disclosures().is_empty());
    Ok(())
}

#[test]
fn local_only_choice_rejects_remote_disclosure() -> Result<(), SourceChoiceError> {
    let mut parameters = base_parameters()?;
    parameters.representations = vec![SourceRepresentation::Embedding];
    parameters.remote_disclosures = vec![RemoteDisclosureConsent {
        provider_id: "provider-a".to_owned(),
        representation: SourceRepresentation::Embedding,
        retention: RemoteRetentionChoice::NoRetention,
    }];
    assert_eq!(
        SourceChoice::try_new(parameters),
        Err(SourceChoiceError::DisclosureWithoutProvider)
    );
    Ok(())
}

#[test]
fn remote_disclosures_must_match_provider_and_supported_representation()
-> Result<(), SourceChoiceError> {
    let mut parameters = base_parameters()?;
    parameters.locality = SourceLocality::LocalWithNamedRemoteProvider {
        provider_id: "provider-a".to_owned(),
    };
    parameters.representations = vec![
        SourceRepresentation::ExtractedText,
        SourceRepresentation::OcrText,
        SourceRepresentation::Embedding,
    ];
    parameters.remote_disclosures = vec![
        RemoteDisclosureConsent {
            provider_id: "provider-a".to_owned(),
            representation: SourceRepresentation::OcrText,
            retention: RemoteRetentionChoice::NoRetention,
        },
        RemoteDisclosureConsent {
            provider_id: "provider-a".to_owned(),
            representation: SourceRepresentation::Embedding,
            retention: RemoteRetentionChoice::ProviderRetentionAccepted,
        },
    ];
    let choice = SourceChoice::try_new(parameters)?;
    assert_eq!(choice.remote_disclosures().len(), 2);

    let mut mismatched_provider = base_parameters()?;
    mismatched_provider.locality = SourceLocality::LocalWithNamedRemoteProvider {
        provider_id: "provider-a".to_owned(),
    };
    mismatched_provider.representations = vec![SourceRepresentation::Embedding];
    mismatched_provider.remote_disclosures = vec![RemoteDisclosureConsent {
        provider_id: "provider-b".to_owned(),
        representation: SourceRepresentation::Embedding,
        retention: RemoteRetentionChoice::NoRetention,
    }];
    assert_eq!(
        SourceChoice::try_new(mismatched_provider),
        Err(SourceChoiceError::InvalidDisclosure)
    );

    let mut unsupported = base_parameters()?;
    unsupported.locality = SourceLocality::LocalWithNamedRemoteProvider {
        provider_id: "provider-a".to_owned(),
    };
    unsupported.representations = vec![SourceRepresentation::ExtractedText];
    unsupported.remote_disclosures = vec![RemoteDisclosureConsent {
        provider_id: "provider-a".to_owned(),
        representation: SourceRepresentation::ExtractedText,
        retention: RemoteRetentionChoice::NoRetention,
    }];
    assert_eq!(
        SourceChoice::try_new(unsupported),
        Err(SourceChoiceError::InvalidDisclosure)
    );
    Ok(())
}

#[test]
fn source_choice_rejects_empty_duplicate_and_unmatched_representations()
-> Result<(), SourceChoiceError> {
    let mut empty = base_parameters()?;
    empty.representations.clear();
    assert_eq!(
        SourceChoice::try_new(empty),
        Err(SourceChoiceError::InvalidRepresentations)
    );

    let mut duplicate = base_parameters()?;
    duplicate.representations = vec![
        SourceRepresentation::ExtractedText,
        SourceRepresentation::ExtractedText,
    ];
    assert_eq!(
        SourceChoice::try_new(duplicate),
        Err(SourceChoiceError::InvalidRepresentations)
    );

    let mut duplicate_disclosures = base_parameters()?;
    duplicate_disclosures.locality = SourceLocality::LocalWithNamedRemoteProvider {
        provider_id: "provider-a".to_owned(),
    };
    duplicate_disclosures.representations = vec![SourceRepresentation::Embedding];
    duplicate_disclosures.remote_disclosures = vec![
        RemoteDisclosureConsent {
            provider_id: "provider-a".to_owned(),
            representation: SourceRepresentation::Embedding,
            retention: RemoteRetentionChoice::NoRetention,
        },
        RemoteDisclosureConsent {
            provider_id: "provider-a".to_owned(),
            representation: SourceRepresentation::Embedding,
            retention: RemoteRetentionChoice::ProviderRetentionAccepted,
        },
    ];
    assert_eq!(
        SourceChoice::try_new(duplicate_disclosures),
        Err(SourceChoiceError::InvalidDisclosure)
    );
    Ok(())
}

#[test]
fn source_resource_budget_accepts_supported_boundaries() -> Result<(), SourceChoiceError> {
    assert!(try_budget((1, 1, 1, 1, 1, 1, 1)).is_ok());
    assert!(
        try_budget((
            16_384,
            8 * 1024 * 1024,
            128 * 1024 * 1024,
            256 * 1024 * 1024,
            100,
            1_000_000_000,
            60_000,
        ))
        .is_ok()
    );
    Ok(())
}

#[test]
fn source_resource_budget_rejects_out_of_range_limits() {
    let invalid_limits = [
        (0, 1, 1, 1, 1, 1, 1),
        (16_385, 1, 1, 1, 1, 1, 1),
        (1, 0, 1, 1, 1, 1, 1),
        (
            1,
            8 * 1024 * 1024 + 1,
            128 * 1024 * 1024,
            256 * 1024 * 1024,
            1,
            1,
            1,
        ),
        (1, 2, 1, 2, 1, 1, 1),
        (1, 1, 128 * 1024 * 1024 + 1, 128 * 1024 * 1024 + 1, 1, 1, 1),
        (1, 2, 2, 1, 1, 1, 1),
        (1, 1, 1, 256 * 1024 * 1024 + 1, 1, 1, 1),
        (1, 1, 1, 1, 0, 1, 1),
        (1, 1, 1, 1, 101, 1, 1),
        (1, 1, 1, 1, 1, 0, 1),
        (1, 1, 1, 1, 1, 1_000_000_001, 1),
        (1, 1, 1, 1, 1, 1, 0),
        (1, 1, 1, 1, 1, 1, 60_001),
    ];
    for limits in invalid_limits {
        assert!(matches!(
            try_budget(limits),
            Err(SourceChoiceError::InvalidBudget)
        ));
    }
}

#[test]
fn serde_round_trip_preserves_a_validated_policy() -> Result<(), Box<dyn std::error::Error>> {
    let mut parameters = base_parameters()?;
    parameters.locality = SourceLocality::LocalWithNamedRemoteProvider {
        provider_id: "provider-a".to_owned(),
    };
    parameters.representations = vec![SourceRepresentation::OcrText];
    parameters.remote_disclosures = vec![RemoteDisclosureConsent {
        provider_id: "provider-a".to_owned(),
        representation: SourceRepresentation::OcrText,
        retention: RemoteRetentionChoice::ProviderRetentionAccepted,
    }];
    let choice = SourceChoice::try_new(parameters)?;
    let wire = serde_json::to_string(&choice)?;
    let decoded = serde_json::from_str::<SourceChoice>(&wire)?;
    assert_eq!(decoded, choice);
    Ok(())
}

#[test]
fn serde_revalidates_scope_budget_and_source_choice_invariants() {
    let invalid_scope = r#"{"path":"relative/file","selection":"File"}"#;
    assert!(serde_json::from_str::<SourceScope>(invalid_scope).is_err());

    let invalid_budget = r#"{
        "max_entries": 0,
        "max_file_bytes": 1,
        "max_total_source_bytes": 1,
        "max_in_memory_bytes": 1,
        "max_results": 1,
        "max_work_units": 1,
        "max_duration_ms": 1
    }"#;
    assert!(serde_json::from_str::<SourceResourceBudget>(invalid_budget).is_err());

    let invalid_choice = r#"{
        "scope": {"path": "/policy/file", "selection": "File"},
        "timing": "OnDemand",
        "locality": "LocalOnly",
        "documentary_retention": "NoDocumentaryPersistence",
        "control_persistence": "SessionOnly",
        "availability": "Enabled",
        "representations": [],
        "remote_disclosures": [],
        "budget": {
            "max_entries": 1,
            "max_file_bytes": 1,
            "max_total_source_bytes": 1,
            "max_in_memory_bytes": 1,
            "max_results": 1,
            "max_work_units": 1,
            "max_duration_ms": 1
        }
    }"#;
    assert!(serde_json::from_str::<SourceChoice>(invalid_choice).is_err());
}
