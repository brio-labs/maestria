use std::ffi::OsStr;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;

use tokio::process::Command;

const BUBBLEWRAP: &str = "/usr/bin/bwrap";

/// Constructs, but does not spawn, a worker command. If bubblewrap, its
/// namespace setup, or any required read-only bind is unavailable, spawn or
/// startup fails; callers must never execute the worker directly instead.
/// The caller supplies only worker arguments referring to `/extension`.
pub(super) fn worker_command(
    worker_binary: &Path,
    installed_bundle: &Path,
    worker_args: &[&OsStr],
) -> io::Result<Command> {
    let worker_binary = regular_file(worker_binary)?;
    let installed_bundle = real_directory(installed_bundle)?;
    if !Path::new(BUBBLEWRAP).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "bubblewrap is required for extensions",
        ));
    }

    let mut command = Command::new(BUBBLEWRAP);
    command.env_clear();
    command.args([
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--disable-userns",
        "--cap-drop",
        "ALL",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "HOME",
        "/home/extension",
        "--setenv",
        "LANG",
        "C.UTF-8",
        "--setenv",
        "PATH",
        "/usr/bin",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind-try",
        "/lib",
        "/lib",
        "--ro-bind-try",
        "/lib64",
        "/lib64",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--dir",
        "/home",
        "--size",
        "16777216",
        "--tmpfs",
        "/tmp",
        "--chdir",
        "/extension",
    ]);
    command.arg("--ro-bind").arg(worker_binary).arg("/worker");
    command
        .arg("--ro-bind")
        .arg(installed_bundle)
        .arg("/extension");
    command.arg("--").arg("/worker");
    command.args(worker_args);
    command.kill_on_drop(true);
    apply_resource_limits(&mut command);
    Ok(command)
}

fn apply_resource_limits(command: &mut Command) {
    // Only async-signal-safe setrlimit calls execute in the forked child.
    // A failed limit aborts spawn; never run a worker without every bound.
    unsafe {
        command.as_std_mut().pre_exec(|| {
            for (resource, cap) in [
                (libc::RLIMIT_AS, 512 * 1024 * 1024_u64),
                (libc::RLIMIT_CPU, 32_u64),
                (libc::RLIMIT_NPROC, 1024_u64),
                (libc::RLIMIT_NOFILE, 64_u64),
                (libc::RLIMIT_FSIZE, 16 * 1024 * 1024_u64),
                (libc::RLIMIT_CORE, 0_u64),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: cap,
                    rlim_max: cap,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

fn regular_file(path: &Path) -> io::Result<std::path::PathBuf> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker binary must be a regular file",
        ));
    }
    path.canonicalize()
}

fn real_directory(path: &Path) -> io::Result<std::path::PathBuf> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "installed extension must be a real directory",
        ));
    }
    path.canonicalize()
}
