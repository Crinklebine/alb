#[cfg(target_os = "linux")]
mod build_report;
mod candidates;
mod catalog;
mod cli;
#[cfg(any(target_os = "linux", test))]
mod copying;
mod discovery;
#[cfg(target_os = "linux")]
mod execution;
mod hashing;
mod inspection;
mod paths;
mod plan;
mod problems;
mod progress;
mod report;
#[cfg(target_os = "linux")]
mod safe_fs;
mod source;
#[cfg(target_os = "linux")]
mod space;

use std::{env, path::Path, process::ExitCode};

fn scan(
    input: &Path,
    verbose: bool,
    hash: bool,
    plan_output: Option<&Path>,
    execute: bool,
    resume: bool,
) -> ExitCode {
    let catalog = match discovery::discover(input) {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!(
                "error: cannot scan {:?}: {:?}",
                error.path,
                error.source.to_string()
            );
            return ExitCode::from(1);
        }
    };
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
    let inspected = inspection::inspect(&catalog.files);
    eprintln!(
        "Catalog: {} files, {} metadata/read errors.",
        inspected.tracks.len(),
        inspected.errors.len()
    );
    let hashes = (hash || plan_output.is_some()).then(|| hashing::analyze(&inspected.tracks));
    if let Some(output) = plan_output {
        let planning = progress::Progress::new("Planning destinations", Some(catalog.files.len()));
        let mut plan = plan::generate(&catalog, &inspected, input, output, hashes.as_ref());
        plan::check_existing_output_mode(&mut plan, resume);
        problems::route(&mut plan, output);
        planning.set(catalog.files.len());
        drop(planning);
        #[cfg(target_os = "linux")]
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
        #[cfg(target_os = "linux")]
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
        #[cfg(target_os = "linux")]
        match execution::execute_resilient(&plan, input, output, resume) {
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
        #[cfg(not(target_os = "linux"))]
        {
            eprintln!("error: safe build execution currently requires Linux; use --dry-run");
            return ExitCode::from(1);
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
            let input = match paths::validate_input(&args.input) {
                Ok(input) => input,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(1);
                }
            };
            return scan(&input, args.verbose, args.hash, None, false, false);
        }
        Ok(cli::Command::Build(paths)) => {
            let dry_run = paths.dry_run;
            let resume = paths.resume;
            let paths = match paths::validate(&paths.input, &paths.output) {
                Ok(paths) => paths,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(1);
                }
            };
            return scan(
                &paths.input,
                false,
                false,
                Some(paths.output.as_path()),
                !dry_run,
                resume,
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
