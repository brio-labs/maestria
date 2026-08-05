use maestria_domain::{
    DomainError, DomainInput, EvidenceId, FederatedAccessRecord, FederatedEvidenceBounds,
    FederatedReadAccess, GrantTokenDigest, IssueRealmReadGrantInput, KernelState, QueryId, RealmId,
    RealmReadGrant, RecordFederatedAccessInput, RevokeRealmReadGrantInput, SearchTraceId,
    Sensitivity, replay_events,
};

fn realm(byte: char) -> Result<RealmId, Box<dyn std::error::Error>> {
    Ok(RealmId::try_from(byte.to_string().repeat(64))?)
}

fn grant(
    consumer: RealmId,
    credential: &[u8],
) -> Result<RealmReadGrant, Box<dyn std::error::Error>> {
    Ok(RealmReadGrant::new(
        GrantTokenDigest::derive(credential),
        realm('a')?,
        consumer,
        FederatedReadAccess::SearchAndOpenEvidence,
        Sensitivity::Confidential,
        FederatedEvidenceBounds::try_new(2, 128)?,
    ))
}

#[test]
fn issue_revoke_and_replay_reconstructs_one_provider_grant()
-> Result<(), Box<dyn std::error::Error>> {
    let grant = grant(realm('b')?, b"capability")?;
    let digest = grant.token_digest().clone();
    let mut state = KernelState::new();
    let issued = state.apply_input(DomainInput::IssueRealmReadGrant(IssueRealmReadGrantInput {
        grant,
    }))?;
    let revoked = state.apply_input(DomainInput::RevokeRealmReadGrant(
        RevokeRealmReadGrantInput {
            token_digest: digest,
        },
    ))?;

    let mut events = issued.events;
    events.extend(revoked.events);
    let replayed = replay_events(&events)?;
    assert_eq!(state.realm_read_grants, replayed.realm_read_grants);
    assert_eq!(replayed.realm_read_grants.len(), 1);
    Ok(())
}

#[test]
fn lifecycle_failures_are_typed() -> Result<(), Box<dyn std::error::Error>> {
    let consumer = realm('b')?;
    let issued_grant = grant(consumer.clone(), b"capability-a")?;
    let digest = issued_grant.token_digest().clone();
    let mut state = KernelState::new();
    state.apply_input(DomainInput::IssueRealmReadGrant(IssueRealmReadGrantInput {
        grant: issued_grant,
    }))?;

    let duplicate = state.apply_input(DomainInput::IssueRealmReadGrant(IssueRealmReadGrantInput {
        grant: grant(consumer, b"capability-b")?,
    }));
    assert!(matches!(
        duplicate,
        Err(DomainError::DuplicateActiveRealmReadGrant { .. })
    ));
    assert!(matches!(
        state.apply_input(DomainInput::RevokeRealmReadGrant(
            RevokeRealmReadGrantInput {
                token_digest: GrantTokenDigest::derive(b"unknown"),
            }
        )),
        Err(DomainError::MissingRealmReadGrant { .. })
    ));

    state.apply_input(DomainInput::RevokeRealmReadGrant(
        RevokeRealmReadGrantInput {
            token_digest: digest.clone(),
        },
    ))?;
    assert!(matches!(
        state.apply_input(DomainInput::RevokeRealmReadGrant(
            RevokeRealmReadGrantInput {
                token_digest: digest.clone(),
            }
        )),
        Err(DomainError::RealmReadGrantAlreadyRevoked { .. })
    ));
    assert!(matches!(
        state.apply_input(DomainInput::RecordFederatedAccess(
            RecordFederatedAccessInput {
                token_digest: digest,
                provider_realm: realm('a')?,
                consumer_realm: realm('b')?,
                record: FederatedAccessRecord::Search {
                    query_id: QueryId::new(1),
                    trace_id: SearchTraceId::new(1),
                },
            }
        )),
        Err(DomainError::RealmReadGrantRevoked { .. })
    ));
    Ok(())
}

#[test]
fn access_record_rejects_wrong_consumer_realm() -> Result<(), Box<dyn std::error::Error>> {
    let grant = grant(realm('b')?, b"capability")?;
    let digest = grant.token_digest().clone();
    let mut state = KernelState::new();
    state.apply_input(DomainInput::IssueRealmReadGrant(IssueRealmReadGrantInput {
        grant,
    }))?;

    assert!(matches!(
        state.apply_input(DomainInput::RecordFederatedAccess(
            RecordFederatedAccessInput {
                token_digest: digest,
                provider_realm: realm('a')?,
                consumer_realm: realm('c')?,
                record: FederatedAccessRecord::Search {
                    query_id: QueryId::new(1),
                    trace_id: SearchTraceId::new(1),
                },
            }
        )),
        Err(DomainError::RealmReadGrantConsumerMismatch { .. })
    ));
    Ok(())
}

#[test]
fn search_only_grant_cannot_record_evidence_access() -> Result<(), Box<dyn std::error::Error>> {
    let consumer = realm('b')?;
    let grant = RealmReadGrant::new(
        GrantTokenDigest::derive(b"search-only"),
        realm('a')?,
        consumer.clone(),
        FederatedReadAccess::SearchOnly,
        Sensitivity::Public,
        FederatedEvidenceBounds::try_new(1, 64)?,
    );
    let digest = grant.token_digest().clone();
    let mut state = KernelState::new();
    state.apply_input(DomainInput::IssueRealmReadGrant(IssueRealmReadGrantInput {
        grant,
    }))?;

    let result = state.apply_input(DomainInput::RecordFederatedAccess(
        RecordFederatedAccessInput {
            token_digest: digest,
            provider_realm: realm('a')?,
            consumer_realm: consumer,
            record: FederatedAccessRecord::Evidence {
                evidence_id: EvidenceId::new(7),
            },
        },
    ));
    assert!(matches!(
        result,
        Err(DomainError::RealmReadGrantUnsupportedAccess { .. })
    ));
    Ok(())
}
