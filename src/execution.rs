//! Execute only complete plans. Every duplicate depends on a verified publication.
use crate::{
    copying, hashing, paths,
    plan::{Action, BuildPlan, PlanEntry},
    safe_fs::Directory,
    source::SourceStamp,
};
#[cfg(test)]
use std::collections::BTreeSet;
use std::{ffi::OsString, fs::File, io, path::Path};

#[derive(Debug, Default)]
pub struct BuildResult {
    pub copied: usize,
    pub reused: usize,
    pub duplicates: usize,
    pub problems: usize,
    pub failed: usize,
}
fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text)
}
fn stamp(entry: &PlanEntry) -> io::Result<&SourceStamp> {
    entry
        .source_stamp
        .as_ref()
        .ok_or_else(|| invalid("missing source stamp"))
}
fn digest(entry: &PlanEntry) -> io::Result<[u8; 32]> {
    entry.digest.ok_or_else(|| invalid("missing hash evidence"))
}
fn open_source(input: &Directory, root: &Path, entry: &PlanEntry) -> io::Result<File> {
    let relative = entry
        .source
        .strip_prefix(root)
        .map_err(|_| invalid("source outside input"))?;
    let (parent, name) = input.parent(relative, false)?;
    let file = parent.read(&name)?;
    if SourceStamp::from_file(&file)? != *stamp(entry)? {
        return Err(io::Error::other("source changed; rescan required"));
    }
    Ok(file)
}
fn verify_file(mut file: File, expected: [u8; 32], size: u64) -> io::Result<()> {
    let before = SourceStamp::from_file(&file)?;
    let (actual, bytes) = hashing::hash_reader(&mut file)?;
    if actual != expected || bytes != size || SourceStamp::from_file(&file)? != before {
        return Err(io::Error::other("published file verification failed"));
    }
    Ok(())
}
pub fn verify_existing(entry: &PlanEntry) -> io::Result<()> {
    let destination = entry
        .destination
        .as_ref()
        .ok_or_else(|| invalid("missing destination"))?;
    let parent = Directory::absolute(
        destination
            .parent()
            .ok_or_else(|| invalid("missing parent"))?,
        false,
        None,
    )?;
    let name = destination
        .file_name()
        .ok_or_else(|| invalid("missing filename"))?;
    parent.check_alias(name)?;
    let file = parent.read(name)?;
    crate::platform::verify_times(
        &file.metadata()?,
        stamp(entry)?.modified,
        stamp(entry)?.created,
    )?;
    verify_file(file, digest(entry)?, stamp(entry)?.len)
}
#[cfg(test)]
pub fn execute(
    plan: &BuildPlan,
    input_root: &Path,
    output_root: &Path,
    resume: bool,
) -> io::Result<BuildResult> {
    if !plan.output_checked
        || !plan.discovery_errors.is_empty()
        || plan.unresolved() != 0
        || plan
            .entries
            .iter()
            .any(|e| !matches!(e.action, Action::Keep | Action::DuplicateOf(_)))
    {
        return Err(invalid(
            "build plan is incomplete or blocked; no copying started",
        ));
    }
    let roots = paths::validate(input_root, output_root).map_err(|e| invalid(&e.to_string()))?;
    let input = Directory::absolute(&roots.input, false, None)?;
    // Recheck every source before even creating output directories.
    for entry in &plan.entries {
        open_source(&input, &roots.input, entry)?;
        digest(entry)?;
        let destination = entry
            .destination
            .as_ref()
            .ok_or_else(|| invalid("missing destination"))?;
        destination
            .strip_prefix(&roots.output)
            .map_err(|_| invalid("destination outside output"))?;
    }
    let output = Directory::absolute(&roots.output, true, Some(&input))?;
    output.lock()?;
    let mut report = crate::build_report::BuildReport::start(&output, plan)?;
    let mut result = BuildResult::default();
    let mut published = BTreeSet::new();
    for entry in plan.entries.iter().filter(|e| e.action == Action::Keep) {
        let destination = entry
            .destination
            .as_ref()
            .ok_or_else(|| invalid("missing destination"))?;
        let relative = destination
            .strip_prefix(&roots.output)
            .map_err(|_| invalid("destination outside output"))?;
        let (parent, name) = output.parent(relative, true)?;
        if resume {
            match parent.read(&name) {
                Ok(file) => {
                    parent.check_alias(&name)?;
                    verify_file(file, digest(entry)?, stamp(entry)?.len)?;
                    let source = open_source(&input, &roots.input, entry)?;
                    verify_file(source, digest(entry)?, stamp(entry)?.len)?;
                    open_source(&input, &roots.input, entry)?;
                    report.reused(entry)?;
                    published.insert(entry.source.clone());
                    result.reused += 1;
                    continue;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        parent.absent(&name)?;
        let mut source = open_source(&input, &roots.input, entry)?;
        let mut partial_name = OsString::from(&name);
        partial_name.push(".alb-partial");
        let mut attempt = 0;
        let mut partial = loop {
            match parent.create(&partial_name) {
                Ok(file) => break file,
                Err(e) if resume && e.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => {
                    attempt += 1;
                    partial_name = name.clone();
                    partial_name.push(format!(".{attempt}.alb-partial"));
                }
                Err(e) => return Err(e),
            }
        };
        copying::transfer_verified(&mut source, &mut partial, stamp(entry)?, digest(entry)?)?;
        open_source(&input, &roots.input, entry)?; // Also confirm the path still identifies this source.
        let partial_stamp = SourceStamp::from_file(&partial)?;
        if SourceStamp::from_file(&parent.read(&partial_name)?)? != partial_stamp {
            return Err(io::Error::other("partial path changed before publication"));
        }
        parent.publish(&partial_name, &name)?;
        verify_file(parent.read(&name)?, digest(entry)?, stamp(entry)?.len)?;
        report.verified(entry, false)?;
        published.insert(entry.source.clone());
        result.copied += 1;
    }
    for entry in &plan.entries {
        if let Action::DuplicateOf(representative) = &entry.action {
            if !published.contains(representative) {
                return Err(invalid("duplicate representative was not published"));
            }
            // Changed duplicate inputs must not be silently omitted.
            let source = open_source(&input, &roots.input, entry)?;
            verify_file(source, digest(entry)?, stamp(entry)?.len)?;
            let destination = entry
                .destination
                .as_ref()
                .ok_or_else(|| invalid("missing duplicate destination"))?;
            let (parent, name) = output.parent(
                destination
                    .strip_prefix(&roots.output)
                    .map_err(|_| invalid("destination outside output"))?,
                false,
            )?;
            verify_file(parent.read(&name)?, digest(entry)?, stamp(entry)?.len)?;
            open_source(&input, &roots.input, entry)?;
            report.verified(entry, true)?;
            result.duplicates += 1;
        }
    }
    report.complete(result.copied, result.duplicates, result.reused)?;
    Ok(result)
}

// Only quarantine copies choose an alternative name on a real destination collision.
fn copy_problem_aware(
    entry: &mut PlanEntry,
    input: &Directory,
    input_root: &Path,
    output: &Directory,
    output_root: &Path,
    resume: bool,
) -> io::Result<bool> {
    let original = entry.destination.clone();
    for attempt in 0..100 {
        if resume && entry.metadata_update.is_none() && !crate::problems::reasons(entry).is_empty()
        {
            let destination = entry
                .destination
                .as_ref()
                .ok_or_else(|| invalid("missing destination"))?;
            let relative = destination
                .strip_prefix(output_root)
                .map_err(|_| invalid("destination outside output"))?;
            let (parent, name) = output.parent(relative, true)?;
            match parent.read(&name) {
                Ok(mut file) => {
                    parent.check_alias(&name)?;
                    SourceStamp::from_file(&file)?;
                    let evidence = hashing::hash_reader(&mut file)?;
                    if evidence != (digest(entry)?, stamp(entry)?.len) {
                        entry.destination = original.as_ref().map(|path| {
                            crate::problems::collision_destination(path, &entry.source, attempt)
                        });
                        continue;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        let result = copy_one(entry, input, input_root, output, output_root, resume);
        match result {
            Err(ref error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    && !crate::problems::reasons(entry).is_empty() =>
            {
                entry.destination = original.as_ref().map(|path| {
                    crate::problems::collision_destination(path, &entry.source, attempt)
                });
            }
            _ => return result,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "problem filename collision limit reached",
    ))
}

fn copy_one(
    entry: &mut PlanEntry,
    input: &Directory,
    input_root: &Path,
    output: &Directory,
    output_root: &Path,
    resume: bool,
) -> io::Result<bool> {
    let destination = entry
        .destination
        .as_ref()
        .ok_or_else(|| invalid("missing destination"))?;
    let relative = destination
        .strip_prefix(output_root)
        .map_err(|_| invalid("destination outside output"))?;
    let (parent, name) = output.parent(relative, true)?;
    let mut reuse_tagged = false;
    if resume {
        match parent.read(&name) {
            Ok(file) => {
                parent.check_alias(&name)?;
                crate::platform::verify_times(
                    &file.metadata()?,
                    stamp(entry)?.modified,
                    stamp(entry)?.created,
                )?;
                if entry.metadata_update.is_some() {
                    reuse_tagged = true;
                } else {
                    verify_file(file, digest(entry)?, stamp(entry)?.len)?;
                    let source = open_source(input, input_root, entry)?;
                    verify_file(source, digest(entry)?, stamp(entry)?.len)?;
                    open_source(input, input_root, entry)?;
                    entry.output_evidence = Some((digest(entry)?, stamp(entry)?.len));
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    if !reuse_tagged {
        parent.absent(&name)?;
    }
    let mut source = open_source(input, input_root, entry)?;
    crate::space::ensure(
        crate::space::required(stamp(entry)?.len, 1)?,
        crate::space::available(&parent)?,
    )?;
    let mut partial_name = OsString::from(&name);
    partial_name.push(".alb-partial");
    let mut attempt = 0;
    let mut partial = loop {
        match parent.create(&partial_name) {
            Ok(file) => break file,
            Err(e) if resume && e.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => {
                attempt += 1;
                partial_name = name.clone();
                partial_name.push(format!(".{attempt}.alb-partial"));
            }
            Err(e) => return Err(e),
        }
    };
    copying::transfer_verified(&mut source, &mut partial, stamp(entry)?, digest(entry)?)?;
    if let Some(update) = &entry.metadata_update {
        crate::tagging::apply(&mut partial, update, entry.file_type)?;
        crate::platform::set_times(&partial, stamp(entry)?.modified, stamp(entry)?.created)?;
    }
    use std::io::Seek;
    partial.rewind()?;
    let final_evidence = if entry.metadata_update.is_some() {
        hashing::hash_reader(&mut partial)?
    } else {
        (digest(entry)?, stamp(entry)?.len)
    };
    open_source(input, input_root, entry)?; // Also confirm the path still identifies this source.
    let partial_stamp = SourceStamp::from_file(&partial)?;
    if SourceStamp::from_file(&parent.read(&partial_name)?)? != partial_stamp {
        return Err(io::Error::other("partial path changed before publication"));
    }
    if reuse_tagged {
        verify_file(parent.read(&name)?, final_evidence.0, final_evidence.1)?;
        parent.remove_partial(&partial_name, &partial)?;
        entry.output_evidence = Some(final_evidence);
        return Ok(true);
    }
    parent.publish(&partial_name, &name)?;
    verify_file(parent.read(&name)?, final_evidence.0, final_evidence.1)?;
    entry.output_evidence = Some(final_evidence);
    Ok(false)
}

/// Sidecars use exclusive writes, atomic publication, and verified identical reuse.
fn sidecar(output: &Directory, root: &Path, entry: &PlanEntry, outcome: &str) -> io::Result<()> {
    use std::io::{Read, Write};
    let destination = entry
        .destination
        .as_ref()
        .ok_or_else(|| invalid("missing problem destination"))?;
    let mut relative = destination
        .strip_prefix(root)
        .map_err(|_| invalid("problem outside output"))?
        .as_os_str()
        .to_os_string();
    relative.push(".txt");
    let (parent, name) = output.parent(Path::new(&relative), true)?;
    let text = crate::problems::description(entry, outcome);
    // Never overwrite a prior explanation, including one from a failed attempt.
    for attempt in 0..100 {
        let mut chosen = name.clone();
        if attempt > 0 {
            chosen.push(format!(".{attempt}.txt"));
        }
        match parent.read(&chosen) {
            Ok(mut file) => {
                SourceStamp::from_file(&file)?;
                let mut bytes = Vec::new();
                std::io::Read::by_ref(&mut file)
                    .take(text.len() as u64 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes == text.as_bytes() {
                    return Ok(());
                }
                continue;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let mut partial = chosen.clone();
        partial.push(".alb-partial");
        let mut file = match parent.create(&partial) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        parent.publish(&partial, &chosen)?;
        return Ok(());
    }
    Err(io::Error::other("cannot reserve problem explanation"))
}

pub fn execute_resilient(
    plan: &BuildPlan,
    input_root: &Path,
    output_root: &Path,
    resume: bool,
) -> io::Result<BuildResult> {
    if !plan.output_checked {
        return Err(invalid("output safety check missing"));
    }
    let requested = if plan.input_roots.is_empty() {
        vec![input_root.to_path_buf()]
    } else {
        plan.input_roots.clone()
    };
    let validated = paths::validate_inputs(&requested).map_err(|e| invalid(&e.to_string()))?;
    let mut sources = Vec::new();
    let mut resolved_output = None;
    for root in validated {
        let roots = paths::validate(&root, output_root).map_err(|e| invalid(&e.to_string()))?;
        let input = Directory::absolute(&roots.input, false, None)?;
        resolved_output = Some(roots.output.clone());
        sources.push((roots.input, input));
    }
    let roots = paths::ValidatedPaths {
        input: sources
            .first()
            .ok_or_else(|| invalid("missing input roots"))?
            .0
            .clone(),
        output: resolved_output.ok_or_else(|| invalid("missing output root"))?,
    };
    crate::space::check(plan, &roots.output, resume)?;
    let input_handles: Vec<_> = sources.iter().map(|(_, directory)| directory).collect();
    let output = Directory::absolute_inputs(&roots.output, true, &input_handles)?;
    output.lock()?;
    let mut report = crate::build_report::BuildReport::start(&output, plan)?;
    let mut result = BuildResult::default();
    let mut published = std::collections::BTreeMap::new();
    let progress =
        crate::progress::Progress::new("Copying and verifying", Some(plan.entries.len()));
    let ordered = plan
        .entries
        .iter()
        .filter(|e| !matches!(e.action, Action::DuplicateOf(_)))
        .chain(
            plan.entries
                .iter()
                .filter(|e| matches!(e.action, Action::DuplicateOf(_))),
        );
    for (index, original) in ordered.enumerate() {
        let (source_root, input) = sources
            .iter()
            .find(|(root, _)| original.source.starts_with(root))
            .ok_or_else(|| invalid("source outside supplied inputs"))?;
        let roots = paths::ValidatedPaths {
            input: source_root.clone(),
            output: roots.output.clone(),
        };
        progress.set(index);
        let mut entry = original.clone();
        let mut duplicate = false;
        let operation = (|| -> io::Result<bool> {
            if let Action::DuplicateOf(representative) = &entry.action {
                if !published.contains_key(representative) {
                    return Err(invalid("duplicate representative unavailable"));
                }
                let source = open_source(input, &roots.input, &entry)?;
                verify_file(source, digest(&entry)?, stamp(&entry)?.len)?;
                let destination = entry
                    .destination
                    .as_ref()
                    .ok_or_else(|| invalid("missing destination"))?;
                let (parent, name) = output.parent(
                    destination
                        .strip_prefix(&roots.output)
                        .map_err(|_| invalid("destination outside output"))?,
                    false,
                )?;
                let evidence: ([u8; 32], u64) = published[representative];
                verify_file(parent.read(&name)?, evidence.0, evidence.1)?;
                entry.output_evidence = Some(evidence);
                open_source(input, &roots.input, &entry)?;
                duplicate = true;
                return Ok(false);
            }
            copy_problem_aware(
                &mut entry,
                input,
                &roots.input,
                &output,
                &roots.output,
                resume,
            )
        })();
        let operation = match operation {
            Ok(reused) => Ok(reused),
            Err(error) => {
                duplicate = false;
                let tagging_failed = error.to_string().starts_with("metadata update failed:");
                if tagging_failed {
                    entry.metadata_update = None;
                }
                crate::problems::mark(
                    &mut entry,
                    &roots.input,
                    &roots.output,
                    if tagging_failed {
                        "Metadata Write Errors"
                    } else {
                        "Copy Errors"
                    },
                    vec![format!("Copy or verification failed: {error}")],
                );
                // A previous inspection/hash read may have failed transiently.
                // Reacquire evidence through secure handles, never trust partial bytes.
                if entry.digest.is_none() || entry.source_stamp.is_none() {
                    let refreshed = (|| -> io::Result<()> {
                        let relative = entry
                            .source
                            .strip_prefix(&roots.input)
                            .map_err(|_| invalid("source outside input"))?;
                        let (parent, name) = input.parent(relative, false)?;
                        let mut file = parent.read(&name)?;
                        let before = SourceStamp::from_file(&file)?;
                        let (hash, bytes) = hashing::hash_reader(&mut file)?;
                        if bytes != before.len || SourceStamp::from_file(&file)? != before {
                            return Err(io::Error::other("source changed while retrying read"));
                        }
                        entry.source_stamp = Some(before);
                        entry.digest = Some(hash);
                        open_source(input, &roots.input, &entry)?;
                        Ok(())
                    })();
                    if let Err(error) = refreshed {
                        entry
                            .notes
                            .push(format!("PROBLEM: Evidence retry failed: {error}"));
                    }
                }
                // Retry independently in quarantine; never overwrite the conflicting destination.
                copy_problem_aware(
                    &mut entry,
                    input,
                    &roots.input,
                    &output,
                    &roots.output,
                    resume,
                )
                .or_else(|error| {
                    // A first failure may have been a destination conflict; tagging
                    // can then fail during quarantine retry. Still preserve the bytes.
                    if entry.metadata_update.is_none()
                        || !error.to_string().starts_with("metadata update failed:")
                    {
                        return Err(error);
                    }
                    entry.metadata_update = None;
                    crate::problems::mark(
                        &mut entry,
                        &roots.input,
                        &roots.output,
                        "Metadata Write Errors",
                        vec![format!(
                            "{error}; original bytes retained without metadata changes"
                        )],
                    );
                    copy_problem_aware(
                        &mut entry,
                        input,
                        &roots.input,
                        &output,
                        &roots.output,
                        resume,
                    )
                })
            }
        };
        let is_problem = !crate::problems::reasons(&entry).is_empty();
        if is_problem {
            result.problems += 1;
        }
        match operation {
            Ok(reused) => {
                if is_problem
                    && let Err(error) = sidecar(
                        &output,
                        &roots.output,
                        &entry,
                        "Copied and verified; review the problems below.",
                    )
                {
                    result.failed += 1;
                    report.failure(
                        &entry,
                        &format!("file copied, but explanation could not be written: {error}"),
                    )?;
                    continue;
                }
                if duplicate {
                    result.duplicates += 1;
                    report.verified(&entry, true)?;
                } else if reused {
                    result.reused += 1;
                    report.reused(&entry)?;
                } else {
                    result.copied += 1;
                    report.verified(&entry, false)?;
                }
                if !is_problem {
                    published.insert(
                        entry.source.clone(),
                        entry
                            .output_evidence
                            .ok_or_else(|| invalid("missing output evidence"))?,
                    );
                }
            }
            Err(error) => {
                result.failed += 1;
                let outcome =
                    format!("NOT COPIED: {error}. The source remains at its original location.");
                let explanation = sidecar(&output, &roots.output, &entry, &outcome);
                report.failure(
                    &entry,
                    &format!("{outcome}; explanation result: {explanation:?}"),
                )?;
            }
        }
    }
    for error in &plan.discovery_errors {
        result.failed += 1;
        // Discovery errors may refer to inaccessible directories rather than files.
        let mut entry = PlanEntry {
            metadata_update: None,
            output_evidence: None,
            fingerprint_root: None,
            source: input_root.to_path_buf(),
            file_type: crate::candidates::FileType::Unknown,
            source_stamp: None,
            digest: None,
            action: Action::Keep,
            collision: false,
            destination: Some(crate::problems::destination(
                Path::new(error),
                &roots.output,
                "Discovery Errors",
            )),
            issues: vec![],
            sanitized: false,
            notes: vec![format!("PROBLEM: {error}")],
        };
        entry.notes.push(
            "PROBLEM: Some source paths could not be enumerated; no copy can be promised for them."
                .into(),
        );
        let explanation = sidecar(
            &output,
            &roots.output,
            &entry,
            "NOT COPIED: discovery failed.",
        );
        report.failure(
            &entry,
            &format!("discovery failed; explanation result: {explanation:?}"),
        )?;
    }
    progress.set(plan.entries.len());
    if result.failed == 0 {
        report.complete(result.copied, result.duplicates, result.reused)?;
    } else {
        report.incomplete(result.failed)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            for n in 0.. {
                let path =
                    std::env::temp_dir().join(format!("alb-execute-{}-{n}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => {
                        fs::create_dir(path.join("in")).unwrap();
                        return Self(fs::canonicalize(path).unwrap());
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("{e}"),
                }
            }
            unreachable!()
        }
        fn plan(&self) -> BuildPlan {
            let input = self.0.join("in");
            let files = crate::discovery::discover(&input).unwrap();
            let tracks = crate::inspection::inspect(&files.files);
            let hashes = crate::hashing::analyze(&tracks.tracks);
            let mut plan =
                crate::plan::generate(&files, &tracks, &input, &self.0.join("out"), Some(&hashes));
            crate::plan::check_existing_output(&mut plan);
            plan
        }
        fn run(&self, plan: &BuildPlan) -> io::Result<BuildResult> {
            execute(plan, &self.0.join("in"), &self.0.join("out"), false)
        }
        fn write(&self, name: &str, bytes: &[u8]) {
            fs::write(self.0.join("in").join(name), bytes).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn problem_names_are_preserved_and_existing_collisions_are_not_overwritten() {
        let f = Fixture::new();
        f.write("broken.mp3", b"unreadable metadata");
        let mut plan = f.plan();
        crate::problems::route(&mut plan, &f.0.join("in"), &f.0.join("out"));
        let destination = plan.entries[0].destination.clone().unwrap();
        assert_eq!(destination.file_name().unwrap(), "broken.mp3");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, b"existing unrelated file").unwrap();
        let result = execute_resilient(&plan, &f.0.join("in"), &f.0.join("out"), false).unwrap();
        assert_eq!(result.failed, 0);
        assert_eq!(result.copied, 1);
        assert_eq!(fs::read(&destination).unwrap(), b"existing unrelated file");
        let alternate =
            crate::problems::collision_destination(&destination, &plan.entries[0].source, 0);
        assert_eq!(fs::read(&alternate).unwrap(), b"unreadable metadata");
        let resumed = execute_resilient(&plan, &f.0.join("in"), &f.0.join("out"), true).unwrap();
        assert_eq!(resumed.failed, 0);
        assert_eq!(resumed.reused, 1);
        assert!(
            alternate
                .with_file_name(format!(
                    "{}.txt",
                    alternate.file_name().unwrap().to_str().unwrap()
                ))
                .exists()
        );
    }

    #[test]
    fn repaired_output_resolves_pending_parse_error_in_audit() {
        let f = Fixture::new();
        f.write("song.flac", include_bytes!("../tests/fixtures/tone.flac"));
        let files = crate::discovery::discover(&f.0.join("in")).unwrap();
        let mut tracks = crate::inspection::inspect(&files.files);
        let track = &mut tracks.tracks[0];
        track.fingerprinted = true;
        track.metadata_update = Some(crate::tagging::MetadataUpdate {
            artist: Some("Recovered Artist".into()),
            title: Some("Recovered Title".into()),
        });
        track.artist = Some("Recovered Artist".into());
        track.title = Some("Recovered Title".into());
        // Model a recovered inspection error; actual output must still pass strict parsing.
        tracks.errors.push((
            track.source_path.clone(),
            "metadata parse failed: recoverable tag error".into(),
        ));
        let hashes = crate::hashing::analyze(&tracks.tracks);
        let mut plan = crate::plan::generate(
            &files,
            &tracks,
            &f.0.join("in"),
            &f.0.join("out"),
            Some(&hashes),
        );
        crate::problems::route(&mut plan, &f.0.join("in"), &f.0.join("out"));
        crate::plan::check_existing_output_mode(&mut plan, false);
        assert!(crate::problems::reasons(&plan.entries[0]).is_empty());
        let result = execute_resilient(&plan, &f.0.join("in"), &f.0.join("out"), false).unwrap();
        assert_eq!(result.failed, 0);
        assert_eq!(result.problems, 0);
        assert!(!f.0.join("out/Problem Files").exists());
        let audit = fs::read_dir(f.0.join("out/_ALB"))
            .unwrap()
            .map(|p| fs::read_to_string(p.unwrap().path()).unwrap())
            .collect::<String>();
        assert!(audit.contains("Metadata warning:"));
    }

    #[test]
    fn insufficient_space_fails_before_output_creation() {
        let f = Fixture::new();
        f.write("a.txt", b"sample");
        let mut plan = f.plan();
        let directory = Directory::absolute(&f.0, false, None).unwrap();
        let available = crate::space::available(&directory).unwrap();
        plan.entries[0].source_stamp.as_mut().unwrap().len = available.saturating_add(1);
        let error = execute_resilient(&plan, &f.0.join("in"), &f.0.join("out"), false).unwrap_err();
        assert!(error.to_string().contains("insufficient output space"));
        assert!(!f.0.join("out").exists());
        assert_eq!(fs::read(f.0.join("in/a.txt")).unwrap(), b"sample");
    }

    #[test]
    fn failed_tagging_preserves_original_bytes_in_problem_folder() {
        let f = Fixture::new();
        f.write("broken.mp3", b"not decodable audio");
        let mut plan = f.plan();
        for entry in &mut plan.entries {
            entry.metadata_update = Some(crate::tagging::MetadataUpdate {
                artist: Some("Artist".into()),
                title: Some("Title".into()),
            });
        }
        crate::problems::route(&mut plan, &f.0.join("in"), &f.0.join("out"));
        let result = execute_resilient(&plan, &f.0.join("in"), &f.0.join("out"), false).unwrap();
        assert_eq!(result.failed, 0);
        assert_eq!(result.copied, 1);
        let copied = fs::read_dir(f.0.join("out/Problem Files/Metadata Write Errors"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "mp3"))
            .unwrap();
        assert_eq!(fs::read(copied).unwrap(), b"not decodable audio");
        assert_eq!(
            fs::read(f.0.join("in/broken.mp3")).unwrap(),
            b"not decodable audio"
        );
    }

    #[test]
    fn recovered_metadata_is_written_for_all_formats_and_resume_and_dedupe_work() {
        use lofty::{
            config::WriteOptions,
            file::TaggedFileExt,
            probe::Probe,
            tag::{Accessor, TagExt},
        };
        use std::io::Seek;
        let f = Fixture::new();
        for (extension, bytes) in [
            (
                "flac",
                include_bytes!("../tests/fixtures/tone.flac").as_slice(),
            ),
            (
                "m4a",
                include_bytes!("../tests/fixtures/tone.m4a").as_slice(),
            ),
            (
                "mp3",
                include_bytes!("../tests/fixtures/tone.mp3").as_slice(),
            ),
            (
                "ogg",
                include_bytes!("../tests/fixtures/tone.ogg").as_slice(),
            ),
            (
                "wav",
                include_bytes!("../tests/fixtures/tone.wav").as_slice(),
            ),
        ] {
            let path = f.0.join("in").join(format!("a.{extension}"));
            fs::write(&path, bytes).unwrap();
            let mut file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            let tagged = Probe::new(&mut file)
                .guess_file_type()
                .unwrap()
                .read()
                .unwrap();
            for tag in tagged.tags() {
                let mut tag = tag.clone();
                tag.remove_artist();
                tag.remove_title();
                file.rewind().unwrap();
                tag.save_to(&mut file, WriteOptions::new()).unwrap();
            }
            drop(file);
            fs::copy(&path, f.0.join("in").join(format!("b.{extension}"))).unwrap();
        }
        let sources: Vec<_> = fs::read_dir(f.0.join("in"))
            .unwrap()
            .map(|p| {
                let path = p.unwrap().path();
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        let files = crate::discovery::discover(&f.0.join("in")).unwrap();
        let mut tracks = crate::inspection::inspect(&files.files);
        for track in &mut tracks.tracks {
            assert!(track.artist.is_none() && track.title.is_none());
            track.artist = Some("Recovered Artist".into());
            track.title = Some("Recovered Title".into());
            track.fingerprinted = true;
            track.metadata_update = Some(crate::tagging::MetadataUpdate {
                artist: track.artist.clone(),
                title: track.title.clone(),
            });
        }
        let hashes = crate::hashing::analyze(&tracks.tracks);
        let mut plan = crate::plan::generate(
            &files,
            &tracks,
            &f.0.join("in"),
            &f.0.join("out"),
            Some(&hashes),
        );
        crate::plan::check_existing_output_mode(&mut plan, false);
        crate::problems::route(&mut plan, &f.0.join("in"), &f.0.join("out"));
        let result = execute_resilient(&plan, &f.0.join("in"), &f.0.join("out"), false).unwrap();
        let audit = fs::read_dir(f.0.join("out/_ALB"))
            .unwrap()
            .map(|p| fs::read_to_string(p.unwrap().path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(result.failed, 0, "{audit}");
        assert_eq!(result.copied, 5);
        assert_eq!(result.duplicates, 5);
        for entry in &plan.entries {
            let tagged = Probe::open(entry.destination.as_ref().unwrap())
                .unwrap()
                .read()
                .unwrap();
            let tag = if tagged.file_type() == lofty::file::FileType::Wav {
                tagged.tag(lofty::tag::TagType::RiffInfo).unwrap()
            } else {
                tagged.primary_tag().unwrap()
            };
            assert_eq!(tag.artist().as_deref(), Some("Recovered Artist"));
            assert_eq!(tag.title().as_deref(), Some("Recovered Title"));
            assert_eq!(tag.album().as_deref(), Some("Test Album"));
        }
        crate::plan::check_existing_output_mode(&mut plan, true);
        let result = execute_resilient(&plan, &f.0.join("in"), &f.0.join("out"), true).unwrap();
        assert_eq!(result.failed, 0);
        assert_eq!(
            result.reused,
            5,
            "{}",
            fs::read_dir(f.0.join("out/_ALB"))
                .unwrap()
                .map(|p| fs::read_to_string(p.unwrap().path()).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(result.duplicates, 5);
        for (path, bytes) in sources {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn vanished_source_gets_explanation_and_other_files_continue() {
        let f = Fixture::new();
        f.write("a.txt", b"gone");
        f.write("b.txt", b"keep");
        let mut plan = f.plan();
        crate::problems::route(&mut plan, &f.0.join("in"), &f.0.join("out"));
        fs::remove_file(f.0.join("in/a.txt")).unwrap();
        let result = execute_resilient(&plan, &f.0.join("in"), &f.0.join("out"), false).unwrap();
        assert_eq!(result.failed, 1);
        assert_eq!(result.copied, 1);
        assert_eq!(fs::read(f.0.join("out/UNKNOWN/b.txt")).unwrap(), b"keep");
        let reports: Vec<_> = fs::read_dir(f.0.join("out/Problem Files/Copy Errors"))
            .unwrap()
            .map(|p| fs::read_to_string(p.unwrap().path()).unwrap())
            .collect();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].contains("NOT COPIED"));
        assert!(reports[0].contains("a.txt"));
    }

    #[test]
    fn builds_all_formats_and_only_omits_verified_same_type_duplicates() {
        let f = Fixture::new();
        for (ext, bytes) in [
            (
                "flac",
                include_bytes!("../tests/fixtures/tone.flac").as_slice(),
            ),
            (
                "m4a",
                include_bytes!("../tests/fixtures/tone.m4a").as_slice(),
            ),
            (
                "mp3",
                include_bytes!("../tests/fixtures/tone.mp3").as_slice(),
            ),
            (
                "ogg",
                include_bytes!("../tests/fixtures/tone.ogg").as_slice(),
            ),
            (
                "wav",
                include_bytes!("../tests/fixtures/tone.wav").as_slice(),
            ),
        ] {
            f.write(&format!("tone.{ext}"), bytes);
        }
        f.write(
            "duplicate.flac",
            include_bytes!("../tests/fixtures/tone.flac"),
        );
        f.write("one.txt", b"same unknown");
        f.write("two.txt", b"same unknown");
        let plan = f.plan();
        let result = f.run(&plan).unwrap();
        assert_eq!((result.copied, result.duplicates), (7, 1));
        for e in &plan.entries {
            assert_eq!(
                SourceStamp::at(&e.source).unwrap(),
                *e.source_stamp.as_ref().unwrap()
            );
            assert_eq!(
                fs::read(&e.source).unwrap(),
                fs::read(e.destination.as_ref().unwrap()).unwrap()
            );
            let mut partial = e.destination.as_ref().unwrap().as_os_str().to_os_string();
            partial.push(".alb-partial");
            assert!(!Path::new(&partial).exists());
        }
        // Rerunning cannot overwrite even identical published bytes.
        assert!(f.run(&f.plan()).is_err());
    }

    #[test]
    fn stale_or_missing_evidence_fails_before_output_creation() {
        let f = Fixture::new();
        f.write("a.txt", b"before");
        let mut plan = f.plan();
        plan.entries[0].digest = None;
        assert!(f.run(&plan).is_err());
        assert!(!f.0.join("out").exists());
        let plan = f.plan();
        f.write("a.txt", b"changed");
        assert!(f.run(&plan).is_err());
        assert!(!f.0.join("out").exists());
    }

    #[test]
    fn occupied_finals_and_partials_are_never_overwritten() {
        for partial in [false, true] {
            let f = Fixture::new();
            f.write("a.txt", b"source");
            let plan = f.plan();
            let dest = plan.entries[0].destination.as_ref().unwrap();
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            let path = if partial {
                PathBuf::from(format!("{}.alb-partial", dest.display()))
            } else {
                dest.clone()
            };
            fs::write(&path, b"preserve").unwrap();
            assert!(f.run(&plan).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"preserve");
            if partial {
                assert!(!dest.exists());
            }
            assert_eq!(fs::read(f.0.join("in/a.txt")).unwrap(), b"source");
        }
    }

    #[test]
    fn bad_digest_retains_partial_without_publishing() {
        let f = Fixture::new();
        f.write("a.txt", b"source");
        let mut plan = f.plan();
        plan.entries[0].digest = Some([0; 32]);
        assert!(f.run(&plan).is_err());
        let dest = plan.entries[0].destination.as_ref().unwrap();
        assert!(!dest.exists());
        assert!(PathBuf::from(format!("{}.alb-partial", dest.display())).exists());
    }

    #[cfg(unix)]
    #[test]
    fn output_symlink_to_input_is_refused() {
        let f = Fixture::new();
        f.write("a.txt", b"source");
        let plan = f.plan();
        fs::create_dir(f.0.join("out")).unwrap();
        std::os::unix::fs::symlink(f.0.join("in"), f.0.join("out/UNKNOWN")).unwrap();
        assert!(f.run(&plan).is_err());
        assert_eq!(fs::read_dir(f.0.join("in")).unwrap().count(), 1);
        assert_eq!(fs::read(f.0.join("in/a.txt")).unwrap(), b"source");
    }

    #[cfg(unix)]
    #[test]
    fn pinned_parent_and_atomic_publication_resist_replacement() {
        let f = Fixture::new();
        let output = Directory::absolute(&f.0.join("out"), true, None).unwrap();
        let (parent, name) = output.parent(Path::new("folder/final"), true).unwrap();
        parent.absent(&name).unwrap();
        parent.create("partial".as_ref()).unwrap();
        // A competing destination appearing after the absence check must win.
        fs::write(f.0.join("out/folder/final"), b"competitor").unwrap();
        assert!(parent.publish("partial".as_ref(), &name).is_err());
        assert_eq!(
            fs::read(f.0.join("out/folder/final")).unwrap(),
            b"competitor"
        );
        assert!(f.0.join("out/folder/partial").exists());
        fs::rename(f.0.join("out/folder"), f.0.join("out/moved")).unwrap();
        std::os::unix::fs::symlink(f.0.join("in"), f.0.join("out/folder")).unwrap();
        parent.create("anchored".as_ref()).unwrap();
        assert!(f.0.join("out/moved/anchored").exists());
        assert!(!f.0.join("in/anchored").exists());
        assert!(output.parent(Path::new("folder/new"), true).is_err());
    }

    #[test]
    fn output_lock_excludes_another_build() {
        let f = Fixture::new();
        let first = Directory::absolute(&f.0.join("out"), true, None).unwrap();
        first.lock().unwrap();
        let second = Directory::absolute(&f.0.join("out"), false, None).unwrap();
        assert!(second.lock().is_err());
        drop(first);
        second.lock().unwrap();
    }
}
