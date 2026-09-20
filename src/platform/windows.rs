//! Windows backend. Every ancestor is held without FILE_SHARE_DELETE, preventing
//! rename/replacement of the path while it is used. All reparse points are refused.
use std::{
    cell::RefCell,
    ffi::OsStr,
    fs::{self, File},
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    sync::Arc,
    time::SystemTime,
};
use unicode_normalization::UnicodeNormalization;
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*};

pub struct Directory {
    pub file: File,
    path: PathBuf,
    ancestors: Vec<Arc<File>>,
    lock: RefCell<Option<File>>,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut units: Vec<_> = path.as_os_str().encode_wide().collect();
    if units.contains(&0) {
        return Err(invalid("path contains NUL"));
    }
    units.push(0);
    Ok(units)
}
fn open(
    path: &Path,
    access: u32,
    share: u32,
    disposition: u32,
    directory: bool,
) -> io::Result<File> {
    let path = wide(path)?;
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | if directory {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            FILE_FLAG_WRITE_THROUGH
        };
    // SAFETY: terminated UTF-16 input, null optional parameters, returned HANDLE checked.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            share,
            std::ptr::null(),
            disposition,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful CreateFileW transfers unique ownership of this HANDLE.
    let file = unsafe { File::from_raw_handle(handle) };
    let metadata = file.metadata()?;
    if super::is_link(&metadata) {
        return Err(invalid(
            "Windows reparse points and junctions are not followed",
        ));
    }
    if directory != metadata.is_dir() {
        return Err(invalid("unexpected file/directory type"));
    }
    if !directory && !metadata.is_file() {
        return Err(invalid("not a regular file"));
    }
    Ok(file)
}
pub fn open_snapshot(path: &Path) -> io::Result<File> {
    open(
        path,
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        OPEN_EXISTING,
        false,
    )
}
fn directory(path: &Path) -> io::Result<File> {
    // Excluding FILE_SHARE_DELETE pins this directory name, including for other ALB processes.
    open(
        path,
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        OPEN_EXISTING,
        true,
    )
}
pub fn identity(file: &File) -> io::Result<(u64, u64, i64, i64)> {
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: live handle, correctly sized writable output.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: populated on success.
    let info = unsafe { info.assume_init() };
    let mut basic = std::mem::MaybeUninit::<FILE_BASIC_INFO>::uninit();
    // SAFETY: live handle, matching FileBasicInfo buffer layout and size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            basic.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: populated on success.
    let basic = unsafe { basic.assume_init() };
    Ok((
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        basic.ChangeTime,
        0,
    ))
}
fn same(a: &File, b: &File) -> io::Result<bool> {
    let a = identity(a)?;
    let b = identity(b)?;
    Ok((a.0, a.1) == (b.0, b.1))
}
impl Directory {
    pub fn absolute(path: &Path, create: bool, input: Option<&Directory>) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(invalid("root must be absolute"));
        }
        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(invalid("missing Windows volume"));
        };
        if !matches!(
            prefix.kind(),
            Prefix::Disk(_)
                | Prefix::VerbatimDisk(_)
                | Prefix::UNC(_, _)
                | Prefix::VerbatimUNC(_, _)
        ) {
            return Err(invalid("unsupported Windows device namespace"));
        }
        if components.next() != Some(Component::RootDir) {
            return Err(invalid("missing volume root"));
        }
        let root: PathBuf = [prefix.as_os_str(), OsStr::new("\\")].iter().collect();
        let mut current = Self {
            file: directory(&root)?,
            path: root,
            ancestors: vec![],
            lock: RefCell::new(None),
        };
        for component in components {
            let Component::Normal(part) = component else {
                return Err(invalid("noncanonical root"));
            };
            if let Some(input) = input {
                current.reject_inside(input)?;
            }
            current = current.child(part, create)?;
        }
        if let Some(input) = input {
            current.reject_inside(input)?;
            input.reject_inside(&current)?;
        }
        Ok(current)
    }
    fn reject_inside(&self, ancestor: &Self) -> io::Result<()> {
        if same(&self.file, &ancestor.file)? {
            return Err(invalid("physical input/output root overlap"));
        }
        for parent in &self.ancestors {
            if same(parent, &ancestor.file)? {
                return Err(invalid("physical input/output root overlap"));
            }
        }
        Ok(())
    }
    fn child(&self, name: &OsStr, create: bool) -> io::Result<Self> {
        super::component(name)?;
        if create {
            self.check_alias(name)?;
        }
        let path = self.path.join(name);
        let file = match directory(&path) {
            Ok(file) => file,
            Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                match fs::create_dir(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
                directory(&path)?
            }
            Err(e) => return Err(e),
        };
        // Reparse points (including volume mounts) were rejected atomically at open.
        let mut ancestors = self.ancestors.clone();
        ancestors.push(Arc::new(self.file.try_clone()?));
        Ok(Self {
            file,
            path,
            ancestors,
            lock: RefCell::new(None),
        })
    }
    pub fn parent(&self, relative: &Path, create: bool) -> io::Result<(Self, std::ffi::OsString)> {
        let mut parts = relative
            .components()
            .map(|part| match part {
                Component::Normal(name) => Ok(name.to_os_string()),
                _ => Err(invalid("unsafe relative path")),
            })
            .collect::<io::Result<Vec<_>>>()?;
        let filename = parts.pop().ok_or_else(|| invalid("empty relative path"))?;
        super::component(&filename)?;
        let mut current = Self {
            file: self.file.try_clone()?,
            path: self.path.clone(),
            ancestors: self.ancestors.clone(),
            lock: RefCell::new(None),
        };
        for part in parts {
            current = current.child(&part, create)?;
        }
        Ok((current, filename))
    }
    pub fn check_alias(&self, wanted: &OsStr) -> io::Result<()> {
        let key = |name: &OsStr| {
            name.to_string_lossy()
                .to_lowercase()
                .nfc()
                .collect::<String>()
        };
        for entry in fs::read_dir(&self.path)? {
            let other = entry?.file_name();
            if other != wanted && key(&other) == key(wanted) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("output name alias: {other:?}"),
                ));
            }
        }
        Ok(())
    }
    pub fn read(&self, name: &OsStr) -> io::Result<File> {
        super::component(name)?;
        open_snapshot(&self.path.join(name))
    }
    pub fn create(&self, name: &OsStr) -> io::Result<File> {
        super::component(name)?;
        self.check_alias(name)?;
        open(
            &self.path.join(name),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            CREATE_NEW,
            false,
        )
    }
    pub fn absent(&self, name: &OsStr) -> io::Result<()> {
        super::component(name)?;
        self.check_alias(name)?;
        match fs::symlink_metadata(self.path.join(name)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination already exists",
            )),
        }
    }
    pub fn publish(&self, partial: &OsStr, destination: &OsStr) -> io::Result<()> {
        super::component(partial)?;
        super::component(destination)?;
        self.check_alias(destination)?;
        let from = wide(&self.path.join(partial))?;
        let to = wide(&self.path.join(destination))?;
        // SAFETY: both terminated names are within the same pinned directory.
        // No REPLACE_EXISTING or COPY_ALLOWED: never replace, never cross volumes.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn lock(&self) -> io::Result<()> {
        if self.lock.borrow().is_some() {
            return Ok(());
        }
        let name = OsStr::new(".alb-build.lock");
        self.check_alias(name)?;
        let file = open(
            &self.path.join(name),
            GENERIC_READ | GENERIC_WRITE,
            0,
            OPEN_ALWAYS,
            false,
        )
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("cannot exclusively lock output: {e}"),
            )
        })?;
        *self.lock.borrow_mut() = Some(file);
        Ok(())
    }
    /// Windows has no supported directory FlushFileBuffers. New files are flushed
    /// and publication requests write-through; directory crash durability is FS-specific.
    pub fn sync(&self) -> io::Result<()> {
        Ok(())
    }
    pub fn available_space(&self) -> io::Result<u64> {
        let path = wide(&self.path)?;
        let mut available = 0;
        // SAFETY: pinned directory path and valid u64 output; optional outputs null.
        if unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(available)
    }
}
pub fn set_creation(file: &File, created: SystemTime) -> io::Result<()> {
    let ns = match created.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    };
    let ticks: u64 = (ns.div_euclid(100) + 116_444_736_000_000_000i128)
        .try_into()
        .map_err(|_| invalid("creation time outside FILETIME range"))?;
    let time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    // SAFETY: live writable file handle, valid creation FILETIME, optional times null.
    if unsafe {
        SetFileTime(
            file.as_raw_handle(),
            &time,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_directory_cannot_be_replaced_and_publication_never_overwrites() {
        let root = (0..)
            .find_map(|n| {
                let path = std::env::temp_dir()
                    .join(format!("alb-win-handles-{}-{n}", std::process::id()));
                fs::create_dir(&path)
                    .ok()
                    .map(|_| fs::canonicalize(path).unwrap())
            })
            .unwrap();
        {
            let directory = Directory::absolute(&root, false, None).unwrap();
            let (parent, name) = directory.parent(Path::new("folder/final"), true).unwrap();
            parent.create(OsStr::new("partial")).unwrap();
            fs::write(root.join("folder/final"), b"existing").unwrap();
            assert!(parent.publish(OsStr::new("partial"), &name).is_err());
            assert_eq!(fs::read(root.join("folder/final")).unwrap(), b"existing");
            assert!(root.join("folder/partial").exists());
            assert!(fs::rename(root.join("folder"), root.join("moved")).is_err());
            assert!(parent.create(OsStr::new("stream:secret")).is_err());
            assert!(parent.read(OsStr::new("../escape")).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
