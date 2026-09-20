use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        for attempt in 0.. {
            let root =
                std::env::temp_dir().join(format!("alb-paths-{}-{attempt}", std::process::id()));
            match fs::create_dir(&root) {
                Ok(()) => return Self(fs::canonicalize(root).unwrap()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create fixture: {error}"),
            }
        }
        unreachable!()
    }
    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn run(&self, input: &Path, output: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_alb"))
            .current_dir(&self.0)
            .arg("build")
            .arg("--dry-run")
            .arg("--input")
            .arg(input)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn diagnostic(output: Output) -> String {
    assert!(matches!(output.status.code(), Some(0 | 1)));
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn rejects_equal_and_nested_roots_without_creating_output() {
    let f = Fixture::new();
    let source = f.dir("source");
    let child = f.dir("source/child");
    for (input, output) in [
        (source.clone(), source.clone()),
        (source.clone(), child.clone()),
        (child, source.clone()),
        (source.clone(), source.join("absent/deep")),
    ] {
        let text = diagnostic(f.run(&input, &output));
        assert!(text.contains("unsafe paths"), "{text}");
        assert!(!text.contains("Root paths validated"));
    }
    assert!(!source.join("absent").exists());
}

#[test]
fn accepts_separate_existing_and_missing_roots_and_component_prefixes() {
    let f = Fixture::new();
    let source = f.dir("music");
    let existing = f.dir("music-copy");
    for output in [existing, f.0.join("new/deep/library")] {
        let text = diagnostic(f.run(&source, &output));
        assert!(text.contains("Root paths validated"), "{text}");
        assert!(text.contains("Dry-run"));
    }
    assert!(!f.0.join("new").exists());
}

#[test]
fn resolves_relative_paths_and_existing_parent_traversal() {
    let f = Fixture::new();
    f.dir("source/child");
    let text = diagnostic(f.run(Path::new("source"), Path::new("source/child/..")));
    assert!(text.contains("unsafe paths"), "{text}");
    let text = diagnostic(f.run(Path::new("./source"), Path::new("source/../new")));
    assert!(text.contains("Root paths validated"), "{text}");
    assert!(!f.0.join("new").exists());
}

#[test]
fn rejects_missing_input_files_and_ambiguous_missing_parents() {
    let f = Fixture::new();
    let source = f.dir("source");
    let file = f.0.join("file");
    fs::write(&file, b"fixture").unwrap();
    for (input, output) in [
        (f.0.join("missing"), f.0.join("out")),
        (file.clone(), f.0.join("out")),
        (source.clone(), file.clone()),
        (source.clone(), file.join("out")),
        // Keep the raw traversal: joining to a Windows verbatim root normalizes it.
        (source, PathBuf::from("missing/../out")),
    ] {
        let text = diagnostic(f.run(&input, &output));
        assert!(!text.contains("Root paths validated"), "{text}");
        assert!(
            !text.contains("library building is not implemented"),
            "{text}"
        );
    }
    assert!(!f.0.join("missing").exists());
    assert!(!f.0.join("out").exists());
}

#[cfg(unix)]
#[test]
fn resolves_symlink_aliases_and_rejects_dangling_links_and_loops() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let source = f.dir("source");
    let alias = f.0.join("alias");
    symlink(&source, &alias).unwrap();
    for (input, output) in [
        (source.clone(), alias.clone()),
        (source.clone(), alias.join("new/deep")),
        (alias.clone(), source.clone()),
        (source.clone(), alias.join("../source")),
    ] {
        let text = diagnostic(f.run(&input, &output));
        assert!(text.contains("unsafe paths"), "{text}");
    }
    let separate = f.dir("separate");
    symlink(&separate, f.0.join("safe-alias")).unwrap();
    assert!(
        diagnostic(f.run(&source, &f.0.join("safe-alias/new"))).contains("Root paths validated")
    );

    symlink(f.0.join("missing"), f.0.join("dangling")).unwrap();
    symlink("loop", f.0.join("loop")).unwrap();
    for output in ["dangling", "dangling/child", "loop", "loop/child"] {
        let text = diagnostic(f.run(&source, &f.0.join(output)));
        assert!(!text.contains("Root paths validated"), "{text}");
    }
    assert!(!source.join("new").exists());
    assert!(!separate.join("new").exists());
    assert!(!f.0.join("missing").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn validates_real_non_unicode_paths_without_lossy_comparison() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let f = Fixture::new();
    // Both names display with the same replacement character when converted
    // lossily; they must still be treated as distinct filesystem paths.
    let source = f.0.join(OsString::from_vec(b"music-\xff".to_vec()));
    let output = f.0.join(OsString::from_vec(b"music-\xfe".to_vec()));
    fs::create_dir(&source).unwrap();
    assert!(diagnostic(f.run(&source, &output)).contains("Root paths validated"));
    fs::create_dir(&output).unwrap();
    assert!(diagnostic(f.run(&source, &output)).contains("Root paths validated"));
    for destination in [
        source.clone(),
        source.join(OsString::from_vec(b"new-\xfd".to_vec())),
    ] {
        assert!(diagnostic(f.run(&source, &destination)).contains("unsafe paths"));
    }
    assert_eq!(fs::read_dir(&source).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn parent_traversal_uses_symlink_target_parent_not_lexical_parent() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let source = f.dir("source");
    let child = f.dir("source/child");
    f.dir("elsewhere");
    let alias = f.0.join("elsewhere/link");
    symlink(&child, &alias).unwrap();
    // Lexical normalization would incorrectly resolve this to elsewhere/new.
    for destination in [alias.join(".."), alias.join("../new/deep")] {
        assert!(diagnostic(f.run(&source, &destination)).contains("unsafe paths"));
    }
    // Resolve the same traversal when it occurs in input as well.
    assert!(diagnostic(f.run(&alias.join(".."), &source)).contains("unsafe paths"));

    let separate_child = f.dir("separate/child");
    let outward_alias = source.join("outward");
    symlink(&separate_child, &outward_alias).unwrap();
    // Lexical normalization would incorrectly classify this as inside source.
    let text = diagnostic(f.run(&source, &outward_alias.join("../new")));
    assert!(text.contains("Root paths validated"), "{text}");
    assert!(text.contains("Dry-run"));
    assert!(!source.join("new").exists());
    assert!(!f.0.join("separate/new").exists());
}

#[derive(Debug, PartialEq, Eq)]
struct EntrySnapshot {
    path: PathBuf,
    kind: fs::FileType,
    modified: std::time::SystemTime,
    readonly: bool,
    contents: Option<Vec<u8>>,
    link_target: Option<PathBuf>,
}

// Inspect synthetic fixtures without following symlinks. Access times are
// deliberately excluded: reading the fixture itself may update them.
fn snapshot(root: &Path) -> Vec<EntrySnapshot> {
    fn visit(root: &Path, path: &Path, entries: &mut Vec<EntrySnapshot>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        entries.push(EntrySnapshot {
            path: path.strip_prefix(root).unwrap().to_path_buf(),
            kind: metadata.file_type(),
            modified: metadata.modified().unwrap(),
            readonly: metadata.permissions().readonly(),
            contents: metadata.is_file().then(|| fs::read(path).unwrap()),
            link_target: metadata.is_symlink().then(|| fs::read_link(path).unwrap()),
        });
        if metadata.is_dir() {
            for child in fs::read_dir(path).unwrap() {
                visit(root, &child.unwrap().path(), entries);
            }
        }
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

#[test]
fn rejected_configurations_preserve_entire_source_tree() {
    let f = Fixture::new();
    let source = f.dir("source");
    let album = f.dir("source/artist/album");
    fs::write(album.join("track.flac"), b"synthetic audio bytes").unwrap();
    fs::write(
        source.join("notes.txt"),
        b"preserve metadata and other files",
    )
    .unwrap();
    f.dir("source/empty");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink("artist/album", source.join("album-link")).unwrap();
    }
    let before = snapshot(&source);
    for (input, output) in [
        (source.clone(), source.clone()),
        (source.clone(), album.clone()),
        (album.clone(), source.clone()),
        (source.clone(), source.join("_ALB/state")),
        (source.clone(), album.join("track.flac")),
        (source.clone(), source.join("absent/../output")),
    ] {
        let text = diagnostic(f.run(&input, &output));
        assert!(!text.contains("Root paths validated"), "{text}");
        assert!(
            !text.contains("library building is not implemented"),
            "{text}"
        );
        assert_eq!(
            snapshot(&source),
            before,
            "input={input:?}, output={output:?}"
        );
    }
}

#[test]
fn discovery_preserves_complete_source_snapshot_and_existing_output() {
    let f = Fixture::new();
    let source = f.dir("source");
    let album = f.dir("source/artist/album");
    f.dir("source/empty");
    fs::write(album.join("track.flac"), b"synthetic audio").unwrap();
    fs::write(source.join(".notes"), b"hidden non-audio").unwrap();
    fs::hard_link(album.join("track.flac"), source.join("hardlink.flac")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink("artist/album", source.join("album-link")).unwrap();
        symlink("missing", source.join("dangling")).unwrap();
    }
    let output = f.dir("output");
    fs::write(output.join("keep.txt"), b"existing output").unwrap();
    let before_source = snapshot(&source);
    let before_output = snapshot(&output);
    for _ in 0..2 {
        let text = diagnostic(f.run(&source, &output));
        assert!(
            text.contains("Discovery: 3 regular files, 4 directories"),
            "{text}"
        );
        assert!(text.contains("0 errors."), "{text}");
        assert!(
            text.contains("Classification: 2 supported files, 1 unknown files."),
            "{text}"
        );
        assert_eq!(snapshot(&source), before_source);
        assert_eq!(snapshot(&output), before_output);
    }
}

#[test]
fn flac_inspection_continues_after_failure_and_preserves_sources() {
    let f = Fixture::new();
    let source = f.dir("source");
    let valid = source.join("z-valid.FLAC");
    fs::write(source.join("a-broken.flac"), b"not a FLAC file").unwrap();
    fs::write(&valid, include_bytes!("fixtures/tone.flac")).unwrap();
    fs::write(source.join("unsupported.mp3"), b"not inspected yet").unwrap();
    let before = snapshot(&source);
    let text = diagnostic(f.run(&source, &f.0.join("output")));
    assert!(
        text.contains("Catalog: 3 files, 2 metadata/read errors."),
        "{text}"
    );
    assert_eq!(snapshot(&source), before);
    assert!(!f.0.join("output").exists());
}

#[test]
fn scan_reports_metadata_unknowns_and_errors_without_output_directory() {
    let f = Fixture::new();
    let source = f.dir("source");
    fs::write(
        source.join("a-good.flac"),
        include_bytes!("fixtures/tone.flac"),
    )
    .unwrap();
    fs::write(
        source.join("b-other.mp3"),
        include_bytes!("fixtures/tone.mp3"),
    )
    .unwrap();
    fs::write(source.join("c-notes.txt"), b"notes").unwrap();
    let before = snapshot(&source);
    let run = |verbose: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_alb"));
        command.arg("scan").arg("--input").arg(&source);
        if verbose {
            command.arg("--verbose");
        }
        command.output().unwrap()
    };
    let quiet = run(false);
    assert!(quiet.status.success());
    assert!(quiet.stdout.is_empty());
    let output = run(true);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("Fixture Tone"));
    assert!(lines[1].contains("MP3 metadata"));
    assert!(lines[2].contains("UNKNOWN"));
    assert_eq!(snapshot(&source), before);
    fs::write(source.join("d-bad.flac"), b"broken").unwrap();
    let before_error = snapshot(&source);
    let output = run(true);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("metadata/read error")
    );
    assert_eq!(snapshot(&source), before_error);
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
}

#[test]
fn scan_hash_reports_exact_flac_copies_without_omitting_files() {
    let f = Fixture::new();
    let source = f.dir("source");
    for name in ["z.flac", "a.flac", "same-bytes.mp3"] {
        fs::write(source.join(name), include_bytes!("fixtures/tone.flac")).unwrap();
    }
    let before = snapshot(&source);
    let output = Command::new(env!("CARGO_BIN_EXE_alb"))
        .arg("scan")
        .arg("--input")
        .arg(&source)
        .arg("--hash")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1)); // FLAC bytes named MP3: tag error, but hashing still works.
    let summary = String::from_utf8(output.stderr).unwrap();
    assert!(
        summary.contains("Exact file hashes: 3 files, 1 duplicate groups, 0 errors."),
        "{summary}"
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("PREFERRED:"));
    assert!(text.find("a.flac").unwrap() < text.find("z.flac").unwrap());
    assert!(!text.contains("same-bytes.mp3"));
    assert!(text.contains("No files omitted"));
    assert_eq!(snapshot(&source), before);
}

#[test]
fn dry_run_reports_exact_duplicates_without_writes() {
    let f = Fixture::new();
    let source = f.dir("source");
    for name in ["a.flac", "b.flac"] {
        fs::write(source.join(name), include_bytes!("fixtures/tone.flac")).unwrap();
    }
    fs::write(source.join("notes.txt"), b"notes").unwrap();
    let before = snapshot(&source);
    let output = Command::new(env!("CARGO_BIN_EXE_alb"))
        .arg("build")
        .arg("--input")
        .arg(&source)
        .arg("--output")
        .arg(f.0.join("out"))
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains("Plan summary: 3 entries"));
    assert!(text.contains("Planned copies: 2; Exact duplicates: 1"));
    assert_eq!(snapshot(&source), before);
    assert!(!f.0.join("out").exists());
}

#[test]
fn dry_run_detects_existing_output_conflicts_without_changes() {
    let f = Fixture::new();
    let source = f.dir("source");
    fs::write(
        source.join("tone.flac"),
        include_bytes!("fixtures/tone.flac"),
    )
    .unwrap();
    let output = f.dir("out");
    let destination =
        output.join("FLAC/Test Ensemble/Test Album/02-03 - Fixture Tone - Test Artist.flac");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"existing output").unwrap();
    let source_before = snapshot(&source);
    let output_before = snapshot(&output);
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_alb"))
            .arg("build")
            .arg("--dry-run")
            .arg("--input")
            .arg(&source)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("Problem routing: 1 files")
    );
    assert_eq!(snapshot(&source), source_before);
    assert_eq!(snapshot(&output), output_before);
    fs::remove_file(&destination).unwrap();
    fs::remove_dir_all(output.join("FLAC")).unwrap();
    fs::create_dir(output.join("flac")).unwrap();
    assert!(
        String::from_utf8(run().stderr)
            .unwrap()
            .contains("Problem routing: 1 files")
    );
    fs::remove_dir(output.join("flac")).unwrap();
    fs::write(output.join("FLAC"), b"not directory").unwrap();
    assert!(
        String::from_utf8(run().stderr)
            .unwrap()
            .contains("Problem routing: 1 files")
    );
}

#[cfg(unix)]
#[test]
fn dry_run_does_not_follow_output_symlinks() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let source = f.dir("source");
    fs::write(
        source.join("tone.flac"),
        include_bytes!("fixtures/tone.flac"),
    )
    .unwrap();
    let output = f.dir("out");
    let elsewhere = f.dir("elsewhere");
    symlink(&elsewhere, output.join("FLAC")).unwrap();
    let before = snapshot(&elsewhere);
    let result = Command::new(env!("CARGO_BIN_EXE_alb"))
        .arg("build")
        .arg("--input")
        .arg(&source)
        .arg("--output")
        .arg(&output)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("Problem routing: 1 files")
    );
    assert_eq!(snapshot(&elsewhere), before);
}

#[test]
fn mixed_library_dry_run_preserves_every_extension_and_nested_unknown() {
    let f = Fixture::new();
    let source = f.dir("source");
    let nested = f.dir("source/extras/nested");
    fs::write(
        source.join("good.FLAC"),
        include_bytes!("fixtures/tone.flac"),
    )
    .unwrap();
    for name in [
        "broken.flac",
        "track.M4A",
        "track.mp3",
        "track.ogg",
        "track.WAV",
    ] {
        fs::write(source.join(name), b"unreadable or untagged contents").unwrap();
    }
    for name in [
        "note.txt",
        "book.m4b",
        "song.opus",
        "movie.mp4",
        "song.oga",
        "song.wave",
    ] {
        fs::write(nested.join(name), b"unknown file preserved").unwrap();
    }
    let before = snapshot(&source);
    let result = Command::new(env!("CARGO_BIN_EXE_alb"))
        .args(["build", "--input"])
        .arg(&source)
        .arg("--output")
        .arg(f.0.join("out"))
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    let text = String::from_utf8(result.stderr).unwrap();
    assert!(text.contains("Plan summary: 12 entries"));
    assert!(text.contains("Problem routing: 5 files"));
    assert_eq!(snapshot(&source), before);
    assert!(!f.0.join("out").exists());
}

#[test]
fn malformed_flac_remains_in_catalog_and_can_be_hashed_exactly() {
    let f = Fixture::new();
    let source = f.dir("source");
    for name in ["a.flac", "b.FLAC"] {
        fs::write(source.join(name), b"bad FLAC").unwrap();
    }
    let before = snapshot(&source);
    let result = Command::new(env!("CARGO_BIN_EXE_alb"))
        .args(["scan", "--input"])
        .arg(&source)
        .args(["--verbose", "--hash"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let summary = String::from_utf8(result.stderr).unwrap();
    assert!(summary.contains("Catalog: 2 files, 2 metadata/read errors"));
    assert!(summary.contains("Exact file hashes: 2 files, 1 duplicate groups, 0 errors"));
    assert_eq!(snapshot(&source), before);
}

#[test]
fn dry_run_loose_track_naming_preserves_source_and_absent_output() {
    let f = Fixture::new();
    let source = f.dir("source");
    let mut bytes = include_bytes!("fixtures/tone.flac").to_vec();
    // Disable only the ALBUM tag in this synthetic fixture without changing lengths.
    let marker = b"ALBUM=Test Album";
    let offset = bytes
        .windows(marker.len())
        .position(|s| s.eq_ignore_ascii_case(marker))
        .unwrap();
    bytes[offset..offset + 5].copy_from_slice(b"OTHER");
    fs::write(source.join("loose.flac"), bytes).unwrap();
    let before = snapshot(&source);
    let result = Command::new(env!("CARGO_BIN_EXE_alb"))
        .args(["build", "--input"])
        .arg(&source)
        .arg("--output")
        .arg(f.0.join("out"))
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    let report = String::from_utf8(result.stderr).unwrap();
    assert!(report.contains("Planned copies: 1; Exact duplicates: 0"));
    assert!(report.contains("Problem routing: 0 files"));
    assert_eq!(snapshot(&source), before);
    assert!(!f.0.join("out").exists());
}

#[test]
fn all_supported_formats_plan_by_metadata_without_source_changes() {
    let f = Fixture::new();
    let source = f.dir("source");
    for (name, bytes) in [
        ("a.FLAC", include_bytes!("fixtures/tone.flac").as_slice()),
        ("b.M4A", include_bytes!("fixtures/tone.m4a").as_slice()),
        ("c.MP3", include_bytes!("fixtures/tone.mp3").as_slice()),
        ("d.OGG", include_bytes!("fixtures/tone.ogg").as_slice()),
        ("e.WAV", include_bytes!("fixtures/tone.wav").as_slice()),
    ] {
        fs::write(source.join(name), bytes).unwrap();
    }
    let before = snapshot(&source);
    let result = Command::new(env!("CARGO_BIN_EXE_alb"))
        .args(["build", "--input"])
        .arg(&source)
        .arg("--output")
        .arg(f.0.join("out"))
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    let report = String::from_utf8(result.stderr).unwrap();
    assert!(report.contains("Planned copies: 5"));
    assert!(report.contains("Metadata warnings: 0"));
    assert_eq!(snapshot(&source), before);
    assert!(!f.0.join("out").exists());
}
