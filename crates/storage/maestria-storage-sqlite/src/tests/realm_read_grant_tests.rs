use std::collections::BTreeSet;

use maestria_domain::*;
use maestria_ports::{EventFilter, EventLog, RealmReadGrantRepository};

use crate::SqliteStore;

fn realm(byte: char) -> Result<RealmId, Box<dyn std::error::Error>> {
    Ok(RealmId::try_from(byte.to_string().repeat(64))?)
}

fn grant(state: RealmReadGrantState) -> Result<RealmReadGrant, Box<dyn std::error::Error>> {
    Ok(RealmReadGrant::from_current_state(
        GrantTokenDigest::derive(b"credential"),
        realm('a')?,
        realm('b')?,
        FederatedReadAccess::SearchAndOpenEvidence,
        Sensitivity::Confidential,
        FederatedEvidenceBounds::try_new(2, 128)?,
        state,
    ))
}

#[test]
fn realm_read_grant_projection_round_trips_and_cleans_stale_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let active = grant(RealmReadGrantState::Active)?;
    let revoked = RealmReadGrant::from_current_state(
        GrantTokenDigest::derive(b"revoked"),
        realm('a')?,
        realm('c')?,
        FederatedReadAccess::SearchOnly,
        Sensitivity::Internal,
        FederatedEvidenceBounds::try_new(1, 1)?,
        RealmReadGrantState::Revoked,
    );
    store.put(active.clone())?;
    store.put(revoked.clone())?;
    assert_eq!(store.get(active.token_digest())?, Some(active.clone()));
    let mut expected = vec![active.clone(), revoked.clone()];
    expected.sort_by(|left, right| left.token_digest().cmp(right.token_digest()));
    assert_eq!(store.list()?, expected);

    store.delete_not_in(&BTreeSet::from([active.token_digest().clone()]))?;
    assert_eq!(store.list()?, vec![active]);
    Ok(())
}

#[test]
fn realm_read_grant_projection_rejects_two_active_consumer_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    store.put(grant(RealmReadGrantState::Active)?)?;
    let second = RealmReadGrant::from_current_state(
        GrantTokenDigest::derive(b"second-credential"),
        realm('a')?,
        realm('b')?,
        FederatedReadAccess::SearchOnly,
        Sensitivity::Public,
        FederatedEvidenceBounds::try_new(1, 1)?,
        RealmReadGrantState::Active,
    );
    assert!(matches!(
        store.put(second),
        Err(maestria_ports::PortError::Conflict { .. })
    ));
    Ok(())
}

#[test]
fn realm_read_grant_events_round_trip_through_strict_payloads()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let grant = grant(RealmReadGrantState::Active)?;
    let digest = grant.token_digest().clone();
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::RealmReadGrantIssued {
                grant: grant.clone(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::FederatedReadAccessRecorded {
                token_digest: digest.clone(),
                provider_realm: realm('a')?,
                consumer_realm: realm('b')?,
                record: FederatedAccessRecord::Search {
                    query_id: QueryId::new(7),
                    trace_id: SearchTraceId::new(8),
                },
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::RealmReadGrantRevoked {
                token_digest: digest,
            },
        },
    ];
    for event in &events {
        store.append(event.clone())?;
    }
    assert_eq!(store.scan(EventFilter { artifact_id: None })?, events);
    Ok(())
}
