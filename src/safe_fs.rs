//! Linux directory-handle operations. Never fall back to path-based writes.
use rustix::fs::{
    self as sys, AtFlags, Dir, FlockOperation, Mode, OFlags, RenameFlags, ResolveFlags,
};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path},
};
use unicode_normalization::UnicodeNormalization;

pub struct Directory {
    pub file: File,
}
fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text)
}
fn flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW
}
fn resolve() -> ResolveFlags {
    ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV
}
fn name(name: &OsStr) -> io::Result<()> {
    let path = Path::new(name);
    if !matches!(path.components().next(), Some(Component::Normal(_)))
        || path.components().count() != 1
    {
        return Err(invalid("expected one normal path component"));
    }
    Ok(())
}
impl Directory {
    /// Canonical absolute roots may cross filesystems, but never symlinks.
    /// Refuse creating anything under the opened input root.
    pub fn absolute(path: &Path, create: bool, input: Option<&Directory>) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(invalid("root must be absolute"));
        }
        let mut current = Self {
            file: File::from(sys::open("/", flags(), Mode::empty())?),
        };
        for component in path.components() {
            let Component::Normal(part) = component else {
                if component == Component::RootDir {
                    continue;
                }
                return Err(invalid("noncanonical root"));
            };
            match sys::openat(&current.file, part, flags(), Mode::empty()) {
                Ok(fd) => current = Self { file: fd.into() },
                Err(rustix::io::Errno::NOENT) if create => {
                    if let Some(input) = input {
                        current.reject_inside(input)?;
                    }
                    current.check_alias(part)?;
                    match sys::mkdirat(&current.file, part, Mode::from_raw_mode(0o755)) {
                        Ok(()) => current.file.sync_all()?,
                        Err(rustix::io::Errno::EXIST) => {}
                        Err(e) => return Err(e.into()),
                    }
                    current = Self {
                        file: sys::openat(&current.file, part, flags(), Mode::empty())?.into(),
                    };
                }
                Err(e) => return Err(e.into()),
            }
        }
        if let Some(input) = input {
            current.reject_inside(input)?;
            input.reject_inside(&current)?;
        }
        Ok(current)
    }
    pub fn lock(&self) -> io::Result<()> {
        sys::flock(&self.file, FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("output is locked by another build: {e}"),
            )
        })
    }
    fn reject_inside(&self, ancestor: &Self) -> io::Result<()> {
        let expected = ancestor.file.metadata()?;
        let mut dir = self.file.try_clone()?;
        loop {
            let here = dir.metadata()?;
            if (here.dev(), here.ino()) == (expected.dev(), expected.ino()) {
                return Err(invalid("physical input/output root overlap"));
            }
            let parent: File = sys::openat(&dir, "..", flags(), Mode::empty())?.into();
            let up = parent.metadata()?;
            if (here.dev(), here.ino()) == (up.dev(), up.ino()) {
                return Ok(());
            }
            dir = parent;
        }
    }
    pub fn check_alias(&self, wanted: &OsStr) -> io::Result<()> {
        let Some(wanted_key) = wanted
            .to_str()
            .map(|s| s.to_lowercase().nfc().collect::<String>())
        else {
            return Ok(());
        };
        for item in Dir::read_from(&self.file)? {
            let item = item?;
            let other = OsStr::from_bytes(item.file_name().to_bytes());
            if other != wanted
                && other
                    .to_str()
                    .is_some_and(|s| s.to_lowercase().nfc().collect::<String>() == wanted_key)
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("output name alias: {other:?}"),
                ));
            }
        }
        Ok(())
    }
    pub fn parent(&self, relative: &Path, create: bool) -> io::Result<(Self, OsString)> {
        let mut parts: Vec<_> = relative
            .components()
            .map(|c| match c {
                Component::Normal(n) => Ok(n.to_os_string()),
                _ => Err(invalid("unsafe relative path")),
            })
            .collect::<io::Result<_>>()?;
        let filename = parts.pop().ok_or_else(|| invalid("empty relative path"))?;
        let mut current = Self {
            file: self.file.try_clone()?,
        };
        for part in parts {
            if create {
                current.check_alias(&part)?;
            }
            match sys::openat2(&current.file, &part, flags(), Mode::empty(), resolve()) {
                Ok(fd) => current = Self { file: fd.into() },
                Err(rustix::io::Errno::NOENT) if create => {
                    match sys::mkdirat(&current.file, &part, Mode::from_raw_mode(0o755)) {
                        Ok(()) => current.file.sync_all()?,
                        Err(rustix::io::Errno::EXIST) => {}
                        Err(e) => return Err(e.into()),
                    }
                    current = Self {
                        file: sys::openat2(
                            &current.file,
                            &part,
                            flags(),
                            Mode::empty(),
                            resolve(),
                        )?
                        .into(),
                    };
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok((current, filename))
    }
    pub fn read(&self, name_: &OsStr) -> io::Result<File> {
        name(name_)?;
        Ok(sys::openat2(
            &self.file,
            name_,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
            resolve(),
        )?
        .into())
    }
    pub fn create(&self, name_: &OsStr) -> io::Result<File> {
        name(name_)?;
        self.check_alias(name_)?;
        Ok(sys::openat2(
            &self.file,
            name_,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_raw_mode(0o600),
            resolve(),
        )?
        .into())
    }
    pub fn absent(&self, name_: &OsStr) -> io::Result<()> {
        name(name_)?;
        self.check_alias(name_)?;
        match sys::statat(&self.file, name_, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination already exists",
            )),
            Err(e) => Err(e.into()),
        }
    }
    pub fn publish(&self, partial: &OsStr, destination: &OsStr) -> io::Result<()> {
        name(partial)?;
        name(destination)?;
        self.check_alias(destination)?;
        sys::renameat_with(
            &self.file,
            partial,
            &self.file,
            destination,
            RenameFlags::NOREPLACE,
        )?;
        self.file.sync_all()
    }
}
