//! Read-only discovery for a stable source tree. No file contents are opened.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct DiscoveryError {
    pub path: PathBuf,
    pub source: io::Error,
}

#[derive(Debug, Default)]
pub struct Catalog {
    /// Regular-file paths sorted by native PathBuf ordering, not display strings.
    pub files: Vec<PathBuf>,
    /// Successfully opened directories, including the root.
    pub directories: usize,
    pub skipped_symlinks: usize,
    pub skipped_special: usize,
    pub errors: Vec<DiscoveryError>,
}

fn error(path: &Path, source: io::Error) -> DiscoveryError {
    DiscoveryError {
        path: path.to_path_buf(),
        source,
    }
}

/// Root must be the canonical input from path validation. A root failure is
/// fatal; errors below it are retained in the catalog. Hardlinks remain separate
/// paths. Concurrent changes to the source tree are not race-proofed.
pub fn discover(root: &Path) -> Result<Catalog, DiscoveryError> {
    discover_with(root, |path| {
        fs::read_dir(path).map(|entries| entries.map(|entry| entry.map(|entry| entry.path())))
    })
}

// Keep directory enumeration injectable so failures can be tested without races
// or permission assumptions (for example, tests running as root).
fn discover_with<I>(
    root: &Path,
    mut read_directory: impl FnMut(&Path) -> io::Result<I>,
) -> Result<Catalog, DiscoveryError>
where
    I: IntoIterator<Item = io::Result<PathBuf>>,
{
    let progress = crate::progress::Progress::new("Discovering files", None);
    let metadata = fs::symlink_metadata(root).map_err(|e| error(root, e))?;
    if !metadata.is_dir() {
        return Err(error(
            root,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "discovery root must be a directory, not a symlink or file",
            ),
        ));
    }
    let entries = read_directory(root).map_err(|e| error(root, e))?;
    let mut catalog = Catalog {
        directories: 1,
        ..Catalog::default()
    };
    let mut pending = Vec::new();
    collect_entries(root, entries, &mut pending, &mut catalog);
    while let Some(path) = pending.pop() {
        progress.set(catalog.files.len() + catalog.directories);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) => {
                catalog.errors.push(error(&path, e));
                continue;
            }
        };
        if crate::platform::is_link(&metadata) {
            catalog.skipped_symlinks += 1;
        } else if metadata.is_dir() {
            match read_directory(&path) {
                Ok(entries) => {
                    catalog.directories += 1;
                    collect_entries(&path, entries, &mut pending, &mut catalog);
                }
                Err(e) => catalog.errors.push(error(&path, e)),
            }
        } else if metadata.is_file() {
            catalog.files.push(path);
        } else {
            catalog.skipped_special += 1;
        }
    }
    catalog.files.sort();
    catalog.errors.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.source.to_string().cmp(&b.source.to_string()))
    });
    Ok(catalog)
}

fn collect_entries(
    directory: &Path,
    entries: impl IntoIterator<Item = io::Result<PathBuf>>,
    pending: &mut Vec<PathBuf>,
    catalog: &mut Catalog,
) {
    for entry in entries {
        match entry {
            Ok(path) => pending.push(path),
            // read_dir errors may not identify the failed child's name.
            Err(e) => catalog.errors.push(error(directory, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            for attempt in 0.. {
                let path = std::env::temp_dir()
                    .join(format!("alb-discovery-{}-{attempt}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(fs::canonicalize(path).unwrap()),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("cannot create fixture: {e}"),
                }
            }
            unreachable!()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn catalogs_nested_files_in_sorted_order_and_preserves_contents() {
        let f = Fixture::new();
        fs::create_dir_all(f.0.join("artist/album")).unwrap();
        fs::create_dir(f.0.join("empty")).unwrap();
        let names = [
            "z.txt",
            "artist/album/b.mp3",
            "artist/album/a.flac",
            ".hidden",
        ];
        for name in names {
            fs::write(f.0.join(name), name.as_bytes()).unwrap();
        }
        let mut expected: Vec<_> = names.iter().map(|name| f.0.join(name)).collect();
        expected.sort();
        for _ in 0..2 {
            let catalog = discover(&f.0).unwrap();
            assert_eq!(catalog.files, expected);
            assert_eq!(catalog.directories, 4);
            assert_eq!(catalog.skipped_symlinks, 0);
            assert_eq!(catalog.skipped_special, 0);
            assert!(catalog.errors.is_empty());
        }
        for name in names {
            assert_eq!(fs::read(f.0.join(name)).unwrap(), name.as_bytes());
        }
    }

    #[test]
    fn empty_root_counts_as_one_directory_and_invalid_roots_fail() {
        let f = Fixture::new();
        let catalog = discover(&f.0).unwrap();
        assert_eq!(catalog.directories, 1);
        assert!(catalog.files.is_empty());
        assert!(catalog.errors.is_empty());
        assert!(discover(&f.0.join("missing")).is_err());
        fs::write(f.0.join("file"), b"test").unwrap();
        assert!(discover(&f.0.join("file")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlink_files_directories_dangling_links_and_loops() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let outside = Fixture::new();
        fs::write(outside.0.join("external"), b"outside").unwrap();
        fs::write(f.0.join("track"), b"inside").unwrap();
        symlink(&outside.0, f.0.join("external-dir")).unwrap();
        symlink(outside.0.join("external"), f.0.join("external-file")).unwrap();
        symlink("missing", f.0.join("dangling")).unwrap();
        symlink(".", f.0.join("loop")).unwrap();
        let catalog = discover(&f.0).unwrap();
        assert_eq!(catalog.files, vec![f.0.join("track")]);
        assert_eq!(catalog.directories, 1);
        assert_eq!(catalog.skipped_symlinks, 4);
        assert!(catalog.errors.is_empty());
        assert!(discover(&f.0.join("external-dir")).is_err());
    }
    #[test]
    fn errors_do_not_hide_good_entries_and_are_sorted() {
        let f = Fixture::new();
        let good = f.0.join("good");
        let blocked = f.0.join("blocked");
        let vanished = f.0.join("vanished");
        fs::write(&good, b"fixture").unwrap();
        fs::create_dir(&blocked).unwrap();
        let catalog = discover_with(&f.0, |path| {
            if path == f.0 {
                Ok(vec![
                    Ok(good.clone()),
                    Err(io::Error::other("injected entry failure")),
                    Ok(vanished.clone()),
                    Ok(blocked.clone()),
                ])
            } else {
                assert_eq!(path, blocked);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected directory failure",
                ))
            }
        })
        .unwrap();
        assert_eq!(catalog.files, vec![good]);
        assert_eq!(catalog.directories, 1);
        assert_eq!(catalog.errors.len(), 3);
        assert_eq!(catalog.errors[0].path, f.0);
        assert_eq!(
            catalog.errors[0].source.to_string(),
            "injected entry failure"
        );
        assert_eq!(catalog.errors[1].path, blocked);
        assert_eq!(
            catalog.errors[1].source.kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(catalog.errors[2].path, vanished);
        assert_eq!(catalog.errors[2].source.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn root_enumeration_failure_is_fatal() {
        let f = Fixture::new();
        let result = discover_with::<Vec<io::Result<PathBuf>>>(&f.0, |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected root failure",
            ))
        });
        let error = result.unwrap_err();
        assert_eq!(error.path, f.0);
        assert_eq!(error.source.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn creation_order_does_not_change_catalog_order_and_hardlinks_survive() {
        let names = ["z", "a", "m"];
        let mut catalogs = Vec::new();
        for reverse in [false, true] {
            let f = Fixture::new();
            let mut order = names.to_vec();
            if reverse {
                order.reverse();
            }
            for name in order {
                fs::write(f.0.join(name), b"fixture").unwrap();
            }
            fs::hard_link(f.0.join("a"), f.0.join("b")).unwrap();
            let catalog = discover(&f.0).unwrap();
            assert!(catalog.errors.is_empty());
            let relative: Vec<_> = catalog
                .files
                .iter()
                .map(|path| path.strip_prefix(&f.0).unwrap().to_path_buf())
                .collect();
            assert_eq!(relative, ["a", "b", "m", "z"].map(PathBuf::from));
            catalogs.push(relative);
        }
        assert_eq!(catalogs[0], catalogs[1]);
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_unicode_names_and_skips_fifo() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let f = Fixture::new();
        let mut expected = Vec::new();
        for byte in [0xff, 0xfe] {
            let path = f.0.join(OsString::from_vec(vec![b'x', byte]));
            fs::write(&path, b"fixture").unwrap();
            expected.push(path);
        }
        expected.sort();
        // Unix fixture creation uses the platform mkfifo utility; discovery
        // itself uses only Rust's standard library.
        let status = std::process::Command::new("mkfifo")
            .arg(f.0.join("pipe"))
            .status()
            .unwrap();
        assert!(status.success());
        let catalog = discover(&f.0).unwrap();
        assert_eq!(catalog.files, expected);
        assert_eq!(catalog.skipped_special, 1);
        assert_eq!(catalog.skipped_symlinks, 0);
        assert!(catalog.errors.is_empty());
    }
}
