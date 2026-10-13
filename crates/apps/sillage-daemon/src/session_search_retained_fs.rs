use std::{
    ffi::{CStr, CString, OsStr},
    fs::File,
    io,
};

use anyhow::{Context, Result};

pub(super) fn c_name(name: &OsStr) -> Result<CString> {
    CString::new(name.as_encoded_bytes()).context("retention store entry name contains a NUL byte")
}

#[cfg(unix)]
fn open_filesystem_root() -> io::Result<File> {
    use std::os::fd::FromRawFd;

    // SAFETY: the fixed C string is NUL-terminated and open returns a new descriptor.
    let descriptor = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: open returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(not(unix))]
fn open_filesystem_root() -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "anchored retention requires Unix directory handles",
    ))
}

#[path = "session_search_retained_namespace.rs"]
pub(in crate::session_search::retained) mod namespace;

pub(super) fn verify_private_directory(directory: &File) -> Result<()> {
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        anyhow::bail!("retention namespace is not a directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        // SAFETY: geteuid has no pointer or resource preconditions.
        let owner = unsafe { libc::geteuid() };
        if metadata.uid() != owner || metadata.mode() & 0o7777 != super::STORE_DIRECTORY_MODE {
            anyhow::bail!("retention directory is not privately owned by this user");
        }
    }
    #[cfg(not(unix))]
    anyhow::bail!("private anchored retention is unsupported on this platform");
    Ok(())
}

pub(super) fn verify_private_file(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        anyhow::bail!("retention entry is not a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        // SAFETY: geteuid has no pointer or resource preconditions.
        let owner = unsafe { libc::geteuid() };
        if metadata.uid() != owner || metadata.mode() & 0o7777 != super::STORE_FILE_MODE {
            anyhow::bail!("retention file is not privately owned by this user");
        }
    }
    #[cfg(not(unix))]
    anyhow::bail!("private anchored retention is unsupported on this platform");
    Ok(())
}

pub(super) fn set_private_directory_mode(directory: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        // SAFETY: descriptor is owned by the supplied File.
        if unsafe { libc::fchmod(directory.as_raw_fd(), super::STORE_DIRECTORY_MODE) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    anyhow::bail!("private anchored retention is unsupported on this platform");
}

pub(super) fn set_private_file_mode(file: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        // SAFETY: descriptor is owned by the supplied File.
        if unsafe { libc::fchmod(file.as_raw_fd(), super::STORE_FILE_MODE) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    anyhow::bail!("private anchored retention is unsupported on this platform");
}

#[cfg(unix)]
pub(super) fn open_directory_at(parent: &File, name: &CStr) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    // SAFETY: name is NUL-terminated and parent is an owned directory handle.
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(not(unix))]
pub(super) fn open_directory_at(_parent: &File, _name: &CStr) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "anchored retention requires Unix directory handles",
    ))
}

pub(super) fn open_file_at(parent: &File, name: &CStr, create_new: bool) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};

        let (flags, mode) = if create_new {
            (
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                super::STORE_FILE_MODE,
            )
        } else {
            (
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0,
            )
        };
        // SAFETY: name is NUL-terminated and parent is an owned directory handle.
        let descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, mode) };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned descriptor.
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
    #[cfg(not(unix))]
    {
        let _ = (parent, name, create_new);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "anchored retention requires Unix directory handles",
        ))
    }
}

pub(super) fn mkdir_at(parent: &File, name: &CStr, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        // SAFETY: name is NUL-terminated and parent is an owned directory handle.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), mode) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (parent, name, mode);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "anchored retention requires Unix directory handles",
        ))
    }
}

pub(super) fn link_at(parent: &File, source: &CStr, destination: &CStr) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        // SAFETY: both names are NUL-terminated and refer to entries under parent.
        if unsafe {
            libc::linkat(
                parent.as_raw_fd(),
                source.as_ptr(),
                parent.as_raw_fd(),
                destination.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (parent, source, destination);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "anchored retention requires Unix directory handles",
        ))
    }
}

pub(super) fn unlink_at(parent: &File, name: &CStr, directory: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
        // SAFETY: name is NUL-terminated and parent is an owned directory handle.
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (parent, name, directory);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "anchored retention requires Unix directory handles",
        ))
    }
}

pub(super) fn visit_directory_entries(
    directory: &File,
    mut visit: impl FnMut(&OsStr) -> Result<()>,
) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;

        // SAFETY: opening "." relative to the owned directory creates a fresh description.
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: fdopendir takes ownership of this newly opened directory descriptor.
        let stream = unsafe { libc::fdopendir(descriptor) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: fdopendir failed and did not take ownership.
            unsafe { libc::close(descriptor) };
            return Err(error.into());
        }
        let result: Result<()> = (|| {
            loop {
                clear_directory_read_error();
                // SAFETY: stream is a live DIR handle until closedir below.
                let entry = unsafe { libc::readdir(stream) };
                if entry.is_null() {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error().is_some_and(|code| code != 0) {
                        return Err(error.into());
                    }
                    return Ok(());
                }
                // SAFETY: readdir returns a NUL-terminated d_name valid until next readdir.
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if name == b"." || name == b".." {
                    continue;
                }
                visit(OsStr::from_bytes(name))?;
            }
        })();
        // SAFETY: stream is live and closed exactly once here.
        let close_result = unsafe { libc::closedir(stream) };
        result?;
        if close_result != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (directory, &mut visit);
        anyhow::bail!("anchored retention requires Unix directory handles");
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn clear_directory_read_error() {
    // SAFETY: this is the calling thread's errno slot.
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn clear_directory_read_error() {
    // SAFETY: this is the calling thread's errno slot.
    unsafe { *libc::__error() = 0 };
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
fn clear_directory_read_error() {}
