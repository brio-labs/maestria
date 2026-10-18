use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn unmarked_existing_profile_is_not_adopted_or_chmodded() -> Result<(), Box<dyn std::error::Error>>
{
    let directory =
        std::env::temp_dir().join(format!("sillage-search-consent-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let profile_root = directory.join("sillage/launcher-search");
    fs::create_dir_all(&profile_root)?;
    fs::set_permissions(&profile_root, fs::Permissions::from_mode(0o755))?;
    let plan = EnablePlan {
        root: directory.clone(),
        startup_root: directory.clone(),
        previously_consented_root: None,
        program: directory.join("sillage-search"),
        socket_path: profile_root.join("system/daemon.sock"),
        profile_root: profile_root.clone(),
        revision: 0,
        previous_service: None,
        previous_managed: None,
    };
    let setup = SearchSetup::new();
    let mut owned = OwnedState::default();

    assert!(setup.prepare_enable_profile(&mut owned, &plan).is_err());
    assert_eq!(
        fs::metadata(&profile_root)?.permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read_dir(&profile_root)?.count(), 0);
    assert!(owned.marker.is_none());
    assert!(owned.profile_lock.is_none());
    fs::remove_dir_all(directory)?;
    Ok(())
}
