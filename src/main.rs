mod acoustid;
mod build_report;
mod candidates;
mod catalog;
mod cli;
mod copying;
mod discovery;
mod execution;
mod hashing;
mod inspection;
mod paths;
mod plan;
mod platform;
mod problems;
mod progress;
mod report;
mod safe_fs;
mod source;
mod space;
mod tagging;

use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
};

fn scan(
    inputs: &[PathBuf],
    verbose: bool,
    hash: bool,
    plan_output: Option<&Path>,
    execute: bool,
    resume: bool,
    acoustid_key: Option<acoustid::ApiKey>,
) -> ExitCode {
    let mut catalog = discovery::Catalog::default();
    for input in inputs {
        let found = match discovery::discover(input) {
            Ok(found) => found,
            Err(error) => {
                eprintln!("error: cannot scan {:?}: {}", error.path, error.source);
                return ExitCode::from(1);
            }
        };
        catalog.files.extend(found.files);
        catalog.directories += found.directories;
        catalog.skipped_symlinks += found.skipped_symlinks;
        catalog.skipped_special += found.skipped_special;
        catalog.errors.extend(found.errors);
    }
    catalog.files.sort();
    eprintln!(
        "Discovery: {} regular files, {} directories (including root), {} symlinks skipped, {} special entries skipped, {} errors.",
        catalog.files.len(),
        catalog.directories,
        catalog.skipped_symlinks,
        catalog.skipped_special,
        catalog.errors.len()
    );
    let candidate_count = catalog
        .files
        .iter()
        .filter(|path| candidates::classify(path) != candidates::FileType::Unknown)
        .count();
    eprintln!(
        "Classification: {} supported files, {} unknown files.",
        candidate_count,
        catalog.files.len() - candidate_count
    );
    let inspected = inspection::inspect_with_key(&catalog.files, acoustid_key);
    eprintln!(
        "Catalog: {} files, {} metadata/read errors.",
        inspected.tracks.len(),
        inspected.errors.len()
    );
    let hashes = (hash || plan_output.is_some()).then(|| hashing::analyze(&inspected.tracks));
    if let Some(output) = plan_output {
        let planning = progress::Progress::new("Planning destinations", Some(catalog.files.len()));
        let mut plan = plan::generate_many(&catalog, &inspected, inputs, output, hashes.as_ref());
        plan::check_existing_output_mode(&mut plan, resume);
        problems::route(&mut plan, &inputs[0], output);
        planning.set(catalog.files.len());
        drop(planning);
        let space_result = space::check(&plan, output, resume);
        let resolved = plan
            .entries
            .iter()
            .filter(|e| e.collision && e.issues.is_empty())
            .count();
        eprintln!(
            "Plan summary: {} entries, {} filename conflicts resolved, {} blocked entries, {} discovery errors.",
            plan.entries.len(),
            resolved,
            plan.unresolved(),
            plan.discovery_errors.len()
        );
        eprintln!(
            "Problem routing: {} files will receive explanations under Problem Files.",
            plan.entries
                .iter()
                .filter(|e| !problems::reasons(e).is_empty())
                .count()
        );
        eprintln!(
            "Planned copies: {}; Exact duplicates: {}; Metadata warnings: {}.",
            plan.entries
                .iter()
                .filter(|e| e.action == plan::Action::Keep)
                .count(),
            plan.entries
                .iter()
                .filter(|e| matches!(e.action, plan::Action::DuplicateOf(_)))
                .count(),
            plan.metadata_warnings
        );
        if let Err(error) = space_result {
            eprintln!("error: {error}. No copying started.");
            return ExitCode::from(1);
        }
        if !execute {
            eprintln!(
                "Root paths validated. Dry-run: no files copied or changed. No report written."
            );
            return if plan.unresolved() == 0 && plan.discovery_errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            };
        }
        match execution::execute_resilient(&plan, &inputs[0], output, resume) {
            Ok(result) => {
                eprintln!(
                    "Build processing finished: {} verified copies, {} exact duplicates retained through their representatives; {} verified existing files reused.",
                    result.copied, result.duplicates, result.reused
                );
                eprintln!(
                    "Problem files: {}; files/errors not fully handled: {}. See output Problem Files and _ALB reports.",
                    result.problems, result.failed
                );
                return if result.failed == 0 {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                };
            }
            Err(error) => {
                eprintln!(
                    "error: build incomplete: {error}. Completed files remain; partial files are never overwritten."
                );
                return ExitCode::from(1);
            }
        }
    }
    if let Some(hashes) = &hashes {
        eprintln!(
            "Exact file hashes: {} files, {} duplicate groups, {} errors.",
            hashes.hashed,
            hashes.groups.len(),
            hashes.errors.len()
        );
    }
    if verbose {
        let stdout = std::io::stdout();
        if let Err(error) = report::write_details(&mut stdout.lock(), &catalog, &inspected) {
            eprintln!("error: cannot write scan report: {error}");
            return ExitCode::from(1);
        }
    }
    if hash
        && let Some(hashes) = &hashes
        && let Err(error) = hashing::write_groups(&mut std::io::stdout().lock(), hashes)
    {
        eprintln!("error: cannot write hash report: {error}");
        return ExitCode::from(1);
    }
    if !catalog.errors.is_empty()
        || !inspected.errors.is_empty()
        || hashes
            .as_ref()
            .is_some_and(|hashes| !hashes.errors.is_empty())
    {
        eprintln!(
            "Warning: scan has discovery, metadata, or hashing errors; use scan --verbose for details."
        );
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    match cli::parse(env::args_os().skip(1).collect()) {
        Ok(cli::Command::Help) => println!("{}", cli::HELP),
        Ok(cli::Command::Version) => println!("alb {}", env!("CARGO_PKG_VERSION")),
        Ok(cli::Command::BuildHelp) => println!("{}", cli::BUILD_HELP),
        Ok(cli::Command::ScanHelp) => println!("{}", cli::SCAN_HELP),
        Ok(cli::Command::Scan(args)) => {
            let input = match paths::validate_inputs(&args.input) {
                Ok(input) => input,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(1);
                }
            };
            return scan(
                &input,
                args.verbose,
                args.hash,
                None,
                false,
                false,
                args.acoustid_key,
            );
        }
        Ok(cli::Command::Build(paths)) => {
            let acoustid_key = paths.acoustid_key;
            let dry_run = paths.dry_run;
            let resume = paths.resume;
            let inputs = match paths::validate_inputs(&paths.input) {
                Ok(inputs) => inputs,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(1);
                }
            };
            let mut output = paths.output;
            for input in &inputs {
                match paths::validate(input, &output) {
                    Ok(roots) => output = roots.output,
                    Err(error) => {
                        eprintln!("error: {error}");
                        return ExitCode::from(1);
                    }
                }
            }
            return scan(
                &inputs,
                false,
                false,
                Some(output.as_path()),
                !dry_run,
                resume,
                acoustid_key,
            );
        }
        Err(error) => {
            eprintln!(
                "error: {error}\nTry 'alb --help', 'alb build --help', or 'alb scan --help'."
            );
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
