//! OS boundary: directory confinement, atomic publication, identity, space and time.
//! Shared stages never use OS-specific APIs or path-based output mutations.
use std::{
    ffi::OsStr,
    fs::{self, File},
    io,
    path::{Component, Path},
    time::SystemTime,
};
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::Directory;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Directory;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
compile_error!("ALB currently supports Linux, macOS and Windows.");

pub fn component(name: &OsStr) -> io::Result<()> {
    let path = Path::new(name);
    if !matches!(path.components().next(), Some(Component::Normal(_)))
        || path.components().count() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected one normal path component",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // Reject alternate streams and Win32 aliases, even on verbatim paths.
        let units: Vec<_> = name.encode_wide().collect();
        if units
            .iter()
            .any(|&c| c < 32 || b"<>:\"/\\|?*".iter().any(|&b| u16::from(b) == c))
            || matches!(units.last(), Some(32 | 46))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe Windows filename",
            ));
        }
    }
    Ok(())
}
pub fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }
    #[cfg(unix)]
    {
        metadata.is_symlink()
    }
}
pub fn identity(file: &File) -> io::Result<(u64, u64, i64, i64)> {
    #[cfg(windows)]
    {
        windows::identity(file)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        Ok((m.dev(), m.ino(), m.ctime(), m.ctime_nsec()))
    }
}
pub fn open_snapshot(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    {
        windows::open_snapshot(path)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(
                rustix::fs::OFlags::NOFOLLOW.bits() as i32
                    | rustix::fs::OFlags::NONBLOCK.bits() as i32,
            )
            .open(path)
    }
}
pub const CREATION_POLICY: &str = if cfg!(target_os = "linux") {
    "Linux destination birth time cannot be restored; original birth time is archived here."
} else {
    "Source birth time is restored on new copies when supplied by the source filesystem; exact timestamp support is required."
};
pub fn set_times(file: &File, modified: SystemTime, created: Option<SystemTime>) -> io::Result<()> {
    file.set_times(fs::FileTimes::new().set_modified(modified))?;
    #[cfg(target_os = "macos")]
    if let Some(created) = created {
        set_macos_creation(file, created)?;
    }
    #[cfg(windows)]
    if let Some(created) = created {
        windows::set_creation(file, created)?;
    }
    verify_times(&file.metadata()?, modified, created)?;
    file.sync_all()
}
pub fn verify_times(
    metadata: &fs::Metadata,
    modified: SystemTime,
    created: Option<SystemTime>,
) -> io::Result<()> {
    if metadata.modified()? != modified {
        return Err(io::Error::other(
            "destination modification time differs from source (filesystem timestamp precision may be unsupported)",
        ));
    }
    if !cfg!(target_os = "linux")
        && let Some(created) = created
        && metadata.created()? != created
    {
        return Err(io::Error::other(
            "destination creation time differs from source",
        ));
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn set_macos_creation(file: &File, created: SystemTime) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let ns = match created.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    };
    let mut time = libc::timespec {
        tv_sec: ns
            .div_euclid(1_000_000_000)
            .try_into()
            .map_err(|_| io::Error::other("creation time out of range"))?,
        tv_nsec: ns.rem_euclid(1_000_000_000) as _,
    };
    let mut attrs = libc::attrlist {
        bitmapcount: 5,
        reserved: 0,
        commonattr: libc::ATTR_CMN_CRTIME,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    // SAFETY: live owned fd, correctly laid out attrlist and timespec, exact buffer size.
    let result = unsafe {
        libc::fsetattrlist(
            file.as_raw_fd(),
            (&mut attrs as *mut libc::attrlist).cast(),
            (&mut time as *mut libc::timespec).cast(),
            std::mem::size_of_val(&time),
            0,
        )
    };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn terminal_progress_supported() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::*;
        // SAFETY: only querying/updating console output mode for the process stderr.
        unsafe {
            let handle = GetStdHandle(STD_ERROR_HANDLE);
            let mut mode = 0;
            GetConsoleMode(handle, &mut mode) != 0
                && SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
        }
    }
    #[cfg(unix)]
    {
        true
    }
}
