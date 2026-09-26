use std::{fs, path::PathBuf, process::Command};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        for n in 0.. {
            let path =
                std::env::temp_dir().join(format!("alb-build-cli-{}-{n}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::create_dir(path.join("in")).unwrap();
                    return Self(fs::canonicalize(path).unwrap());
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("{e}"),
            }
        }
        unreachable!()
    }
    fn run(&self) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_alb"))
            .arg("build")
            .arg("--input")
            .arg(self.0.join("in"))
            .arg("--output")
            .arg(self.0.join("out"))
            .output()
            .unwrap()
    }
    fn report(&self) -> String {
        let paths: Vec<_> = fs::read_dir(self.0.join("out/_ALB")).unwrap().collect();
        assert_eq!(paths.len(), 1);
        fs::read_to_string(paths[0].as_ref().unwrap().path()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_builds_five_formats_and_persists_verified_audit() {
    let f = Fixture::new();
    for (ext, data) in [
        ("flac", include_bytes!("fixtures/tone.flac").as_slice()),
        ("m4a", include_bytes!("fixtures/tone.m4a").as_slice()),
        ("mp3", include_bytes!("fixtures/tone.mp3").as_slice()),
        ("ogg", include_bytes!("fixtures/tone.ogg").as_slice()),
        ("wav", include_bytes!("fixtures/tone.wav").as_slice()),
    ] {
        fs::write(f.0.join("in").join(format!("tone.{ext}")), data).unwrap();
    }
    fs::copy(f.0.join("in/tone.flac"), f.0.join("in/duplicate.flac")).unwrap();
    fs::write(f.0.join("in/notes.txt"), b"preserve unknown").unwrap();
    let before: Vec<_> = fs::read_dir(f.0.join("in"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let metadata = fs::metadata(&path).unwrap();
            (
                path.clone(),
                fs::read(&path).unwrap(),
                metadata.modified().unwrap(),
            )
        })
        .collect();
    let result = f.run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("6 verified copies, 1 exact duplicates")
    );
    assert!(result.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("SOURCE:"));
    let report = f.report();
    assert!(report.contains("SOURCE:"));
    assert!(report.contains("BLAKE3:"));
    assert!(report.contains("PROPOSED:"));
    assert_eq!(
        report
            .lines()
            .filter(|s| s.starts_with("VERIFIED_COPY "))
            .count(),
        6
    );
    assert_eq!(
        report
            .lines()
            .filter(|s| s.starts_with("VERIFIED_DUPLICATE "))
            .count(),
        1
    );
    assert!(report.ends_with("COMPLETE copied=6 duplicates=1 reused=0\n"));
    for (path, bytes, modified) in before {
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
    }
    for ext in ["flac", "m4a", "mp3", "ogg", "wav"] {
        let dest = f.0.join(format!(
            "out/{}/Test Ensemble/Test Album/02-03 - Fixture Tone - Test Artist.{ext}",
            ext.to_uppercase()
        ));
        assert_eq!(
            fs::read(dest).unwrap(),
            fs::read(f.0.join(format!("in/tone.{ext}"))).unwrap()
        );
    }
    assert_eq!(
        fs::read(f.0.join("out/UNKNOWN/notes.txt")).unwrap(),
        b"preserve unknown"
    );
    assert!(f.run().status.success());
    assert!(
        fs::read_dir(f.0.join("out/_ALB"))
            .unwrap()
            .any(|p| fs::read_to_string(p.unwrap().path()).unwrap() == report)
    );
    assert!(f.0.join("out/Problem Files/Destination Conflicts").is_dir());
}

#[test]
fn occupied_partial_is_preserved_and_file_is_quarantined() {
    let f = Fixture::new();
    fs::write(f.0.join("in/a.txt"), b"source").unwrap();
    fs::create_dir_all(f.0.join("out/UNKNOWN")).unwrap();
    let partial = f.0.join("out/UNKNOWN/a.txt.alb-partial");
    fs::write(&partial, b"interrupted").unwrap();
    assert!(f.run().status.success());
    let report = f.report();
    assert!(report.contains("VERIFIED_COPY"));
    let files: Vec<_> = fs::read_dir(f.0.join("out/Problem Files/Copy Errors"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|p| fs::read(p).unwrap() == b"source"));
    assert!(files.iter().any(|p| {
        fs::read_to_string(p)
            .unwrap()
            .contains("Copy or verification failed")
    }));
    assert_eq!(fs::read(partial).unwrap(), b"interrupted");
    assert!(!f.0.join("out/UNKNOWN/a.txt").exists());
}

#[cfg(unix)]
#[test]
fn report_symlink_cannot_redirect_writes_into_input() {
    let f = Fixture::new();
    fs::write(f.0.join("in/a.txt"), b"source").unwrap();
    fs::create_dir(f.0.join("out")).unwrap();
    std::os::unix::fs::symlink(f.0.join("in"), f.0.join("out/_ALB")).unwrap();
    assert!(!f.run().status.success());
    assert_eq!(fs::read_dir(f.0.join("in")).unwrap().count(), 1);
    assert!(!f.0.join("out/UNKNOWN").exists());
}

fn resume(f: &Fixture, dry_run: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_alb"));
    command
        .arg("build")
        .arg("--resume")
        .arg("--input")
        .arg(f.0.join("in"))
        .arg("--output")
        .arg(f.0.join("out"));
    if dry_run {
        command.arg("--dry-run");
    }
    command.output().unwrap()
}

#[test]
fn resume_reuses_verified_files_and_copies_missing_files_preserving_old_partials() {
    let f = Fixture::new();
    fs::write(f.0.join("in/a.txt"), b"first").unwrap();
    assert!(f.run().status.success());
    let original_report = f.report();
    let existing = f.0.join("out/UNKNOWN/a.txt");
    let before = fs::metadata(&existing).unwrap().modified().unwrap();
    fs::write(f.0.join("in/b.txt"), b"second").unwrap();
    let partial = f.0.join("out/UNKNOWN/b.txt.alb-partial");
    fs::write(&partial, b"incomplete").unwrap();
    assert!(resume(&f, true).status.success());
    assert_eq!(f.report(), original_report);
    assert!(!f.0.join("out/UNKNOWN/b.txt").exists());
    let result = resume(&f, false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("1 verified existing files reused"));
    assert_eq!(fs::metadata(existing).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read(partial).unwrap(), b"incomplete");
    assert_eq!(fs::read(f.0.join("out/UNKNOWN/b.txt")).unwrap(), b"second");
    let reports: Vec<_> = fs::read_dir(f.0.join("out/_ALB"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().any(|s| s == &original_report));
    assert!(reports.iter().any(|s| s.contains("VERIFIED_REUSE ")
        && s.ends_with("COMPLETE copied=1 duplicates=0 reused=1\n")));
}

#[cfg(unix)]
#[test]
fn resume_rejects_mismatched_bytes_and_symlinks_without_writes() {
    let f = Fixture::new();
    fs::write(f.0.join("in/a.txt"), b"first").unwrap();
    assert!(f.run().status.success());
    let report = f.report();
    let destination = f.0.join("out/UNKNOWN/a.txt");
    fs::write(&destination, b"wrong").unwrap(); // Same size, different digest.
    assert!(resume(&f, true).status.success());
    assert_eq!(f.report(), report); // Dry-run still makes no writes.
    assert!(resume(&f, false).status.success());
    assert_eq!(fs::read(&destination).unwrap(), b"wrong");
    fs::remove_file(&destination).unwrap();
    std::os::unix::fs::symlink(f.0.join("in/a.txt"), &destination).unwrap();
    assert!(resume(&f, false).status.success());
    assert!(fs::symlink_metadata(destination).unwrap().is_symlink());
    assert_eq!(fs::read(f.0.join("in/a.txt")).unwrap(), b"first");
}

#[test]
fn resume_duplicate_requires_verified_representative() {
    let f = Fixture::new();
    fs::write(f.0.join("in/a.flac"), include_bytes!("fixtures/tone.flac")).unwrap();
    fs::write(f.0.join("in/b.flac"), include_bytes!("fixtures/tone.flac")).unwrap();
    assert!(f.run().status.success());
    let result = resume(&f, false);
    assert!(result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("0 verified copies, 1 exact duplicates"));
    assert!(stderr.contains("1 verified existing files reused"));
}

#[test]
fn colliding_nonidentical_tracks_are_both_copied_and_resume_reuses_them() {
    let f = Fixture::new();
    let original = include_bytes!("fixtures/tone.flac");
    let mut changed = original.to_vec();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    fs::write(f.0.join("in/a.flac"), original).unwrap();
    fs::write(f.0.join("in/b.flac"), original).unwrap();
    fs::write(f.0.join("in/c.flac"), &changed).unwrap();
    let result = f.run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let directory = f.0.join("out/FLAC/Test Ensemble/Test Album");
    let files: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|p| fs::read(p.unwrap().path()).unwrap())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|bytes| bytes == original));
    assert!(files.contains(&changed));
    assert!(
        f.report()
            .contains("collision resolved; preserve all sources")
    );
    assert!(resume(&f, false).status.success());
    assert_eq!(fs::read(f.0.join("in/a.flac")).unwrap(), original);
    assert_eq!(fs::read(f.0.join("in/c.flac")).unwrap(), changed);
}

#[test]
fn malformed_and_missing_metadata_get_sidecars_while_good_files_build() {
    let f = Fixture::new();
    fs::write(
        f.0.join("in/good.flac"),
        include_bytes!("fixtures/tone.flac"),
    )
    .unwrap();
    fs::write(
        f.0.join("in/missing.wav"),
        include_bytes!("fixtures/untagged.wav"),
    )
    .unwrap();
    fs::write(f.0.join("in/broken.m4a"), b"not an audio container").unwrap();
    let result = f.run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        f.0.join("out/FLAC/Test Ensemble/Test Album/02-03 - Fixture Tone - Test Artist.flac")
            .exists()
    );
    for (class, original) in [
        ("Missing Metadata", "missing.wav"),
        ("Metadata Errors", "broken.m4a"),
    ] {
        let files: Vec<_> = fs::read_dir(f.0.join("out/Problem Files").join(class))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        assert_eq!(files.len(), 2);
        let copied = files
            .iter()
            .find(|p| p.extension().unwrap() != "txt")
            .unwrap();
        assert_eq!(
            fs::read(copied).unwrap(),
            fs::read(f.0.join("in").join(original)).unwrap()
        );
        let explanation = fs::read_to_string(
            files
                .iter()
                .find(|p| p.extension().unwrap() == "txt")
                .unwrap(),
        )
        .unwrap();
        assert!(explanation.contains(original));
        assert!(explanation.contains("Copied and verified"));
        assert!(explanation.contains("The source was not modified"));
    }
    assert!(resume(&f, false).status.success());
}

#[test]
fn per_file_output_failure_does_not_stop_other_files() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("in/Band/Album")).unwrap();
    fs::write(f.0.join("in/Band/Album/a.txt"), b"first").unwrap();
    fs::write(f.0.join("in/b.txt"), b"second").unwrap();
    fs::create_dir_all(f.0.join("out/UNKNOWN/Band/Album")).unwrap();
    fs::write(f.0.join("out/UNKNOWN/Band/Album/a.txt"), b"existing").unwrap();
    // Make only this problem class unavailable; source is never overwritten.
    fs::create_dir_all(f.0.join("out/Problem Files")).unwrap();
    fs::write(
        f.0.join("out/Problem Files/Destination Conflicts"),
        b"not a directory",
    )
    .unwrap();
    let result = f.run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(f.0.join("out/UNKNOWN/b.txt")).unwrap(), b"second");
    assert_eq!(
        fs::read(f.0.join("out/UNKNOWN/Band/Album/a.txt")).unwrap(),
        b"existing"
    );
    let folder = f.0.join("out/Problem Files/Copy Errors/Band/Album");
    let files: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|p| fs::read(p).unwrap() == b"first"));
    assert!(
        files
            .iter()
            .any(|p| fs::read_to_string(p).unwrap().contains("ALB problem file"))
    );
    assert_eq!(fs::read(f.0.join("in/Band/Album/a.txt")).unwrap(), b"first");
}

#[test]
fn unavailable_problem_storage_still_allows_unaffected_work_and_reports_failure() {
    let f = Fixture::new();
    fs::write(f.0.join("in/a.m4a"), b"broken").unwrap();
    fs::write(f.0.join("in/b.txt"), b"good").unwrap();
    fs::create_dir(f.0.join("out")).unwrap();
    fs::write(f.0.join("out/Problem Files"), b"occupied").unwrap();
    let result = f.run();
    assert!(!result.status.success());
    assert_eq!(fs::read(f.0.join("out/UNKNOWN/b.txt")).unwrap(), b"good");
    assert_eq!(
        fs::read(f.0.join("out/Problem Files")).unwrap(),
        b"occupied"
    );
    assert!(f.report().contains("FINISHED_WITH_ERRORS"));
    assert!(f.report().contains("NOT COPIED"));
    assert!(!f.report().lines().any(|s| s.starts_with("COMPLETE ")));
}

#[test]
fn archived_times_survive_normal_and_problem_copies() {
    use std::{
        fs::FileTimes,
        time::{Duration, SystemTime},
    };
    let f = Fixture::new();
    let time = SystemTime::UNIX_EPOCH + Duration::new(946684800, 123456700);
    for name in ["archive.txt", "broken.m4a"] {
        let path = f.0.join("in").join(name);
        fs::write(&path, b"archival bytes").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(time))
            .unwrap();
    }
    let original = fs::metadata(f.0.join("in/archive.txt")).unwrap();
    let birth = original.created().ok();
    let result = f.run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("Space check:"));
    assert_eq!(
        fs::metadata(f.0.join("out/UNKNOWN/archive.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        original.modified().unwrap()
    );
    let problems: Vec<_> = fs::read_dir(f.0.join("out/Problem Files/Metadata Errors"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect();
    let copied = problems
        .iter()
        .find(|p| p.extension().unwrap() == "m4a")
        .unwrap();
    assert_eq!(fs::metadata(copied).unwrap().modified().unwrap(), time);
    #[cfg(any(windows, target_os = "macos"))]
    {
        assert_eq!(
            fs::metadata(f.0.join("out/UNKNOWN/archive.txt"))
                .unwrap()
                .created()
                .ok(),
            birth
        );
        assert_eq!(
            fs::metadata(copied).unwrap().created().ok(),
            fs::metadata(f.0.join("in/broken.m4a"))
                .unwrap()
                .created()
                .ok()
        );
    }
    let report = f.report();
    assert!(report.contains("modified=\"2000-01-01 00:00:00.123456700 UTC\""));
    if let Some(birth) = birth {
        let date: chrono::DateTime<chrono::Utc> = birth.into();
        assert!(report.contains(&format!(
            "created=\"{}\"",
            date.format("%Y-%m-%d %H:%M:%S%.9f UTC")
        )));
    } else {
        assert!(report.contains("created=\"unavailable\""));
    }
    assert_eq!(
        fs::metadata(f.0.join("in/archive.txt"))
            .unwrap()
            .created()
            .ok(),
        birth
    );
    assert_eq!(
        fs::metadata(f.0.join("in/archive.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
    assert!(resume(&f, false).status.success());
}

#[cfg(windows)]
#[test]
fn windows_junctions_are_skipped_and_never_receive_output() {
    let f = Fixture::new();
    let external = f.0.join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("private.txt"), b"outside").unwrap();
    let source_link = f.0.join("in/junction");
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&source_link)
        .arg(&external)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    fs::write(f.0.join("in/a.txt"), b"source").unwrap();
    fs::create_dir(f.0.join("out")).unwrap();
    let output_link = f.0.join("out/UNKNOWN");
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&output_link)
        .arg(&external)
        .output()
        .unwrap();
    assert!(status.status.success());
    let result = f.run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read_dir(&external).unwrap().count(), 1);
    assert_eq!(fs::read(external.join("private.txt")).unwrap(), b"outside");
    assert!(String::from_utf8_lossy(&result.stderr).contains("1 symlinks skipped"));
    assert!(f.0.join("out/Problem Files/Destination Conflicts").exists());
    // Remove the junction entries themselves before fixture cleanup.
    fs::remove_dir(source_link).unwrap();
    fs::remove_dir(output_link).unwrap();
}

#[test]
fn missing_metadata_preserves_nested_source_folders_and_sidecars_on_resume() {
    let f = Fixture::new();
    for folder in ["Band/Album A", "Band/Album B"] {
        let source = f.0.join("in").join(folder);
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("song.wav"),
            include_bytes!("fixtures/untagged.wav"),
        )
        .unwrap();
    }
    let result = f.run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for (folder, class) in [
        ("Band/Album A", "Missing Metadata"),
        ("Band/Album B", "Duplicate Problems"),
    ] {
        let directory = f.0.join("out/Problem Files").join(class).join(folder);
        let files: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        assert_eq!(files.len(), 2);
        let copied = files
            .iter()
            .find(|p| p.extension().is_some_and(|e| e == "wav"))
            .unwrap();
        assert_eq!(
            fs::read(copied).unwrap(),
            include_bytes!("fixtures/untagged.wav")
        );
        assert!(
            files
                .iter()
                .any(|p| p.extension().is_some_and(|e| e == "txt"))
        );
        assert_eq!(
            fs::read(f.0.join("in").join(folder).join("song.wav")).unwrap(),
            include_bytes!("fixtures/untagged.wav")
        );
    }
    assert!(resume(&f, false).status.success());
}

#[test]
fn parentheses_and_long_generated_problem_names_keep_source_folders() {
    let f = Fixture::new();
    for (folder, extension, bytes) in [
        (
            "Clash",
            "wav",
            include_bytes!("fixtures/untagged.wav").as_slice(),
        ),
        (
            "Clash (Live)",
            "m4a",
            include_bytes!("fixtures/untagged.m4a").as_slice(),
        ),
    ] {
        let directory =
            f.0.join("in")
                .join(folder)
                .join("A fairly long original album folder name");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("{}.{}", "long song name ".repeat(6), extension)),
            bytes,
        )
        .unwrap();
    }
    let result = f.run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let root = f.0.join("out/Problem Files/Missing Metadata");
    assert!(
        fs::read_dir(&root)
            .unwrap()
            .all(|p| p.unwrap().path().is_dir())
    );
    for folder in ["Clash", "Clash (Live)"] {
        let directory = root
            .join(folder)
            .join("A fairly long original album folder name");
        let files: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        assert_eq!(files.len(), 2);
        assert!(
            files
                .iter()
                .any(|p| p.extension().is_some_and(|e| e == "txt"))
        );
    }
    assert!(resume(&f, false).status.success());
}
