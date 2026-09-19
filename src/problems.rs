//! Per-source quarantine destinations, retaining causes in the plan and sidecar.
use crate::plan::{Action, BuildPlan, PlanEntry};
use std::path::{Path, PathBuf};

pub fn reasons(entry: &PlanEntry) -> Vec<String> {
    entry
        .notes
        .iter()
        .filter_map(|n| n.strip_prefix("PROBLEM: ").map(str::to_owned))
        .collect()
}
pub fn destination(source: &Path, output: &Path, class: &str) -> PathBuf {
    let id = blake3::hash(source.as_os_str().as_encoded_bytes()).to_hex();
    let extension = source
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| s.len() <= 12 && s.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    let mut short = source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    while short.len() > 32 {
        short.pop();
    }
    let short = alb::naming::sanitize_component(&short)
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|_| "file".into());
    output
        .join("Problem Files")
        .join(class)
        .join(format!("{short} [{id}]{extension}"))
}
pub fn mark(entry: &mut PlanEntry, output: &Path, class: &str, causes: Vec<String>) {
    if reasons(entry).is_empty() {
        entry.notes.push(format!(
            "PROBLEM: Original proposed destination: {:?}",
            entry.destination
        ));
    }
    for cause in causes {
        entry.notes.push(format!("PROBLEM: {cause}"));
    }
    entry.destination = Some(destination(&entry.source, output, class));
    entry.issues.clear();
    entry.action = Action::Keep;
}
pub fn route(plan: &mut BuildPlan, output: &Path) {
    for entry in &mut plan.entries {
        let mut causes = entry.issues.clone();
        causes.extend(
            entry
                .notes
                .iter()
                .filter(|n| {
                    n.starts_with("metadata/read error:")
                        || n.starts_with("missing positive track number")
                        || (n.starts_with("fallback path")
                            && entry.file_type != crate::candidates::FileType::Unknown)
                })
                .cloned(),
        );
        if causes.is_empty() {
            continue;
        }
        let text = causes.join(" ");
        let class = if text.contains("hash") || text.contains("source changed") {
            "Read Errors"
        } else if text.contains("budget") || text.contains("too long") {
            "Path Too Long"
        } else if text.contains("metadata/read error") {
            "Metadata Errors"
        } else if text.contains("missing")
            || text.contains("fallback")
            || text.contains("disc number")
        {
            "Missing Metadata"
        } else {
            "Destination Conflicts"
        };
        mark(entry, output, class, causes);
    }
    // A duplicate cannot rely on the former normal destination of a problem source.
    let targets: std::collections::BTreeMap<_, _> = plan
        .entries
        .iter()
        .map(|e| {
            (
                e.source.clone(),
                (e.destination.clone(), !reasons(e).is_empty()),
            )
        })
        .collect();
    for entry in &mut plan.entries {
        if let Action::DuplicateOf(source) = &entry.action
            && let Some((destination, problem)) = targets.get(source)
        {
            if *problem {
                mark(
                    entry,
                    output,
                    "Duplicate Problems",
                    vec![format!(
                        "Representative {:?} has a file problem; preserve this source independently.",
                        source
                    )],
                );
            } else {
                entry.destination = destination.clone();
            }
        }
    }
}
pub fn description(entry: &PlanEntry, outcome: &str) -> String {
    let timestamps = format!(
        "Original modified time (UTC): {}\nOriginal creation time (UTC): {}\nLinux destination creation time cannot be restored; this is the original source record.\n",
        crate::source::timestamp(entry.source_stamp.as_ref().map(|s| s.modified)),
        crate::source::timestamp(entry.source_stamp.as_ref().and_then(|s| s.created))
    );
    format!(
        "ALB problem file\nSource: {:?}\nDestination: {:?}\nOutcome: {outcome}\n{timestamps}\n{}\n\nThe source was not modified. Review its metadata or the reported filesystem error.\n",
        entry.source,
        entry.destination,
        reasons(entry).join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_paths_and_missing_artist_route_without_losing_the_reason() {
        for (reason, class) in [
            ("missing artist", "Missing Metadata"),
            (
                "proposed full path exceeds the conservative 240-byte budget",
                "Path Too Long",
            ),
        ] {
            let source = PathBuf::from(format!("/input/{}.m4a", "long name".repeat(24)));
            let mut plan = BuildPlan::default();
            plan.entries.push(PlanEntry {
                source: source.clone(),
                file_type: crate::candidates::FileType::M4a,
                source_stamp: None,
                digest: Some([1; 32]),
                action: Action::Blocked,
                collision: false,
                destination: None,
                issues: vec![reason.into()],
                sanitized: false,
                notes: vec![],
            });
            route(&mut plan, Path::new("/output"));
            let entry = &plan.entries[0];
            assert_eq!(entry.action, Action::Keep);
            assert_eq!(plan.unresolved(), 0);
            assert!(
                entry
                    .destination
                    .as_ref()
                    .unwrap()
                    .starts_with(Path::new("/output/Problem Files").join(class))
            );
            assert!(entry.destination.as_ref().unwrap().as_os_str().len() < 240);
            let text = description(entry, "test");
            assert!(text.contains(reason));
            assert!(text.contains(source.to_str().unwrap()));
        }
    }
}
