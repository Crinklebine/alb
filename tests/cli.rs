use std::process::Command;

#[test]
fn help_and_default_describe_current_scope() {
    for args in [vec![], vec!["--help"], vec!["-h"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_alb"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Usage: alb [OPTIONS]"));
        assert!(stdout.contains("Build execution requires Linux"));
    }
}

#[test]
fn version_matches_package_version() {
    for arg in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_alb"))
            .arg(arg)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            output.stdout,
            format!("alb {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
    }
}

#[test]
fn unsupported_arguments_fail() {
    for args in [vec!["analyze"], vec!["--unknown"], vec!["--help", "extra"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_alb"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("unsupported arguments")
        );
    }
}

#[test]
fn build_help_explains_required_options_and_limitations() {
    for arg in ["--help", "-h"] {
        let output = Command::new(env!("CARGO_BIN_EXE_alb"))
            .args(["build", arg])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("--input SOURCE --output DESTINATION"));
        assert!(stdout.contains("never overwrites destinations"));
    }
}

#[test]
fn malformed_build_arguments_are_usage_errors() {
    let cases = [
        (vec![], "required option --input"),
        (vec!["--input", "source"], "required option --output"),
        (vec!["--output", "dest"], "required option --input"),
        (vec!["--input"], "--input requires"),
        (vec!["--input", ""], "--input requires"),
        (vec!["--input", "--output", "dest"], "--input requires"),
        (vec!["--input", "source", "--output"], "--output requires"),
        (
            vec!["--input", "source", "--output", ""],
            "--output requires",
        ),
        (
            vec!["--input", "a", "--input", "b"],
            "--input was supplied more than once",
        ),
        (
            vec!["--output", "a", "--output", "b"],
            "--output was supplied more than once",
        ),
        (vec!["source", "dest"], "unsupported arguments"),
        (vec!["--unknown"], "unsupported arguments"),
        (vec!["--input=a", "--output=b"], "unsupported arguments"),
        (
            vec!["--input", "a", "--output", "b", "extra"],
            "unsupported arguments",
        ),
    ];
    for (args, diagnostic) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_alb"))
            .arg("build")
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains(diagnostic),
            "{args:?}"
        );
    }
}

#[test]
fn dry_run_leaves_source_and_absent_output_untouched() {
    use std::fs;
    // Atomically reserve a unique temporary directory; never touch a music library.
    let mut attempt = 0;
    let root = loop {
        let root = std::env::temp_dir().join(format!("alb-cli-{}-{attempt}", std::process::id()));
        match fs::create_dir(&root) {
            Ok(()) => break root,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => attempt += 1,
            Err(error) => panic!("cannot create test directory: {error}"),
        }
    };
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let source = root.join("source music");
    let destination = root.join("new library");
    fs::create_dir(&source).unwrap();
    let track = source.join("synthetic.flac");
    fs::write(&track, b"synthetic fixture, not audio").unwrap();
    let before = fs::metadata(&track).unwrap().modified().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_alb"))
        .arg("build")
        .arg("--dry-run")
        .arg("--input")
        .arg(&source)
        .arg("--output")
        .arg(&destination)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Dry-run"));
    assert!(stderr.contains("Root paths validated"));
    assert!(stderr.contains("Discovery: 1 regular files, 1 directories"));
    assert!(stderr.contains("0 errors."));
    assert!(!destination.exists());
    assert_eq!(fs::read(&track).unwrap(), b"synthetic fixture, not audio");
    assert_eq!(fs::metadata(&track).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read_dir(&source).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn scan_help_and_argument_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_alb"))
        .args(["scan", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("alb scan --input SOURCE")
    );
    for args in [
        vec!["scan"],
        vec!["scan", "--input"],
        vec!["scan", "--input", ""],
        vec!["scan", "--input", "a", "--input", "b"],
        vec!["scan", "--input", "a", "--verbose", "--verbose"],
        vec!["scan", "--input", "a", "--output", "b"],
        vec!["scan", "--input", "--verbose"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_alb"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn removed_analyzer_option_is_rejected() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_alb"))
        .args(["scan", "--input", ".", "--validate"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
