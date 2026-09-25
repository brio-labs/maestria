use std::collections::BTreeSet;

use maestria_domain::*;
use maestria_ports::contract_tests::assert_realm_read_grant_repository_contract;
use maestria_ports::{EventFilter, EventLog, RealmReadGrantRepository};
use rusqlite::params;

use crate::SqliteStore;
use crate::payloads::event_payloads::StoredEventPayload;
use crate::payloads::realm_read_grant_event_payloads::{
    StoredFederatedReadAccess, StoredFederatedSensitivity,
};

fn realm(byte: char) -> Result<RealmId, Box<dyn std::error::Error>> {
    Ok(RealmId::try_from(byte.to_string().repeat(64))?)
}

fn grant(state: RealmReadGrantState) -> Result<RealmReadGrant, Box<dyn std::error::Error>> {
    Ok(RealmReadGrant::from_current_state(
        RealmReadGrant::new(
            GrantTokenDigest::derive(b"credential"),
            realm('a')?,
            realm('b')?,
            FederatedReadAccess::SearchAndOpenEvidence,
            Sensitivity::Confidential,
            FederatedEvidenceBounds::try_new(2, 128)?,
            RealmReadGrantExpiry::new(1_000_000_000)?,
        ),
        state,
    ))
}

fn scoped_grant(state: RealmReadGrantState) -> Result<RealmReadGrant, Box<dyn std::error::Error>> {
    Ok(grant(state)?.with_allowed_roots(vec![std::path::PathBuf::from("/provider/approved")]))
}

#[test]
fn realm_read_grant_projection_round_trips_and_cleans_stale_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let active = scoped_grant(RealmReadGrantState::Active)?;
    let revoked = RealmReadGrant::from_current_state(
        RealmReadGrant::new(
            GrantTokenDigest::derive(b"revoked"),
            realm('a')?,
            realm('c')?,
            FederatedReadAccess::SearchOnly,
            Sensitivity::Internal,
            FederatedEvidenceBounds::try_new(1, 1)?,
            RealmReadGrantExpiry::new(1_000_000_000)?,
        ),
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
fn realm_read_grant_scope_survives_reopen_and_event_replay()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("scoped-grant.db");
    let issued = scoped_grant(RealmReadGrantState::Active)?;
    {
        let store = SqliteStore::open(&path)?;
        store.append(DomainEventEnvelope {
            id: EventId::new(1),
            event: DomainEvent::RealmReadGrantIssued {
                grant: issued.clone(),
            },
        })?;
        let events = store.scan(EventFilter { artifact_id: None })?;
        let replayed = replay_events(events)?;
        let projected = replayed
            .realm_read_grants
            .get(issued.token_digest())
            .ok_or("scoped grant did not replay")?;
        assert_eq!(projected.allowed_roots(), issued.allowed_roots());
        store.put(projected.clone())?;
    }

    let reopened = SqliteStore::open(&path)?;
    assert_eq!(reopened.get(issued.token_digest())?, Some(issued.clone()));
    let events = reopened.scan(EventFilter { artifact_id: None })?;
    let replayed = replay_events(events)?;
    let persisted = replayed
        .realm_read_grants
        .get(issued.token_digest())
        .ok_or("scoped grant did not replay after reopen")?;
    assert_eq!(persisted, &issued);
    Ok(())
}

#[test]
fn realm_read_grant_projection_rejects_two_active_consumer_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    store.put(grant(RealmReadGrantState::Active)?)?;
    let second = RealmReadGrant::from_current_state(
        RealmReadGrant::new(
            GrantTokenDigest::derive(b"second-credential"),
            realm('a')?,
            realm('b')?,
            FederatedReadAccess::SearchOnly,
            Sensitivity::Public,
            FederatedEvidenceBounds::try_new(1, 1)?,
            RealmReadGrantExpiry::new(1_000_000_000)?,
        ),
        RealmReadGrantState::Active,
    );
    assert!(matches!(
        store.put(second),
        Err(maestria_ports::PortError::Conflict { .. })
    ));
    Ok(())
}

#[test]
fn realm_read_grant_projection_rejects_invalid_stored_roots()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let grant = grant(RealmReadGrantState::Active)?;
    store.put(grant.clone())?;
    for invalid_scope in [
        Vec::<String>::new(),
        vec!["/provider/approved".to_string(); 65],
        vec![format!("/{}", "x".repeat(8192))],
    ] {
        let connection = store.lock()?;
        connection.execute(
            "UPDATE realm_read_grants SET allowed_roots_json = ?1 WHERE token_digest = ?2",
            params![
                serde_json::to_string(&invalid_scope)?,
                grant.token_digest().as_str()
            ],
        )?;
        drop(connection);
        assert!(store.get(grant.token_digest()).is_err());
    }
    Ok(())
}

#[test]
fn realm_read_grant_event_decode_rejects_invalid_or_unbounded_roots()
-> Result<(), Box<dyn std::error::Error>> {
    let invalid_payloads = [
        Vec::<String>::new(),
        vec!["/provider/../approved".to_string()],
        vec!["/provider/approved".to_string(); 65],
        vec![format!("/{}", "x".repeat(8192))],
    ];
    for roots in invalid_payloads {
        let store = SqliteStore::in_memory()?;
        let payload = StoredEventPayload::RealmReadGrantIssued {
            token_digest: "a".repeat(64),
            provider_realm: "b".repeat(64),
            consumer_realm: "c".repeat(64),
            access: StoredFederatedReadAccess::SearchAndOpenEvidence,
            max_sensitivity: StoredFederatedSensitivity::Confidential,
            max_results: 2,
            max_evidence_bytes: 128,
            expires_at_unix_seconds: 1_000_000_000,
            allowed_roots: Some(roots),
        };
        let connection = store.lock()?;
        connection.execute(
            "INSERT INTO domain_events
                 (id, event_kind, artifact_id, payload_json, payload_version)
             VALUES (1, 'realm_read_grant_issued', NULL, ?1, 2)",
            params![serde_json::to_string(&payload)?],
        )?;
        drop(connection);
        assert!(store.scan(EventFilter { artifact_id: None }).is_err());
    }
    Ok(())
}

#[test]
fn realm_read_grant_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    assert_realm_read_grant_repository_contract(&store)?;
    Ok(())
}

#[test]
fn realm_read_grant_events_round_trip_through_strict_payloads()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let grant = scoped_grant(RealmReadGrantState::Active)?;
    let digest = grant.token_digest().clone();
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            event: DomainEvent::RealmReadGrantIssued {
                grant: grant.clone(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
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
