#![cfg(target_os = "linux")]

use std::error::Error;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, SystemTime};

use maestria_extensions::bundle::BundleStore;

const LOCK_FILE: &str = ".bundle-store.lock";
const EXAMPLE: &[u8] =
    include_bytes!("../../../../extension-sdk/examples/greetings.extension.json");
const MODE_ENV: &str = "MAESTRIA_BUNDLE_STORE_LOCK_TEST_MODE";
const ROOT_ENV: &str = "MAESTRIA_BUNDLE_STORE_LOCK_TEST_ROOT";
const SIGNALS_ENV: &str = "MAESTRIA_BUNDLE_STORE_LOCK_TEST_SIGNALS";
const EXTENSION_ENV: &str = "MAESTRIA_BUNDLE_STORE_LOCK_TEST_EXTENSION";

#[test]
fn process_lock_worker() -> Result<(), Box<dyn Error>> {
    let Ok(mode) = std::env::var(MODE_ENV) else {
        return Ok(());
    };
    let root = PathBuf::from(std::env::var_os(ROOT_ENV).ok_or("missing test store root")?);
    let signals = PathBuf::from(std::env::var_os(SIGNALS_ENV).ok_or("missing test signals path")?);
    match mode.as_str() {
        "hold" => {
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true).mode(0o600);
            let file = options.open(root.join(LOCK_FILE))?;
            file.lock()?;
            fs::write(signals.join("holder-ready"), b"ready")?;
            while !signals.join("release-holder").exists() {
                thread::sleep(Duration::from_millis(10));
            }
        }
        "revoke" => {
            fs::write(signals.join("revoker-started"), b"started")?;
            let extension_id = std::env::var(EXTENSION_ENV)?;
            BundleStore::open(root)?.revoke(&extension_id)?;
            fs::write(signals.join("revoker-finished"), b"finished")?;
        }
        _ => return Err(format!("unknown worker mode {mode:?}").into()),
    }
    Ok(())
}

#[test]
fn mutator_waits_for_another_process_store_lock() -> Result<(), Box<dyn Error>> {
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
    let installed = store.install_directory(&source, |_| true)?;
    let signals = temporary.path().join("signals");
    fs::create_dir_all(&signals)?;

    let mut holder = spawn_worker("hold", &store_path, &signals, &installed.extension_id)?;
    if !wait_for_marker(
        &signals.join("holder-ready"),
        &mut holder,
        Duration::from_secs(5),
    )? {
        fs::write(signals.join("release-holder"), b"release")?;
        let _ = wait_for_exit(&mut holder, Duration::from_secs(5))?;
        return Err("lock-holder process did not acquire the store lock".into());
    }

    let mut revoker = spawn_worker("revoke", &store_path, &signals, &installed.extension_id)?;
    let started = wait_for_marker(
        &signals.join("revoker-started"),
        &mut revoker,
        Duration::from_secs(5),
    )?;
    let remained_blocked = started
        && !wait_for_marker(
            &signals.join("revoker-finished"),
            &mut revoker,
            Duration::from_secs(1),
        )?;

    fs::write(signals.join("release-holder"), b"release")?;
    let holder_status = wait_for_exit(&mut holder, Duration::from_secs(5))?;
    let revoker_status = wait_for_exit(&mut revoker, Duration::from_secs(5))?;
    assert!(
        holder_status.success(),
        "lock-holder process failed: {holder_status}"
    );
    assert!(
        revoker_status.success(),
        "revoker process failed: {revoker_status}"
    );
    assert!(
        remained_blocked,
        "the revoker committed while another process held the exclusive store lock"
    );
    assert!(store.active_bundle(&installed.extension_id)?.is_none());
    Ok(())
}

fn spawn_worker(
    mode: &str,
    root: &Path,
    signals: &Path,
    extension_id: &str,
) -> Result<Child, Box<dyn Error>> {
    Ok(Command::new(std::env::current_exe()?)
        .args(["--exact", "process_lock_worker", "--nocapture"])
        .env(MODE_ENV, mode)
        .env(ROOT_ENV, root)
        .env(SIGNALS_ENV, signals)
        .env(EXTENSION_ENV, extension_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?)
}

fn wait_for_marker(marker: &Path, child: &mut Child, timeout: Duration) -> std::io::Result<bool> {
    let deadline = SystemTime::now()
        .checked_add(timeout)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "timeout overflow"))?;
    loop {
        if marker.exists() {
            return Ok(true);
        }
        if child.try_wait()?.is_some() || SystemTime::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::io::Result<ExitStatus> {
    let deadline = SystemTime::now()
        .checked_add(timeout)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "timeout overflow"))?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if SystemTime::now() >= deadline {
            child.kill()?;
            return child.wait();
        }
        thread::sleep(Duration::from_millis(10));
    }
}
