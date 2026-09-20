//! Exact whole-file hashes. No audio equivalence or automatic omission.
use crate::{candidates::FileType, catalog::Track, source::SourceStamp};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::PathBuf,
};

#[derive(Debug)]
pub struct DuplicateGroup {
    pub file_type: FileType,
    pub digest: [u8; 32],
    /// Sorted source paths; first is the deterministic preferred representative.
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct HashedFile {
    pub file_type: FileType,
    pub digest: [u8; 32],
    pub source_stamp: SourceStamp,
}

#[derive(Debug, Default)]
pub struct HashCatalog {
    pub hashed: usize,
    pub files: BTreeMap<PathBuf, HashedFile>,
    pub groups: Vec<DuplicateGroup>,
    pub errors: Vec<(PathBuf, io::Error)>,
}

pub fn analyze(tracks: &[Track]) -> HashCatalog {
    let mut catalog = HashCatalog::default();
    let mut groups: BTreeMap<(FileType, [u8; 32]), Vec<PathBuf>> = BTreeMap::new();
    let progress = crate::progress::Progress::new("Hashing source files", Some(tracks.len()));
    for (index, track) in tracks.iter().enumerate() {
        progress.set(index);
        match hash_track(track) {
            Ok(digest) => {
                catalog.hashed += 1;
                // hash_track succeeds only with a matching inspection stamp.
                if let Some(stamp) = &track.source_stamp {
                    catalog.files.insert(
                        track.source_path.clone(),
                        HashedFile {
                            file_type: track.file_type,
                            digest,
                            source_stamp: stamp.clone(),
                        },
                    );
                }
                groups
                    .entry((track.file_type, digest))
                    .or_default()
                    .push(track.source_path.clone());
            }
            Err(error) => catalog.errors.push((track.source_path.clone(), error)),
        }
    }
    for ((file_type, digest), mut paths) in groups {
        paths.sort();
        paths.dedup();
        if paths.len() > 1 {
            catalog.groups.push(DuplicateGroup {
                file_type,
                digest,
                paths,
            });
        }
    }
    catalog.errors.sort_by(|a, b| a.0.cmp(&b.0));
    progress.set(tracks.len());
    catalog
}

fn changed() -> io::Error {
    io::Error::other("source changed since inspection or during hashing; rescan required")
}

fn hash_track(track: &Track) -> io::Result<[u8; 32]> {
    hash_track_with(track, hash_reader)
}

// Allows deterministic read failures and mid-read changes in tests.
fn hash_track_with(
    track: &Track,
    read: impl FnOnce(&mut fs::File) -> io::Result<([u8; 32], u64)>,
) -> io::Result<[u8; 32]> {
    let expected = track
        .source_stamp
        .as_ref()
        .ok_or_else(|| io::Error::other("source has no inspection stamp"))?;
    if &SourceStamp::at(&track.source_path)? != expected {
        return Err(changed());
    }
    let mut file = crate::platform::open_snapshot(&track.source_path)?;
    if &SourceStamp::from_file(&file)? != expected {
        return Err(changed());
    }
    let (digest, bytes) = read(&mut file)?;
    if bytes != expected.len
        || &SourceStamp::from_file(&file)? != expected
        || &SourceStamp::at(&track.source_path)? != expected
    {
        return Err(changed());
    }
    Ok(digest)
}

pub(crate) fn hash_reader(reader: &mut impl Read) -> io::Result<([u8; 32], u64)> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("file size overflow"))?;
        hasher.update(&buffer[..read]);
    }
    Ok((*hasher.finalize().as_bytes(), total))
}

pub fn write_groups(writer: &mut impl io::Write, catalog: &HashCatalog) -> io::Result<()> {
    for (index, group) in catalog.groups.iter().enumerate() {
        writeln!(
            writer,
            "Exact duplicate group {}: {}, BLAKE3 {}",
            index + 1,
            group.file_type.group(),
            blake3::Hash::from(group.digest).to_hex()
        )?;
        for (index, path) in group.paths.iter().enumerate() {
            let role = if index == 0 { "PREFERRED" } else { "MATCH" };
            writeln!(writer, "  {role}: {path:?}")?;
        }
        writeln!(
            writer,
            "  Reason: identical whole-file hash and extension-defined file type; native source-path order breaks ties. No files omitted."
        )?;
    }
    for (path, error) in &catalog.errors {
        writeln!(writer, "{path:?}: hash error: {:?}", error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_known_empty_vector_and_streaming_digest() {
        let (digest, count) = hash_reader(&mut io::Cursor::new([])).unwrap();
        assert_eq!(
            blake3::Hash::from(digest).to_hex().as_str(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(count, 0);
        let data = vec![42u8; 200_000];
        let (digest, count) = hash_reader(&mut io::Cursor::new(&data)).unwrap();
        assert_eq!(digest, *blake3::hash(&data).as_bytes());
        assert_eq!(count, data.len() as u64);
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            for attempt in 0.. {
                let path = std::env::temp_dir()
                    .join(format!("alb-hashing-{}-{attempt}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(fs::canonicalize(path).unwrap()),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create fixture: {error}"),
                }
            }
            unreachable!()
        }
        fn track(&self, name: &str) -> Track {
            let path = self.0.join(name);
            fs::write(&path, include_bytes!("../tests/fixtures/tone.flac")).unwrap();
            let mut catalog = crate::inspection::inspect(&[path]);
            catalog.tracks.remove(0)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn retries_interrupted_reads_but_never_returns_partial_hash_on_failure() {
        struct Reader {
            cursor: io::Cursor<Vec<u8>>,
            interrupted: bool,
            fail: bool,
        }
        impl Read for Reader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                if self.fail && self.cursor.position() > 0 {
                    return Err(io::Error::other("injected disk failure"));
                }
                self.cursor.read(buf)
            }
        }
        for fail in [false, true] {
            let mut reader = Reader {
                cursor: io::Cursor::new(b"abc".to_vec()),
                interrupted: false,
                fail,
            };
            let result = hash_reader(&mut reader);
            if fail {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("injected disk failure")
                );
            } else {
                let (digest, bytes) = result.unwrap();
                assert_eq!(digest, *blake3::hash(b"abc").as_bytes());
                assert_eq!(bytes, 3);
            }
        }
    }

    #[test]
    fn excludes_changed_and_missing_sources_but_continues_grouping() {
        let f = Fixture::new();
        let changed_track = f.track("changed.flac");
        let missing = f.track("missing.flac");
        let a = f.track("a.flac");
        let z = f.track("z.flac");
        fs::write(&changed_track.source_path, b"changed length").unwrap();
        fs::remove_file(&missing.source_path).unwrap();
        let catalog = analyze(&[changed_track, z, missing, a]);
        assert_eq!(catalog.hashed, 2);
        assert_eq!(catalog.groups.len(), 1);
        assert_eq!(
            catalog.groups[0].paths,
            vec![f.0.join("a.flac"), f.0.join("z.flac")]
        );
        assert_eq!(catalog.errors.len(), 2);
        assert_eq!(catalog.errors[0].0, f.0.join("changed.flac"));
        assert!(catalog.errors[0].1.to_string().contains("source changed"));
        assert_eq!(catalog.errors[1].1.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn detects_changes_during_read_and_propagates_read_errors() {
        let f = Fixture::new();
        let track = f.track("track.flac");
        let error =
            hash_track_with(&track, |_| Err(io::Error::other("injected read error"))).unwrap_err();
        assert!(error.to_string().contains("injected read error"));
        let result = hash_track_with(&track, |file| {
            let result = hash_reader(file)?;
            // Mutation belongs to the test fixture only.
            fs::write(&track.source_path, b"changed during hash")?;
            Ok(result)
        });
        assert!(result.unwrap_err().to_string().contains("source changed"));
    }

    #[test]
    fn rejects_missing_stamp_and_duplicate_paths_do_not_form_groups() {
        let f = Fixture::new();
        let mut track = f.track("track.flac");
        track.source_stamp = None;
        assert!(
            hash_track(&track)
                .unwrap_err()
                .to_string()
                .contains("no inspection stamp")
        );
        let path = track.source_path;
        let catalog = crate::inspection::inspect(&[path.clone(), path]);
        assert!(analyze(&catalog.tracks).groups.is_empty());
    }

    #[test]
    fn exact_hash_groups_are_extension_scoped_for_every_type() {
        let f = Fixture::new();
        let mut tracks = Vec::new();
        for extension in ["flac", "m4a", "mp3", "ogg", "wav", "txt"] {
            tracks.push(f.track(&format!("a.{extension}")));
            tracks.push(f.track(&format!("b.{extension}")));
        }
        let catalog = analyze(&tracks);
        assert_eq!(catalog.hashed, 12);
        assert!(catalog.errors.is_empty());
        assert_eq!(catalog.groups.len(), 6);
        assert!(catalog.groups.iter().all(|g| g.paths.len() == 2));
        // All bytes are identical, but no group crosses an extension-defined type.
        assert!(
            catalog
                .groups
                .windows(2)
                .all(|pair| pair[0].digest == pair[1].digest)
        );
        assert!(catalog.groups.iter().all(|g| {
            g.paths
                .iter()
                .all(|p| crate::candidates::classify(p) == g.file_type)
        }));
    }

    #[test]
    fn grouping_and_report_do_not_depend_on_input_order() {
        let f = Fixture::new();
        let mut tracks = vec![f.track("z.flac"), f.track("a.flac"), f.track("unique.flac")];
        let unique_path = tracks[2].source_path.clone();
        let mut bytes = fs::read(&unique_path).unwrap();
        let title = bytes
            .windows(12)
            .position(|bytes| bytes == b"Fixture Tone")
            .unwrap();
        bytes[title] = b'M';
        fs::write(&unique_path, bytes).unwrap();
        tracks[2] = crate::inspection::inspect(&[unique_path]).tracks.remove(0);
        let first = analyze(&tracks);
        tracks.reverse();
        let second = analyze(&tracks);
        let mut first_report = Vec::new();
        let mut second_report = Vec::new();
        write_groups(&mut first_report, &first).unwrap();
        write_groups(&mut second_report, &second).unwrap();
        assert_eq!(first_report, second_report);
        assert_eq!(first.groups.len(), 1);
        assert_eq!(first.groups[0].paths.len(), 2);
        assert!(first.errors.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_replacement_and_symlink_even_when_size_and_mtime_match() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let track = f.track("track.flac");
        let replacement = f.track("replacement.flac");
        let original_mtime = fs::metadata(&track.source_path)
            .unwrap()
            .modified()
            .unwrap();
        fs::File::options()
            .write(true)
            .open(&replacement.source_path)
            .unwrap()
            .set_modified(original_mtime)
            .unwrap();
        fs::rename(&replacement.source_path, &track.source_path).unwrap();
        assert!(
            hash_track(&track)
                .unwrap_err()
                .to_string()
                .contains("source changed")
        );
        fs::remove_file(&track.source_path).unwrap();
        symlink("missing", &track.source_path).unwrap();
        assert_eq!(
            hash_track(&track).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
