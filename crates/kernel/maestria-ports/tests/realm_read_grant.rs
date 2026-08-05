use std::collections::BTreeSet;

use maestria_domain::{
    FederatedEvidenceBounds, FederatedReadAccess, GrantTokenDigest, RealmId, RealmReadGrant,
    Sensitivity,
};
use maestria_ports::{InMemoryRealmReadGrantRepository, RealmReadGrantRepository};

fn realm(byte: char) -> Result<RealmId, Box<dyn std::error::Error>> {
    Ok(RealmId::try_from(byte.to_string().repeat(64))?)
}

fn grant(credential: &[u8], consumer: char) -> Result<RealmReadGrant, Box<dyn std::error::Error>> {
    Ok(RealmReadGrant::new(
        GrantTokenDigest::derive(credential),
        realm('a')?,
        realm(consumer)?,
        FederatedReadAccess::SearchOnly,
        Sensitivity::Internal,
        FederatedEvidenceBounds::try_new(1, 1)?,
    ))
}

#[test]
fn current_grant_repository_replaces_and_cleans_projection()
-> Result<(), Box<dyn std::error::Error>> {
    let repository = InMemoryRealmReadGrantRepository::new();
    let first = grant(b"first", 'b')?;
    let second = grant(b"second", 'c')?;
    repository.put(first.clone())?;
    repository.put(second.clone())?;
    assert_eq!(repository.get(first.token_digest())?, Some(first.clone()));
    assert_eq!(repository.list()?.len(), 2);

    repository.delete_not_in(&BTreeSet::from([second.token_digest().clone()]))?;
    assert_eq!(repository.get(first.token_digest())?, None);
    assert_eq!(repository.list()?, vec![second]);
    Ok(())
}

#[test]
fn repository_rejects_two_active_grants_for_one_consumer() -> Result<(), Box<dyn std::error::Error>>
{
    let repository = InMemoryRealmReadGrantRepository::new();
    repository.put(grant(b"first", 'b')?)?;
    assert!(matches!(
        repository.put(grant(b"second", 'b')?),
        Err(maestria_ports::PortError::Conflict { .. })
    ));
    Ok(())
}
