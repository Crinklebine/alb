//! Conservative free-space estimates; never a reservation or quota guarantee.
use crate::{
    plan::{Action, BuildPlan},
    safe_fs::Directory,
};
use std::{io, path::Path};
const RESERVE: u64 = 16 * 1024 * 1024;
pub fn available(directory: &Directory) -> io::Result<u64> {
    let stat = rustix::fs::fstatvfs(&directory.file)?;
    stat.f_bavail
        .checked_mul(stat.f_frsize)
        .ok_or_else(|| io::Error::other("available space overflow"))
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
pub fn check(plan: &BuildPlan, output: &Path, resume: bool) -> io::Result<()> {
    let mut ancestor = output;
    while !ancestor.try_exists()? {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| io::Error::other("no existing output ancestor"))?;
    }
    let directory = Directory::absolute(ancestor, false, None)?;
    let bytes = plan
        .entries
        .iter()
        .filter(|entry| !matches!(entry.action, Action::DuplicateOf(_)))
        .filter(|entry| {
            !(resume
                && entry
                    .notes
                    .iter()
                    .any(|n| n.starts_with("resume: existing bytes verified"))
                && crate::problems::reasons(entry).is_empty())
        })
        .try_fold(0u64, |sum, entry| {
            sum.checked_add(entry.source_stamp.as_ref().map_or(0, |s| s.len))
                .ok_or_else(|| io::Error::other("copy size overflow"))
        })?;
    let needed = required(bytes, plan.entries.len())?;
    let free = available(&directory)?;
    eprintln!(
        "Space check: {:.2} GiB planned; {:.2} GiB needed including allowance; {:.2} GiB available.",
        bytes as f64 / 1_073_741_824.0,
        needed as f64 / 1_073_741_824.0,
        free as f64 / 1_073_741_824.0
    );
    ensure(needed, free)
}
#[cfg(test)]
mod tests {
    use super::*;
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
