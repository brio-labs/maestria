//! Shared [`RealmReadGrantRepository`] contract (Rule 25: every concrete
//! realm-read-grant repository executes the shared persistence and
//! exclusivity suite).
//!
//! Scenario realms and digests are disjoint per step so the suite runs
//! against a single shared repository instance.

use std::collections::BTreeSet;

use maestria_domain::{
    FederatedEvidenceBounds, FederatedReadAccess, GrantTokenDigest, RealmId, RealmReadGrant,
    RealmReadGrantState, Sensitivity,
};

use super::*;

fn realm(byte: char) -> Result<RealmId, Box<dyn std::error::Error>> {
    Ok(RealmId::try_from(byte.to_string().repeat(64))?)
}

fn active_grant(
    digest: &[u8],
    provider: char,
    consumer: char,
    access: FederatedReadAccess,
) -> Result<RealmReadGrant, Box<dyn std::error::Error>> {
    Ok(RealmReadGrant::new(
        GrantTokenDigest::derive(digest),
        realm(provider)?,
        realm(consumer)?,
        access,
        Sensitivity::Public,
        FederatedEvidenceBounds::try_new(1, 1)?,
    ))
}

/// Put round trip, identical-put idempotence, same-token upsert replace,
/// active-grant exclusivity, revoked-non-blocking, and delete_not_in.
pub fn assert_realm_read_grant_repository_contract(
    repository: &impl RealmReadGrantRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    let grant = active_grant(b"contract-a", 'a', 'b', FederatedReadAccess::SearchOnly)?;

    // put round trips through get and list
    repository.put(grant.clone())?;
    assert_eq!(
        repository.get(grant.token_digest())?,
        Some(grant.clone()),
        "put must be immediately visible to get"
    );
    assert_eq!(repository.list()?, vec![grant.clone()]);

    // put of an identical value is idempotent
    repository.put(grant.clone())?;
    assert_eq!(repository.get(grant.token_digest())?, Some(grant.clone()));

    // put of the same token with new content replaces (upsert)
    let replaced = active_grant(
        b"contract-a",
        'a',
        'b',
        FederatedReadAccess::SearchAndOpenEvidence,
    )?;
    repository.put(replaced.clone())?;
    assert_eq!(
        repository.get(replaced.token_digest())?,
        Some(replaced.clone()),
        "same-token put must replace the stored grant"
    );

    // a second active grant for the same consumer realm conflicts
    let second = active_grant(b"contract-b", 'a', 'b', FederatedReadAccess::SearchOnly)?;
    let Err(error) = repository.put(second.clone()) else {
        return Err("expected exclusivity conflict".into());
    };
    assert!(
        matches!(error, PortError::Conflict { .. }),
        "second active grant for the same consumer realm must conflict, got {error:?}"
    );
    assert_eq!(repository.get(second.token_digest())?, None);

    // a revoked grant does not block a new active grant for the same consumer
    // realm; clear the active row first (delete_not_in keeps only revoked)
    let revoked = maestria_domain::RealmReadGrant::from_current_state(
        GrantTokenDigest::derive(b"contract-c"),
        realm('a')?,
        realm('b')?,
        FederatedReadAccess::SearchOnly,
        Sensitivity::Public,
        FederatedEvidenceBounds::try_new(1, 1)?,
        RealmReadGrantState::Revoked,
    );
    repository.put(revoked.clone())?;
    repository.delete_not_in(&BTreeSet::from([revoked.token_digest().clone()]))?;
    let resumed = active_grant(b"contract-d", 'a', 'b', FederatedReadAccess::SearchOnly)?;
    repository.put(resumed.clone())?;
    assert_eq!(
        repository.get(resumed.token_digest())?,
        Some(resumed.clone())
    );

    // delete_not_in removes everything except the kept digests
    repository.delete_not_in(&BTreeSet::from([resumed.token_digest().clone()]))?;
    assert_eq!(repository.list()?, vec![resumed]);
    Ok(())
}
