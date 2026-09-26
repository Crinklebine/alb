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
    let root = output.join("Problem Files").join(class);
    let name = source.file_name().unwrap_or_default().to_string_lossy();
    let budget = 180.min(220usize.saturating_sub(root.as_os_str().as_encoded_bytes().len() + 1));
    root.join(short_name(&name, budget))
}
fn short_name(name: &str, budget: usize) -> String {
    let path = Path::new(name);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 12 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let mut raw = name.to_owned();
    // Sanitization rejects overlong components, so shorten before sanitizing.
    if raw.len() > budget {
        raw = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let stem_budget = budget.saturating_sub(extension.len());
        while raw.len() > stem_budget {
            raw.pop();
        }
        raw.push_str(&extension);
    }
    alb::naming::sanitize_component(&raw)
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|_| format!("file{extension}"))
}

fn sidecar_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".txt");
    PathBuf::from(name)
}

pub fn collision_destination(path: &Path, source: &Path, attempt: usize) -> PathBuf {
    let id = blake3::hash(source.as_os_str().as_encoded_bytes()).to_hex();
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    let suffix = format!(" [{}-{attempt}]{extension}", &id[..12]);
    let budget = 180.min(
        220usize.saturating_sub(
            path.parent()
                .unwrap_or(Path::new(""))
                .as_os_str()
                .as_encoded_bytes()
                .len()
                + 1,
        ),
    );
    let mut stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    while !stem.is_empty() && stem.len() + suffix.len() > budget {
        stem.pop();
    }
    path.with_file_name(format!("{stem}{suffix}"))
}

pub fn mark(entry: &mut PlanEntry, input: &Path, output: &Path, class: &str, causes: Vec<String>) {
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
    preserve_layout(entry, input, output, class);
}
// Preserve folders below the input root; never reproduce absolute source paths.
fn preserve_layout(entry: &mut PlanEntry, input: &Path, output: &Path, class: &str) {
    let nested = (|| -> Option<PathBuf> {
        let relative = entry.source.strip_prefix(input).ok()?;
        let mut nested = output.join("Problem Files").join(class);
        for component in relative.parent()?.components() {
            let std::path::Component::Normal(name) = component else {
                return None;
            };
            let clean = alb::naming::sanitize_component(&name.to_string_lossy()).ok()?;
            nested.push(clean.as_str());
        }
        let name = entry.source.file_name()?.to_string_lossy();
        let budget =
            180.min(220usize.checked_sub(nested.as_os_str().as_encoded_bytes().len() + 1)?);
        if budget < 16 {
            return None;
        }
        nested.push(short_name(&name, budget));
        Some(nested)
    })();
    match nested {
        Some(path) => entry.destination = Some(path),
        None => entry.notes.push("PROBLEM: Source directory layout could not be retained within safe path limits; original source path is recorded here.".into()),
    }
}
pub fn route(plan: &mut BuildPlan, input: &Path, output: &Path) {
    let inputs = plan.input_roots.clone();
    for entry in &mut plan.entries {
        let input = crate::paths::source_root(&entry.source, &inputs).unwrap_or(input);
        let mut causes = entry.issues.clone();
        causes.extend(
            entry
                .notes
                .iter()
                .filter(|n| {
                    n.starts_with("metadata/read error:")
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
        mark(entry, input, output, class, causes);
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
        let input = crate::paths::source_root(&entry.source, &inputs).unwrap_or(input);
        if let Action::DuplicateOf(source) = &entry.action
            && let Some((destination, problem)) = targets.get(source)
        {
            if *problem {
                mark(
                    entry,
                    input,
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
    // Reuse one spelling for case/Unicode aliases, as the normal planner does.
    use unicode_normalization::UnicodeNormalization;
    let mut folders = std::collections::BTreeMap::new();
    for entry in &mut plan.entries {
        if reasons(entry).is_empty() {
            continue;
        }
        let Some(destination) = entry.destination.as_ref() else {
            continue;
        };
        let Ok(relative) = destination.strip_prefix(output) else {
            continue;
        };
        let mut chosen = output.to_path_buf();
        for component in relative.parent().into_iter().flat_map(Path::components) {
            chosen.push(component);
            let key = chosen
                .to_string_lossy()
                .to_lowercase()
                .nfc()
                .collect::<String>();
            chosen = folders.entry(key).or_insert_with(|| chosen.clone()).clone();
        }
        if let Some(name) = destination.file_name() {
            chosen.push(name);
        }
        entry.destination = Some(chosen);
    }
    // Preserve the source name unless another planned file (or sidecar) needs it.
    let key = |p: &Path| p.to_string_lossy().to_lowercase().nfc().collect::<String>();
    let mut reserved: std::collections::BTreeSet<_> = plan
        .entries
        .iter()
        .filter_map(|e| e.destination.as_ref().map(|p| key(p)))
        .collect();
    let mut used = std::collections::BTreeSet::new();
    for entry in &mut plan.entries {
        if reasons(entry).is_empty() {
            continue;
        }
        let Some(original) = entry.destination.clone() else {
            continue;
        };
        let mut chosen = original.clone();
        let mut attempt = 0;
        while used.contains(&key(&chosen)) || reserved.contains(&key(&sidecar_path(&chosen))) {
            loop {
                chosen = collision_destination(&original, &entry.source, attempt);
                attempt += 1;
                if !reserved.contains(&key(&chosen))
                    && !used.contains(&key(&chosen))
                    && !reserved.contains(&key(&sidecar_path(&chosen)))
                    && !used.contains(&key(&sidecar_path(&chosen)))
                {
                    break;
                }
            }
        }
        used.insert(key(&chosen));
        let mut sidecar = chosen.as_os_str().to_os_string();
        sidecar.push(".txt");
        used.insert(key(Path::new(&sidecar)));
        reserved.insert(key(&chosen));
        entry.destination = Some(chosen);
    }
}

pub fn description(entry: &PlanEntry, outcome: &str) -> String {
    let timestamps = format!(
        "Original modified time (UTC): {}\nOriginal creation time (UTC): {}\n{}\n",
        crate::source::timestamp(entry.source_stamp.as_ref().map(|s| s.modified)),
        crate::source::timestamp(entry.source_stamp.as_ref().and_then(|s| s.created)),
        crate::platform::CREATION_POLICY
    );
    format!(
        "ALB problem file\nSource: {:?}\nDestination: {:?}\nOutcome: {outcome}\n{timestamps}\n{}\n\nThe source was not modified. Review its metadata or the reported filesystem error.\n",
        entry.source,
        entry.destination,
        entry
            .notes
            .iter()
            .filter(|n| n.starts_with("PROBLEM: ") || n.starts_with("AcoustID: "))
            .map(|n| n.strip_prefix("PROBLEM: ").unwrap_or(n))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planned_names_and_sidecars_do_not_collide() {
        let mut plan = BuildPlan::default();
        for name in ["song.mp3", "SONG.mp3", "song.mp3.txt", "other/song.mp3"] {
            plan.entries.push(PlanEntry {
                metadata_update: None,
                output_evidence: None,
                fingerprint_root: None,
                source: Path::new("input").join(name),
                file_type: crate::candidates::FileType::Mp3,
                source_stamp: None,
                digest: None,
                action: Action::Blocked,
                collision: false,
                destination: None,
                issues: vec!["missing artist".into()],
                sanitized: false,
                notes: vec![],
            });
        }
        route(&mut plan, Path::new("input"), Path::new("output"));
        let mut names = std::collections::BTreeSet::new();
        for entry in &plan.entries {
            let path = entry.destination.as_ref().unwrap();
            assert!(names.insert(path.to_string_lossy().to_lowercase()));
            assert!(names.insert(sidecar_path(path).to_string_lossy().to_lowercase()));
        }
        assert_eq!(
            plan.entries[3].destination.as_ref().unwrap(),
            Path::new("output/Problem Files/Missing Metadata/other/song.mp3")
        );
    }

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
                metadata_update: None,
                output_evidence: None,
                fingerprint_root: None,
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
            route(&mut plan, Path::new("/input"), Path::new("/output"));
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
            let mut deep = entry.clone();
            deep.source = Path::new("/input")
                .join("folder".repeat(35))
                .join("song.m4a");
            deep.destination = Some(destination(&deep.source, Path::new("/output"), class));
            preserve_layout(&mut deep, Path::new("/input"), Path::new("/output"), class);
            assert_eq!(
                deep.destination,
                Some(destination(&deep.source, Path::new("/output"), class))
            );
            assert!(description(&deep, "test").contains("layout could not be retained"));
        }
    }
}
