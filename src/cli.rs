use crate::acoustid::ApiKey;
use std::{ffi::OsString, fmt, path::PathBuf};

pub const HELP: &str = "alb — Audio Library Builder

Usage: alb [OPTIONS]
       alb build --input SOURCE [--input SOURCE ...] --output DESTINATION [--dry-run] [--resume]
       alb scan --input SOURCE [--input SOURCE ...] [--verbose] [--hash]

Commands:
  build           Build a verified output library
  scan            Inspect source files without creating an output library

Options:
  -h, --help       Print help
  -V, --version    Print version

Build supports Linux, macOS and Windows; use --dry-run to preview without writes.
Use 'alb build --help' or 'alb scan --help' for command options.
Source libraries must always remain immutable.";

pub const BUILD_HELP: &str = "Usage: alb build --input SOURCE [--input SOURCE ...] --output DESTINATION [--dry-run] [--resume]

Options:
  --acoustid-key KEY     Optional Artist/Title fallback using fpcalc and AcoustID
  --input SOURCE         Required; repeat for multiple source libraries
  --output DESTINATION   Required output library path
  --resume               Verify and reuse matching outputs; retain old partials
  --dry-run              Hash files and summarize planned work; never write files
  -h, --help             Print help

Use separate option values. Prefix paths beginning with '-' with './'.
Root paths are checked for equal or nested directories, including symlink aliases.
All inputs must exist and must not overlap each other or the output.
Output may be absent. Dry-run creates no directories.
Discovery counts regular files and skips all symlinks.
Files are grouped by extension: FLAC, M4A, MP3, OGG, WAV, UNKNOWN.
Basic metadata is read for all five supported types without changing files.
Build copies through verified partial files and never overwrites destinations.
Copies preserve modification time; macOS/Windows also restore available creation time.
Original timestamps are archived in _ALB reports on all platforms.
Free space is checked before copying, with a safety allowance.
File problems go to Problem Files/<Problem Type> with a text explanation.
Unreadable files are reported; other files continue. Root/output safety failures are fatal.
Terminal progress uses one status line; redirected stderr has stage summaries.
Execution uses native filesystem safety checks. Failed partials are retained. Resume rechecks current sources and output bytes.";

pub const SCAN_HELP: &str =
    "Usage: alb scan --input SOURCE [--input SOURCE ...] [--verbose] [--hash]

Options:
  --acoustid-key KEY  Optional Artist/Title fallback using fpcalc and AcoustID
  --input SOURCE  Required; repeat for multiple source libraries
  --verbose       Print per-file types, metadata, and errors
  --hash          Hash all catalog files and report exact duplicate groups
  -h, --help      Print help

Scan is read-only. All symlinks below the input root are skipped.
Classification uses extensions only. Basic tags populate the generic catalog.
Exit status: 0 completed, 1 scan/inspection failure, 2 usage error.";

#[derive(Debug, PartialEq, Eq)]
pub struct ScanArgs {
    pub acoustid_key: Option<ApiKey>,
    pub input: Vec<PathBuf>,
    pub verbose: bool,
    pub hash: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BuildArgs {
    pub acoustid_key: Option<ApiKey>,
    pub input: Vec<PathBuf>,
    pub output: PathBuf,
    pub dry_run: bool,
    pub resume: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    BuildHelp,
    ScanHelp,
    Scan(ScanArgs),
    Build(BuildArgs),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    UnsupportedArguments,
    MissingOption(&'static str),
    MissingValue(&'static str),
    DuplicateOption(&'static str),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedArguments => write!(f, "unsupported arguments"),
            Self::MissingOption(option) => write!(f, "required option {option} is missing"),
            Self::MissingValue(option) => write!(f, "{option} requires a non-empty value"),
            Self::DuplicateOption(option) => write!(f, "{option} was supplied more than once"),
        }
    }
}

impl std::error::Error for CliError {}

pub fn parse(args: Vec<OsString>) -> Result<Command, CliError> {
    match args.as_slice() {
        [] => Ok(Command::Help),
        [arg] if arg == "--help" || arg == "-h" => Ok(Command::Help),
        [arg] if arg == "--version" || arg == "-V" => Ok(Command::Version),
        [command, rest @ ..] if command == "build" => parse_build(rest),
        [command, rest @ ..] if command == "scan" => parse_scan(rest),
        _ => Err(CliError::UnsupportedArguments),
    }
}

fn parse_build(args: &[OsString]) -> Result<Command, CliError> {
    if matches!(args, [arg] if arg == "--help" || arg == "-h") {
        return Ok(Command::BuildHelp);
    }

    let mut input = Vec::new();
    let mut acoustid_key = None;
    let mut output = None;
    let mut dry_run = false;
    let mut resume = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--acoustid-key" {
            if acoustid_key.is_some() {
                return Err(CliError::DuplicateOption("--acoustid-key"));
            }
            let value = args
                .next()
                .and_then(|v| v.to_str())
                .filter(|v| !v.trim().is_empty() && !v.starts_with('-'))
                .ok_or(CliError::MissingValue("--acoustid-key"))?;
            acoustid_key = Some(ApiKey::new(value.to_owned()));
            continue;
        }
        if arg == "--resume" {
            if resume {
                return Err(CliError::DuplicateOption("--resume"));
            }
            resume = true;
            continue;
        }
        if arg == "--dry-run" {
            if dry_run {
                return Err(CliError::DuplicateOption("--dry-run"));
            }
            dry_run = true;
            continue;
        }
        if arg == "--input" {
            let value = args.next().ok_or(CliError::MissingValue("--input"))?;
            if value.is_empty() || value.as_encoded_bytes().starts_with(b"-") {
                return Err(CliError::MissingValue("--input"));
            }
            input.push(PathBuf::from(value));
            continue;
        }
        let (name, slot) = if arg == "--output" {
            ("--output", &mut output)
        } else {
            return Err(CliError::UnsupportedArguments);
        };
        if slot.is_some() {
            return Err(CliError::DuplicateOption(name));
        }
        let value = args.next().ok_or(CliError::MissingValue(name))?;
        // Reject option-looking values without converting the path to Unicode.
        if value.is_empty() || value.as_encoded_bytes().starts_with(b"-") {
            return Err(CliError::MissingValue(name));
        }
        *slot = Some(PathBuf::from(value));
    }

    Ok(Command::Build(BuildArgs {
        acoustid_key,
        input: if input.is_empty() {
            return Err(CliError::MissingOption("--input"));
        } else {
            input
        },
        output: output.ok_or(CliError::MissingOption("--output"))?,
        dry_run,
        resume,
    }))
}

fn parse_scan(args: &[OsString]) -> Result<Command, CliError> {
    if matches!(args, [arg] if arg == "--help" || arg == "-h") {
        return Ok(Command::ScanHelp);
    }
    let mut input = Vec::new();
    let mut acoustid_key = None;
    let mut verbose = false;
    let mut hash = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--acoustid-key" {
            if acoustid_key.is_some() {
                return Err(CliError::DuplicateOption("--acoustid-key"));
            }
            let value = args
                .next()
                .and_then(|v| v.to_str())
                .filter(|v| !v.trim().is_empty() && !v.starts_with('-'))
                .ok_or(CliError::MissingValue("--acoustid-key"))?;
            acoustid_key = Some(ApiKey::new(value.to_owned()));
            continue;
        }
        if arg == "--hash" {
            if hash {
                return Err(CliError::DuplicateOption("--hash"));
            }
            hash = true;
        } else if arg == "--verbose" {
            if verbose {
                return Err(CliError::DuplicateOption("--verbose"));
            }
            verbose = true;
        } else if arg == "--input" {
            let value = args.next().ok_or(CliError::MissingValue("--input"))?;
            if value.is_empty() || value.as_encoded_bytes().starts_with(b"-") {
                return Err(CliError::MissingValue("--input"));
            }
            input.push(PathBuf::from(value));
        } else {
            return Err(CliError::UnsupportedArguments);
        }
    }
    Ok(Command::Scan(ScanArgs {
        acoustid_key,
        input: if input.is_empty() {
            return Err(CliError::MissingOption("--input"));
        } else {
            input
        },
        verbose,
        hash,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_paths_in_either_option_order() {
        for args in [
            ["--input", "music collection", "--output", "../clean"],
            ["--output", "../clean", "--input", "music collection"],
        ] {
            let args = std::iter::once("build")
                .chain(args)
                .map(OsString::from)
                .collect();
            assert_eq!(
                parse(args),
                Ok(Command::Build(BuildArgs {
                    acoustid_key: None,
                    input: vec![PathBuf::from("music collection")],
                    output: PathBuf::from("../clean"),
                    dry_run: false,
                    resume: false,
                }))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_unicode_paths() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"music-\xff".to_vec());
        let args = vec![
            "build".into(),
            "--input".into(),
            path.clone(),
            "--output".into(),
            "clean".into(),
        ];
        assert_eq!(
            parse(args),
            Ok(Command::Build(BuildArgs {
                acoustid_key: None,
                input: vec![PathBuf::from(path)],
                output: PathBuf::from("clean"),
                dry_run: false,
                resume: false,
            }))
        );
    }
}

#[cfg(test)]
mod acoustid_tests {
    use super::*;
    #[test]
    fn optional_key_parses_for_both_commands_and_is_redacted() {
        for args in [
            vec![
                "scan",
                "--input",
                "source",
                "--acoustid-key",
                "private-test-key",
            ],
            vec![
                "build",
                "--input",
                "source",
                "--output",
                "out",
                "--acoustid-key",
                "private-test-key",
            ],
        ] {
            let command = parse(args.into_iter().map(Into::into).collect()).unwrap();
            assert!(!format!("{command:?}").contains("private-test-key"));
            let key = match command {
                Command::Scan(s) => s.acoustid_key,
                Command::Build(b) => b.acoustid_key,
                _ => panic!("wrong command"),
            };
            assert_eq!(key, Some(ApiKey::new("private-test-key".into())));
        }
    }
    #[test]
    fn missing_blank_duplicate_and_option_looking_keys_have_safe_errors() {
        for tail in [
            vec!["--acoustid-key"],
            vec!["--acoustid-key", " "],
            vec!["--acoustid-key", "--hash"],
            vec![
                "--acoustid-key",
                "private-test-key",
                "--acoustid-key",
                "private-test-key",
            ],
        ] {
            let args = [vec!["scan", "--input", "source"], tail].concat();
            let error = parse(args.into_iter().map(Into::into).collect()).unwrap_err();
            assert!(!format!("{error} {error:?}").contains("private-test-key"));
        }
        assert!(BUILD_HELP.contains("--acoustid-key"));
        assert!(SCAN_HELP.contains("--acoustid-key"));
    }
}
