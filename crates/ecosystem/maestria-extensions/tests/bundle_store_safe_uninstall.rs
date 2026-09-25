#![cfg(target_os = "linux")]

use std::error::Error;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use maestria_extensions::bundle::{BundleStore, InstallReceipt};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use rustix::io::Errno;

const EXAMPLE: &[u8] =
    include_bytes!("../../../../extension-sdk/examples/greetings.extension.json");

fn install_example(root: &Path) -> Result<(BundleStore, InstallReceipt, PathBuf), Box<dyn Error>> {
    let source = root.join("source");
    fs::create_dir_all(source.join("dist"))?;
    fs::write(source.join("manifest.json"), EXAMPLE)?;
    fs::write(
        source.join("dist/main.js"),
        b"export default { commands: { greetings: async () => ({ kind: 'list', title: 'Greetings', items: [] }) } };",
    )?;

    let store_path = root.join("store");
    let store = BundleStore::open(store_path.clone())?;
    let receipt = store.install_directory(&source, |_| true)?;
    Ok((store, receipt, store_path))
}

fn file_mode(path: &Path) -> Result<u32, Box<dyn Error>> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o777)
}

#[test]
fn uninstall_removes_readonly_package_and_data_without_following_static_symlinks()
-> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let (store, receipt, store_path) = install_example(temporary.path())?;
    let extension_id = receipt.extension_id;
    let package = store_path
        .join("packages")
        .join(&extension_id)
        .join(format!(
            "{}-{}",
            receipt.package.version, receipt.package.sha256
        ));
    let package_file = package.join("dist/main.js");
    assert_eq!(file_mode(&package)?, 0o555);
    assert_eq!(file_mode(&package_file)?, 0o444);

    let data = store_path.join("data").join(&extension_id);
    let readonly_data = data.join("readonly");
    fs::create_dir(&readonly_data)?;
    let data_file = readonly_data.join("state.bin");
    fs::write(&data_file, b"extension state")?;
    fs::set_permissions(&data_file, fs::Permissions::from_mode(0o444))?;

    let outside = temporary.path().join("outside");
    fs::create_dir(&outside)?;
    let outside_file = outside.join("host-state.bin");
    fs::write(&outside_file, b"host state")?;
    fs::set_permissions(&outside_file, fs::Permissions::from_mode(0o444))?;
    symlink(&outside_file, readonly_data.join("host-link"))?;
    fs::set_permissions(&readonly_data, fs::Permissions::from_mode(0o555))?;

    store.uninstall(&extension_id, false)?;

    assert!(!package.exists(), "sealed package remained after uninstall");
    assert!(!data.exists(), "extension data remained after uninstall");
    assert_eq!(fs::read(&outside_file)?, b"host state");
    assert_eq!(file_mode(&outside_file)?, 0o444);
    Ok(())
}

#[test]
fn uninstall_does_not_follow_a_concurrently_exchanged_data_symlink() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let (store, receipt, store_path) = install_example(temporary.path())?;
    let extension_id = receipt.extension_id;
    let data = store_path.join("data").join(&extension_id);
    let raced_directory = data.join("changing-tree");
    let exchange_slot = store_path.join("data").join("exchange-slot");
    fs::create_dir(&raced_directory)?;
    for index in 0..512 {
        fs::write(raced_directory.join(format!("entry-{index}")), b"data")?;
    }

    let outside = temporary.path().join("outside");
    fs::create_dir(&outside)?;
    let outside_file = outside.join("host-state.bin");
    fs::write(&outside_file, b"host state")?;
    fs::set_permissions(&outside_file, fs::Permissions::from_mode(0o444))?;
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o555))?;
    symlink(&outside, &exchange_slot)?;

    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let mut started = false;
        while !worker_stop.load(Ordering::Relaxed) {
            match renameat_with(
                CWD,
                &raced_directory,
                CWD,
                &exchange_slot,
                RenameFlags::EXCHANGE,
            ) {
                Ok(()) => {
                    if !started {
                        started = true;
                        let _ = started_tx.send(());
                    }
                    thread::sleep(Duration::from_micros(50));
                }
                Err(Errno::NOENT) => break,
                Err(Errno::INTR) => {}
                Err(_) => break,
            }
        }
        started
    });
    if let Err(source) = started_rx.recv_timeout(Duration::from_secs(2)) {
        stop.store(true, Ordering::Relaxed);
        let _ = worker.join();
        return Err(Box::new(source));
    }

    let first_uninstall = store.uninstall(&extension_id, false);
    stop.store(true, Ordering::Relaxed);
    let raced = worker
        .join()
        .map_err(|_| std::io::Error::other("symlink exchange worker panicked"))?;
    assert!(raced, "the symlink exchange did not start");
    if first_uninstall.is_err() {
        store.uninstall(&extension_id, false)?;
    }

    let observed_directory_mode = file_mode(&outside)?;
    let observed_file_mode = file_mode(&outside_file).ok();
    let observed_file_contents = fs::read(&outside_file).ok();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o700))?;
    assert_eq!(observed_directory_mode, 0o555);
    assert_eq!(observed_file_mode, Some(0o444));
    assert_eq!(observed_file_contents.as_deref(), Some(&b"host state"[..]));
    Ok(())
}
