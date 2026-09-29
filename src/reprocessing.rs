//! Recognize previous ALB output without treating arbitrary text files as reports.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const BUILD_HEADER: &[u8] = b"ALB BUILD REPORT v1\nSTARTED\n";
const PROBLEM_HEADER: &[u8] = b"ALB problem file\n";

fn starts_with(path: &Path, header: &[u8]) -> bool {
    let Ok(mut file) = crate::platform::open_snapshot(path) else {
        return false;
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return false;
    }
    let mut bytes = vec![0; header.len()];
    file.read_exact(&mut bytes).is_ok() && bytes == header
}

pub fn is_library(root: &Path) -> bool {
    let reports = root.join("_ALB");
    if !fs::symlink_metadata(&reports).is_ok_and(|m| m.is_dir() && !crate::platform::is_link(&m)) {
        return false;
    }
    fs::read_dir(reports).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("build-")
                && name.ends_with(".txt")
                && starts_with(&entry.path(), BUILD_HEADER)
        })
    })
}

/// Earlier releases added UNKNOWN wrappers on each pass through the library.
pub fn unknown_relative<'a>(source: &'a Path, input: &Path, library: bool) -> Option<&'a Path> {
    let mut relative = source.strip_prefix(input).ok()?;
    if library {
        while let Ok(inner) = relative.strip_prefix("UNKNOWN") {
            if inner.as_os_str().is_empty() {
                break;
            }
            relative = inner;
        }
    }
    Some(relative)
}

pub fn report_relative(source: &Path, input: &Path, library: bool) -> Option<PathBuf> {
    if !library || source.extension()? != "txt" {
        return None;
    }
    let relative = unknown_relative(source, input, true)?;
    let archive = Path::new("_ALB/Previous Reports");
    if relative.starts_with(archive) {
        return (starts_with(source, BUILD_HEADER) || starts_with(source, PROBLEM_HEADER))
            .then(|| relative.to_owned());
    }
    if relative.starts_with("_ALB") && starts_with(source, BUILD_HEADER) {
        return Some(archive.join(relative.strip_prefix("_ALB").ok()?));
    }
    let problem = relative.strip_prefix("Problem Files").ok()?;
    let category = problem.components().next()?.as_os_str().to_str()?;
    if crate::problems::is_category(category) && starts_with(source, PROBLEM_HEADER) {
        return Some(archive.join(relative));
    }
    None
}
