use std::fs;

use maestria_extensions::Permission;
use maestria_extensions::bundle::{BundleError, BundleStore};
use serde_json::Value;

const EXAMPLE: &[u8] =
    include_bytes!("../../../../extension-sdk/examples/greetings.extension.json");

#[test]
fn approval_precedes_store_writes_and_denied_updates_preserve_active_grants()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let source = temporary.path().join("source");
    fs::create_dir_all(source.join("dist"))?;
    fs::write(source.join("manifest.json"), EXAMPLE)?;
    fs::write(
        source.join("dist/main.js"),
        b"export default { commands: { greetings: async () => ({ kind: 'list', title: 'Greetings', items: [] }) } };",
    )?;

    let store_path = temporary.path().join("store");
    let store = BundleStore::open(store_path.clone())?;
    let initial = store.preview_directory(&source)?;
    assert!(!store_path.exists(), "preview wrote to an unapproved store");
    assert!(matches!(
        store.install_directory(&source, |_| false),
        Err(BundleError::ApprovalDenied)
    ));
    assert!(
        !store_path.exists(),
        "denied installation created store state"
    );

    store
        .install_directory(&source, |actual| actual.matches(&initial))
        .map_err(|error| format!("approved install failed: {error}"))?;
    let active = store
        .active_bundle(&initial.manifest.id)
        .map_err(|error| format!("approved package revalidation failed: {error}"))?
        .ok_or("approved package is inactive")?;
    let original = active.package;
    let original_grants = active.granted_permissions;

    let mut expanded: Value = serde_json::from_slice(EXAMPLE)?;
    expanded["version"] = Value::from("1.1.0");
    expanded["permissions"]
        .as_array_mut()
        .ok_or("example manifest permissions are missing")?
        .push(serde_json::from_str(r#"{"type":"userFileRead"}"#)?);
    fs::write(source.join("manifest.json"), serde_json::to_vec(&expanded)?)?;
    let proposal = store.preview_directory(&source)?;
    assert!(
        proposal
            .permissions
            .added
            .contains(&Permission::UserFileRead)
    );
    assert!(matches!(
        store.install_directory(&source, |_| false),
        Err(BundleError::ApprovalDenied)
    ));
    let unchanged = store
        .active_bundle(&initial.manifest.id)?
        .ok_or("denied update disabled active package")?;
    assert_eq!(unchanged.package, original);
    assert_eq!(unchanged.granted_permissions, original_grants);

    expanded["version"] = Value::from("1.2.0");
    fs::write(source.join("manifest.json"), serde_json::to_vec(&expanded)?)?;
    assert!(matches!(
        store.install_directory(&source, |actual| actual.matches(&proposal)),
        Err(BundleError::ApprovalDenied)
    ));
    let unchanged = store
        .active_bundle(&initial.manifest.id)?
        .ok_or("changed proposal disabled active package")?;
    assert_eq!(unchanged.package, original);
    assert_eq!(unchanged.granted_permissions, original_grants);
    Ok(())
}
