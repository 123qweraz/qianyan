use std::fs::File;
use std::io;
use std::path::Path;

/// Acquire a shared (read) lock on a file.
/// Multiple shared locks can coexist; an exclusive lock blocks all shared locks.
pub fn lock_shared(path: &Path) -> io::Result<LockGuard> {
    let file = File::open(path)?;
    lock_file_shared(&file)?;
    Ok(LockGuard { file })
}

/// Acquire an exclusive (write) lock on a file.
/// Only one exclusive lock can be held at a time; no shared locks coexist.
pub fn lock_exclusive(path: &Path) -> io::Result<LockGuard> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    lock_file_exclusive(&file)?;
    Ok(LockGuard { file })
}

/// A file lock guard — releases the lock when dropped.
pub struct LockGuard {
    /// Kept alive to hold the lock; field intentionally never read.
    #[allow(dead_code)]
    file: File,
}

#[cfg(target_os = "linux")]
fn lock_file_shared(file: &File) -> io::Result<()> {
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(file);
    let ret = unsafe { libc::flock(fd, libc::LOCK_SH) };
    if ret != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn lock_file_exclusive(file: &File) -> io::Result<()> {
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(file);
    let ret = unsafe { libc::flock(fd, libc::LOCK_EX) };
    if ret != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn lock_file_shared(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn lock_file_exclusive(_file: &File) -> io::Result<()> {
    Ok(())
}

impl LockGuard {
    pub fn try_shared(path: &Path) -> io::Result<Option<Self>> {
        let file = File::open(path)?;
        try_lock_shared(&file).map(|acquired| {
            if acquired { Some(LockGuard { file }) } else { None }
        })
    }

    pub fn try_exclusive(path: &Path) -> io::Result<Option<Self>> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        try_lock_exclusive(&file).map(|acquired| {
            if acquired { Some(LockGuard { file }) } else { None }
        })
    }
}

#[cfg(target_os = "linux")]
fn try_lock_shared(file: &File) -> io::Result<bool> {
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(file);
    let ret = unsafe { libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB) };
    if ret == 0 {
        Ok(true)
    } else {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::WouldBlock {
            Ok(false)
        } else {
            Err(err)
        }
    }
}

#[cfg(target_os = "linux")]
fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(file);
    let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if ret == 0 {
        Ok(true)
    } else {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::WouldBlock {
            Ok(false)
        } else {
            Err(err)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn try_lock_shared(_file: &File) -> io::Result<bool> {
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn try_lock_exclusive(_file: &File) -> io::Result<bool> {
    Ok(true)
}
