use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        for n in 0.. {
            let root = std::env::temp_dir().join(format!("alb-multi-{}-{n}", std::process::id()));
            if fs::create_dir(&root).is_ok() {
                for dir in ["a", "b"] {
                    fs::create_dir(root.join(dir)).unwrap();
                }
                return Self(root);
            }
        }
        unreachable!()
    }
    fn run(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_alb"))
            .current_dir(&self.0)
            .args(["build", "--input", "a", "--input", "b", "--output", "out"])
            .args(extra)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for p in fs::read_dir(root).unwrap() {
        let p = p.unwrap().path();
        if p.is_dir() {
            found.extend(files(&p));
        } else {
            found.push(p);
        }
    }
    found
}
fn success(result: &Output) {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn combines_deduplicates_and_resumes_without_losing_colliding_files() {
    let f = Fixture::new();
    for dir in ["a", "b"] {
        fs::write(
            f.0.join(dir).join("same.flac"),
            include_bytes!("fixtures/tone.flac"),
        )
        .unwrap();
        fs::create_dir(f.0.join(dir).join("Band (Live)")).unwrap();
        fs::write(
            f.0.join(dir).join("Band (Live)/broken.mp3"),
            format!("bad audio from {dir}"),
        )
        .unwrap();
        fs::write(f.0.join(dir).join("notes.txt"), dir).unwrap();
    }
    success(&f.run(&["--dry-run"]));
    assert!(!f.0.join("out").exists());
    let result = f.run(&[]);
    success(&result);
    let text = String::from_utf8_lossy(&result.stderr);
    assert!(text.contains("1 exact duplicates"), "{text}");
    let output = files(&f.0.join("out"));
    assert_eq!(
        output
            .iter()
            .filter(|p| p.extension().is_some_and(|x| x == "flac"))
            .count(),
        1
    );
    let problems: Vec<_> = output
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "mp3"))
        .collect();
    assert_eq!(problems.len(), 2);
    assert!(
        problems
            .iter()
            .all(|p| p.parent().unwrap().ends_with("Band (Live)"))
    );
    let mut contents: Vec<_> = problems
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    contents.sort();
    assert_eq!(contents, ["bad audio from a", "bad audio from b"]);
    assert_eq!(files(&f.0.join("out/UNKNOWN")).len(), 2);
    let resumed = f.run(&["--resume"]);
    success(&resumed);
    assert!(String::from_utf8_lossy(&resumed.stderr).contains("5 verified existing files reused"));
    let report: String = files(&f.0.join("out/_ALB"))
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    assert!(report.contains("INPUT_ROOT"));
    for dir in ["a", "b"] {
        assert_eq!(
            fs::read(f.0.join(dir).join("same.flac")).unwrap(),
            include_bytes!("fixtures/tone.flac")
        );
    }
}
#[test]
fn validates_every_root_before_writing() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("a/nested")).unwrap();
    for args in [
        vec!["--input", "a", "--input", "a", "--output", "out"],
        vec!["--input", "a", "--input", "a/nested", "--output", "out"],
        vec!["--input", "a", "--input", "b", "--output", "b/output"],
        vec!["--input", "a", "--input", "missing", "--output", "out"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_alb"))
            .current_dir(&f.0)
            .arg("build")
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!f.0.join("out").exists());
        assert!(!f.0.join("b/output").exists());
    }
}
#[test]
fn scan_combines_roots_and_hashes_across_them() {
    let f = Fixture::new();
    for dir in ["a", "b"] {
        fs::write(
            f.0.join(dir).join("song.flac"),
            include_bytes!("fixtures/tone.flac"),
        )
        .unwrap();
    }
    let result = Command::new(env!("CARGO_BIN_EXE_alb"))
        .current_dir(&f.0)
        .args(["scan", "--input", "a", "--input", "b", "--hash"])
        .output()
        .unwrap();
    success(&result);
    assert!(String::from_utf8_lossy(&result.stderr).contains("2 files, 1 duplicate groups"));
    assert!(!f.0.join("out").exists());
}
#[cfg(unix)]
#[test]
fn aliases_cannot_scan_the_same_root_twice() {
    let f = Fixture::new();
    std::os::unix::fs::symlink(f.0.join("a"), f.0.join("alias")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_alb"))
        .current_dir(&f.0)
        .args([
            "build", "--input", "a", "--input", "alias", "--output", "out",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!f.0.join("out").exists());
}
