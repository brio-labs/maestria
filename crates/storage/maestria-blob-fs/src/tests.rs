use super::*;
use maestria_ports::{BlobStore, PortError, contract_tests};
use tempfile::tempdir;

#[test]
fn satisfies_shared_blob_store_contract() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = FsBlobStore::open(root.path())?;

    contract_tests::assert_blob_store_round_trip(&store)?;
    Ok(())
}

#[test]
fn same_bytes_produce_same_id_and_digest() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = FsBlobStore::open(root.path())?;

    let first = store.put_with_digest(b"same bytes".to_vec())?;
    let second = store.put_with_digest(b"same bytes".to_vec())?;

    assert_eq!(first, second);
    assert_eq!(first.1, store.digest_for_id(first.0)?);
    Ok(())
}

#[test]
fn different_bytes_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = FsBlobStore::open(root.path())?;

    let first = store.put(b"first".to_vec())?;
    let second = store.put(b"second".to_vec())?;

    assert_ne!(first, second);
    assert_eq!(store.get(first)?, b"first");
    assert_eq!(store.get(second)?, b"second");
    Ok(())
}

#[test]
fn missing_blob_returns_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = FsBlobStore::open(root.path())?;

    assert_eq!(store.get(BlobId::new(42)), Err(PortError::NotFound));
    Ok(())
}

#[test]
fn tampered_blob_rejected_on_get() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = FsBlobStore::open(root.path())?;

    let bytes = b"untampered payload".to_vec();
    let id = store.put(bytes.clone())?;
    assert_eq!(store.get(id)?, bytes);

    let digest_hex = store.digest_for_id(id)?;
    let object_path = store.object_path_for_digest(&digest_hex)?;
    fs::write(&object_path, b"tampered payload")?;

    let result = store.get(id);
    assert!(matches!(
        &result,
        Err(PortError::InternalContext {
            context: "blob integrity check failed",
            ..
        })
    ));
    Ok(())
}

#[test]
fn swapped_valid_index_digest_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = FsBlobStore::open(root.path())?;

    let first = store.put_with_digest(b"first".to_vec())?;
    let second = store.put_with_digest(b"second".to_vec())?;
    assert_ne!(first.0, second.0);
    fs::write(store.index_path(first.0), second.1.as_bytes())?;

    let digest_result = store.digest_for_id(first.0);
    assert!(matches!(
        &digest_result,
        Err(PortError::InternalContext {
            context: "blob integrity check failed",
            ..
        })
    ));
    let get_result = store.get(first.0);
    assert!(matches!(
        &get_result,
        Err(PortError::InternalContext {
            context: "blob integrity check failed",
            ..
        })
    ));
    assert_eq!(store.get(second.0)?, b"second");
    Ok(())
}

#[test]
fn malformed_index_digest_remains_invalid_input() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = FsBlobStore::open(root.path())?;
    let id = store.put(b"payload".to_vec())?;

    fs::write(store.index_path(id), b"not a digest")?;
    let result = store.get(id);
    assert!(matches!(
        &result,
        Err(PortError::InvalidInputContext {
            context: "invalid blob digest",
            ..
        })
    ));
    Ok(())
}

#[test]
fn stores_on_same_root_share_blobs() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let writer = FsBlobStore::open(root.path())?;
    let reader = FsBlobStore::open(root.path())?;

    let id = writer.put(b"shared".to_vec())?;
    assert_eq!(reader.get(id)?, b"shared");
    Ok(())
}

#[test]
fn digest_derived_paths_stay_under_root() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = FsBlobStore::open(root.path())?;
    let (_, digest) = store.put_with_digest(b"caller cannot pick paths".to_vec())?;

    let object_path = store.object_path_for_digest(&digest)?;
    assert!(object_path.starts_with(store.root()));
    assert!(object_path.exists());
    assert!(
        object_path
            .strip_prefix(store.root())?
            .components()
            .all(|component| !matches!(component, std::path::Component::ParentDir))
    );

    let malicious = "../aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert!(matches!(
        store.object_path_for_digest(malicious),
        Err(PortError::InvalidInputContext { .. })
    ));
    Ok(())
}
