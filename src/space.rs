//! Conservative free-space estimates; never a reservation or quota guarantee.
use crate::{
    plan::{Action, BuildPlan},
    safe_fs::Directory,
};
use std::{io, path::Path};
const RESERVE: u64 = 16 * 1024 * 1024;
pub fn available(directory: &Directory) -> io::Result<u64> {
    directory.available_space()
}
pub fn required(bytes: u64, entries: usize) -> io::Result<u64> {
    let overhead = u64::try_from(entries)
        .ok()
        .and_then(|n| n.checked_mul(16 * 1024));
    bytes
        .checked_add(bytes / 100)
        .and_then(|n| n.checked_add(RESERVE))
        .and_then(|n| overhead.and_then(|extra| n.checked_add(extra)))
        .ok_or_else(|| io::Error::other("required space overflow"))
}
pub fn ensure(required: u64, available: u64) -> io::Result<()> {
    if available < required {
        Err(io::Error::new(
            io::ErrorKind::StorageFull,
            format!(
                "insufficient output space: need {required} bytes including safety allowance, available {available} bytes; free space or use another destination"
            ),
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct Estimate {
    new_bytes: u64,
    comparison_bytes: u64,
}
impl Estimate {
    fn bytes(&self) -> io::Result<u64> {
        self.new_bytes
            .checked_add(self.comparison_bytes)
            .ok_or_else(|| io::Error::other("copy size overflow"))
    }
}

fn estimate(plan: &BuildPlan, resume: bool) -> io::Result<Estimate> {
    let mut estimate = Estimate::default();
    for entry in &plan.entries {
        if matches!(entry.action, Action::DuplicateOf(_)) {
            continue;
        }
        let bytes = entry.source_stamp.as_ref().map_or(0, |s| s.len);
        let reusable = resume
            && entry.action == Action::Keep
            && entry.issues.is_empty()
            && crate::problems::reasons(entry).is_empty()
            && entry.destination.as_ref().is_some_and(|p| {
                std::fs::symlink_metadata(p)
                    .is_ok_and(|m| m.is_file() && !crate::platform::is_link(&m))
            });
        if reusable {
            if entry.metadata_update.is_some() {
                // Successful tagged comparisons are removed before the next entry.
                estimate.comparison_bytes = estimate.comparison_bytes.max(bytes);
                continue;
            }
            if entry
                .notes
                .iter()
                .any(|n| n.starts_with("resume: existing bytes verified"))
            {
                continue;
            }
        }
        estimate.new_bytes = estimate
            .new_bytes
            .checked_add(bytes)
            .ok_or_else(|| io::Error::other("copy size overflow"))?;
    }
    Ok(estimate)
}

pub fn check(plan: &BuildPlan, output: &Path, resume: bool) -> io::Result<()> {
    let mut ancestor = output;
    while !ancestor.try_exists()? {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| io::Error::other("no existing output ancestor"))?;
    }
    let directory = Directory::absolute(ancestor, false, None)?;
    let estimate = estimate(plan, resume)?;
    let needed = required(estimate.bytes()?, plan.entries.len())?;
    let free = available(&directory)?;
    eprintln!(
        "Space check: {:.2} GiB new copies; {:.2} GiB comparison scratch; {:.2} GiB needed including allowance; {:.2} GiB available.",
        estimate.new_bytes as f64 / 1_073_741_824.0,
        estimate.comparison_bytes as f64 / 1_073_741_824.0,
        needed as f64 / 1_073_741_824.0,
        free as f64 / 1_073_741_824.0
    );
    ensure(needed, free)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = (0..)
                .find_map(|n| {
                    let path = std::env::temp_dir()
                        .join(format!("alb-resume-space-{}-{n}", std::process::id()));
                    fs::create_dir(&path).ok().map(|_| path)
                })
                .unwrap();
            let root = fs::canonicalize(root).unwrap();
            fs::create_dir(root.join("in")).unwrap();
            fs::write(
                root.join("in/song.flac"),
                include_bytes!("../tests/fixtures/tone.flac"),
            )
            .unwrap();
            fs::write(
                root.join("in/song.wav"),
                include_bytes!("../tests/fixtures/tone.wav"),
            )
            .unwrap();
            fs::write(root.join("in/memo.txt"), b"original notes").unwrap();
            Self(root)
        }
        fn plan(&self, resume: bool) -> BuildPlan {
            let input = self.0.join("in");
            let output = self.0.join("out");
            let files = crate::discovery::discover(&input).unwrap();
            let tracks = crate::inspection::inspect(&files.files);
            let hashes = crate::hashing::analyze(&tracks.tracks);
            let mut plan = crate::plan::generate(&files, &tracks, &input, &output, Some(&hashes));
            crate::plan::check_existing_output_mode(&mut plan, resume);
            crate::problems::route(&mut plan, &input, &output);
            plan
        }
        fn run(&self, plan: &BuildPlan, resume: bool) -> crate::execution::BuildResult {
            let result = crate::execution::execute_resilient(
                plan,
                &self.0.join("in"),
                &self.0.join("out"),
                resume,
            )
            .unwrap();
            assert_eq!(result.failed, 0);
            result
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn completed_resume_needs_only_the_largest_tagged_comparison() {
        let f = Fixture::new();
        let initial = f.plan(false);
        assert_eq!(f.run(&initial, false).copied, 3);
        let resumed = f.plan(true);
        let space = estimate(&resumed, true).unwrap();
        let largest = include_bytes!("../tests/fixtures/tone.flac").len() as u64;
        let all_audio = largest + include_bytes!("../tests/fixtures/tone.wav").len() as u64;
        assert_eq!(space.new_bytes, 0);
        assert_eq!(space.comparison_bytes, largest);
        // Simulate a nearly full destination: one comparison fits, their sum does not.
        let free = required(largest, resumed.entries.len()).unwrap();
        assert!(
            ensure(
                required(space.bytes().unwrap(), resumed.entries.len()).unwrap(),
                free
            )
            .is_ok()
        );
        assert!(ensure(required(all_audio, resumed.entries.len()).unwrap(), free).is_err());
        let result = f.run(&resumed, true);
        assert_eq!(result.reused, 3);
        assert_eq!(result.copied, 0);
        for entry in &resumed.entries {
            assert_eq!(
                fs::read(&entry.source).unwrap(),
                fs::read(entry.destination.as_ref().unwrap()).unwrap()
            );
        }
    }

    #[test]
    fn mixed_resume_counts_new_files_and_rechecks_missing_outputs() {
        let f = Fixture::new();
        f.run(&f.plan(false), false);
        fs::write(f.0.join("in/new.txt"), vec![7; 10_000]).unwrap();
        fs::copy(f.0.join("in/song.flac"), f.0.join("in/z-duplicate.flac")).unwrap();
        let plan = f.plan(true);
        let space = estimate(&plan, true).unwrap();
        let flac = include_bytes!("../tests/fixtures/tone.flac").len() as u64;
        let wav = include_bytes!("../tests/fixtures/tone.wav").len() as u64;
        assert_eq!(space.new_bytes, 10_000);
        assert_eq!(space.comparison_bytes, flac);
        assert_eq!(space.bytes().unwrap(), 10_000 + flac);
        // A destination removed after planning must be counted as a new copy.
        let destination = plan
            .entries
            .iter()
            .find(|e| e.source.ends_with("song.wav"))
            .unwrap()
            .destination
            .as_ref()
            .unwrap();
        fs::remove_file(destination).unwrap();
        let memo = plan
            .entries
            .iter()
            .find(|e| e.source.ends_with("memo.txt"))
            .unwrap();
        fs::remove_file(memo.destination.as_ref().unwrap()).unwrap();
        let missing = estimate(&plan, true).unwrap();
        assert_eq!(
            missing.new_bytes,
            10_000 + wav + b"original notes".len() as u64
        );
        assert_eq!(missing.comparison_bytes, flac);
        // Problem fallbacks need persistent copy space even when a destination exists.
        let mut problem_plan = f.plan(true);
        let tagged = problem_plan
            .entries
            .iter_mut()
            .find(|e| e.source.ends_with("song.flac"))
            .unwrap();
        tagged.notes.push("PROBLEM: output requires review".into());
        let problem = estimate(&problem_plan, true).unwrap();
        assert_eq!(problem.new_bytes, missing.new_bytes + flac);
        assert_eq!(problem.comparison_bytes, 0);
        let result = f.run(&plan, true);
        assert_eq!(result.copied, 3);
        assert_eq!(result.reused, 1);
        assert_eq!(result.duplicates, 1);
    }
    #[test]
    fn space_threshold_and_overflow_fail_safely() {
        let needed = required(10_000, 2).unwrap();
        assert_eq!(needed, 10_000 + 100 + RESERVE + 32768);
        assert!(ensure(needed, needed).is_ok());
        assert_eq!(
            ensure(needed, needed - 1).unwrap_err().kind(),
            io::ErrorKind::StorageFull
        );
        assert!(required(u64::MAX, 2).is_err());
    }
}
