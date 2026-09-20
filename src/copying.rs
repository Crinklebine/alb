//! Copy and verify bytes using handles secured by the platform backend.
#[cfg(test)]
use crate::paths;
use crate::{hashing, source::SourceStamp};
#[cfg(test)]
use std::{fs, fs::OpenOptions, path::Path};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

/// Copy using already-secured handles; verify disk-read contents against the plan.
pub fn transfer_verified(
    source: &mut File,
    partial: &mut File,
    stamp: &SourceStamp,
    digest: [u8; 32],
) -> io::Result<()> {
    if SourceStamp::from_file(source)? != *stamp {
        return Err(io::Error::other("source changed before copy"));
    }
    let bytes = io::copy(
        &mut (&mut *source).take(stamp.len.saturating_add(1)),
        partial,
    )?;
    if bytes != stamp.len || SourceStamp::from_file(source)? != *stamp {
        return Err(io::Error::other("source changed during copy"));
    }
    partial.sync_all()?;
    partial.seek(SeekFrom::Start(0))?;
    let (actual, bytes) = hashing::hash_reader(partial)?;
    if actual != digest || bytes != stamp.len {
        return Err(io::Error::other("partial verification failed"));
    }
    if SourceStamp::from_file(source)? != *stamp {
        return Err(io::Error::other("source changed during verification"));
    }
    crate::platform::set_times(partial, stamp.modified, stamp.created)?;
    Ok(())
}

#[cfg(test)]
pub struct StageRequest<'a> {
    pub input_root: &'a Path,
    pub output_root: &'a Path,
    pub source: &'a Path,
    pub partial: &'a Path,
    pub source_stamp: &'a SourceStamp,
    pub digest: [u8; 32],
}

/// Create a NEW partial file in an already-existing output directory. On any
/// failure, retain it as incomplete; never reuse, truncate, publish or delete it.
/// Success means verified staging only, not a completed copy or omission authority.
#[cfg(test)]
pub fn stage(request: &StageRequest<'_>) -> io::Result<()> {
    stage_with(request, |source, partial| {
        io::copy(
            &mut source.take(request.source_stamp.len.saturating_add(1)),
            partial,
        )
    })
}

#[cfg(test)]
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
fn source_matches(request: &StageRequest<'_>, file: &File) -> io::Result<()> {
    if SourceStamp::at(request.source)? != *request.source_stamp
        || SourceStamp::from_file(file)? != *request.source_stamp
    {
        return Err(io::Error::other("source changed; rescan required"));
    }
    Ok(())
}

#[cfg(test)]
fn stage_with(
    request: &StageRequest<'_>,
    transfer: impl FnOnce(&mut File, &mut File) -> io::Result<u64>,
) -> io::Result<()> {
    let roots = paths::validate(request.input_root, request.output_root)
        .map_err(|e| invalid(&e.to_string()))?;
    if !request.source.starts_with(&roots.input) || !request.partial.starts_with(&roots.output) {
        return Err(invalid("source or partial is outside its permitted root"));
    }
    if !request
        .partial
        .file_name()
        .is_some_and(|name| name.as_encoded_bytes().ends_with(b".alb-partial"))
    {
        return Err(invalid("staging path must end in .alb-partial"));
    }
    // Snapshot checks only. A future executor must anchor output operations to
    // directory handles before this primitive can be used on changing trees.
    for path in [request.source, request.partial] {
        let parent = path
            .parent()
            .ok_or_else(|| invalid("missing parent directory"))?;
        if fs::canonicalize(parent)? != parent {
            return Err(invalid("noncanonical or symlinked parent is not allowed"));
        }
    }
    if SourceStamp::at(request.source)? != *request.source_stamp {
        return Err(io::Error::other(
            "source changed before staging; rescan required",
        ));
    }
    let mut source = File::open(request.source)?;
    source_matches(request, &source)?;
    // create_new atomically refuses existing files, hardlinks and symlinks at
    // this final path component. Never open an existing partial for writing.
    let mut partial = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(request.partial)?;
    let bytes = transfer(&mut source, &mut partial)?;
    source_matches(request, &source)?;
    if bytes != request.source_stamp.len || partial.metadata()?.len() != request.source_stamp.len {
        return Err(io::Error::other(
            "partial length does not match planned source",
        ));
    }
    partial.sync_all()?;
    partial.seek(SeekFrom::Start(0))?;
    let (digest, verified_bytes) = hashing::hash_reader(&mut partial)?;
    if digest != request.digest || verified_bytes != request.source_stamp.len {
        return Err(io::Error::other("partial verification failed"));
    }
    source_matches(request, &source)?;
    if SourceStamp::at(request.partial)? != SourceStamp::from_file(&partial)? {
        return Err(io::Error::other("partial path changed during staging"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, path::PathBuf};
    struct Fixture {
        root: PathBuf,
        input: PathBuf,
        output: PathBuf,
        source: PathBuf,
        partial: PathBuf,
        stamp: SourceStamp,
        digest: [u8; 32],
    }
    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            let root = (0..)
                .find_map(|n| {
                    let p =
                        std::env::temp_dir().join(format!("alb-stage-{}-{n}", std::process::id()));
                    match fs::create_dir(&p) {
                        Ok(()) => Some(fs::canonicalize(p).unwrap()),
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
                        Err(e) => panic!("{e}"),
                    }
                })
                .unwrap();
            let input = root.join("input");
            let output = root.join("output");
            fs::create_dir(&input).unwrap();
            fs::create_dir(&output).unwrap();
            let source = input.join("track.bin");
            fs::write(&source, bytes).unwrap();
            let stamp = SourceStamp::at(&source).unwrap();
            let partial = output.join("track.bin.alb-partial");
            Self {
                root,
                input,
                output,
                source,
                partial,
                stamp,
                digest: *blake3::hash(bytes).as_bytes(),
            }
        }
        fn request(&self) -> StageRequest<'_> {
            StageRequest {
                input_root: &self.input,
                output_root: &self.output,
                source: &self.source,
                partial: &self.partial,
                source_stamp: &self.stamp,
                digest: self.digest,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn stages_verified_empty_and_multibuffer_files_without_publishing_or_changing_source() {
        for bytes in [vec![], vec![0x5a; 200_000]] {
            let f = Fixture::new(&bytes);
            stage(&f.request()).unwrap();
            assert_eq!(fs::read(&f.partial).unwrap(), bytes);
            assert_eq!(fs::read(&f.source).unwrap(), bytes);
            assert_eq!(SourceStamp::at(&f.source).unwrap(), f.stamp);
            assert_eq!(fs::read_dir(&f.input).unwrap().count(), 1);
            assert!(!f.output.join("track.bin").exists());
            assert_eq!(
                stage(&f.request()).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
        }
    }
    #[test]
    fn rejects_stale_sources_before_creating_output() {
        let f = Fixture::new(b"original");
        fs::write(&f.source, b"changed fixture").unwrap();
        assert!(
            stage(&f.request())
                .unwrap_err()
                .to_string()
                .contains("source changed")
        );
        assert!(!f.partial.exists());
    }
    #[test]
    fn failures_leave_distinct_incomplete_files_and_never_resume_them() {
        let f = Fixture::new(b"original");
        let error = stage_with(&f.request(), |_, partial| {
            partial.write_all(b"part")?;
            Err(io::Error::other("injected write failure"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected write failure"));
        assert_eq!(fs::read(&f.partial).unwrap(), b"part");
        assert_eq!(
            stage(&f.request()).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&f.source).unwrap(), b"original");
    }
    #[test]
    fn rejects_bad_digests_and_changes_during_copy() {
        let f = Fixture::new(b"original");
        let mut request = f.request();
        request.digest = [0; 32];
        assert!(
            stage(&request)
                .unwrap_err()
                .to_string()
                .contains("verification failed")
        );
        assert!(f.partial.exists());
        let g = Fixture::new(b"original");
        let error = stage_with(&g.request(), |source, partial| {
            let bytes = io::copy(source, partial)?;
            fs::write(&g.source, b"changed synthetic source")?;
            Ok(bytes)
        })
        .unwrap_err();
        assert!(error.to_string().contains("source changed"));
    }
    #[test]
    fn rereads_partial_instead_of_trusting_transfer_count() {
        let f = Fixture::new(b"original");
        let error = stage_with(&f.request(), |_, partial| {
            partial.write_all(b"tampered")?;
            Ok(8)
        })
        .unwrap_err();
        assert!(error.to_string().contains("verification failed"));
        assert_eq!(fs::read(&f.source).unwrap(), b"original");
    }
    #[test]
    fn rejects_overlap_escape_and_final_destination_names() {
        let f = Fixture::new(b"original");
        let mut request = f.request();
        request.output_root = &f.input;
        assert!(stage(&request).is_err());
        let escaped = f.input.join("new.alb-partial");
        request = f.request();
        request.partial = &escaped;
        assert!(stage(&request).is_err());
        assert!(!escaped.exists());
        let final_name = f.output.join("track.bin");
        request = f.request();
        request.partial = &final_name;
        assert!(stage(&request).is_err());
        assert!(!final_name.exists());
    }
    #[cfg(unix)]
    #[test]
    fn never_follows_partial_or_parent_symlinks() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new(b"original");
        symlink(&f.source, &f.partial).unwrap();
        assert_eq!(
            stage(&f.request()).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&f.source).unwrap(), b"original");
        let link = f.output.join("link");
        symlink(&f.input, &link).unwrap();
        let partial = link.join("escaped.alb-partial");
        let mut request = f.request();
        request.partial = &partial;
        assert!(stage(&request).is_err());
        assert!(!f.input.join("escaped.alb-partial").exists());
    }
}
