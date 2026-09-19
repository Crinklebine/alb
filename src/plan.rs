//! Advisory keep/duplicate decisions only. No execution or filesystem writes.
use crate::{
    candidates::{FileType, classify},
    catalog::{Track, TrackCatalog},
    discovery::Catalog,
    hashing::HashCatalog,
    source::SourceStamp,
};
use alb::naming::sanitize_component;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Pending,
    Keep,
    DuplicateOf(PathBuf),
    Blocked,
}

#[derive(Debug, Clone)]
pub struct PlanEntry {
    pub source: PathBuf,
    pub file_type: FileType,
    pub source_stamp: Option<SourceStamp>,
    pub digest: Option<[u8; 32]>,
    pub action: Action,
    pub collision: bool,
    pub destination: Option<PathBuf>,
    pub issues: Vec<String>,
    pub sanitized: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Default)]
pub struct BuildPlan {
    pub entries: Vec<PlanEntry>,
    pub discovery_errors: Vec<String>,
    pub output_checked: bool,
    pub metadata_warnings: usize,
}

impl BuildPlan {
    pub fn unresolved(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| !entry.issues.is_empty())
            .count()
    }
}

fn required<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing {field}"))
}

fn album(track: &Track) -> Option<&str> {
    track
        .album
        .as_deref()
        .filter(|value| !value.trim().is_empty())
}

fn folder_artist(track: &Track) -> Option<&str> {
    if album(track).is_some() {
        track
            .album_artist
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or(track.artist.as_deref())
    } else {
        track.artist.as_deref()
    }
}

fn destination(track: &Track, output: &Path) -> Result<(PathBuf, String, bool), String> {
    let artist = required(track.artist.as_deref(), "artist")?;
    let album = album(track);
    let title = required(track.title.as_deref(), "title")?;
    // Missing album means loose track, not a claim about a commercial single.
    // Track/disc numbering only applies within a known album.
    let prefix = match (album, track.track_number.filter(|n| *n > 0)) {
        (Some(_), Some(number)) => match track.disc_number {
            Some(0) => return Err("disc number must be positive when supplied".into()),
            Some(disc) => format!("{disc:02}-{number:02} - "),
            None => format!("{number:02} - "),
        },
        _ => String::new(),
    };
    let album_artist = folder_artist(track).unwrap_or(artist);
    let clean = |raw: &str| sanitize_component(raw).map_err(|error| error.to_string());
    let artist_name = clean(artist)?;
    let title_name = clean(title)?;
    let folder = clean(album_artist)?;
    let album_name = clean(album.unwrap_or("Loose Tracks"))?;
    let filename = clean(&format!(
        "{prefix}{} - {}.{}",
        title_name.as_str(),
        artist_name.as_str(),
        track.file_type.extension()
    ))?;
    let group = track.file_type.group();
    let relative: PathBuf = [
        group,
        folder.as_str(),
        album_name.as_str(),
        filename.as_str(),
    ]
    .iter()
    .collect();
    let full = output.join(relative);
    // Conservative preview budget; not a promise of portability on every OS.
    if full.as_os_str().as_encoded_bytes().len() > 240 {
        return Err("proposed full path exceeds the conservative 240-byte budget".into());
    }
    let key = format!(
        "{group}/{}/{}/{}",
        folder.collision_key(),
        album_name.collision_key(),
        filename.collision_key()
    );
    let changed = [artist_name, title_name, folder, album_name, filename]
        .iter()
        .any(|name| name.changed);
    Ok((full, key, changed))
}

/// Preserve every discovered file, including unknowns and inspection failures.
/// Hash evidence enables advisory dedupe before destination collision checks.
pub fn generate(
    discovery: &Catalog,
    inspection: &TrackCatalog,
    input: &Path,
    output: &Path,
    hashes: Option<&HashCatalog>,
) -> BuildPlan {
    let tracks: BTreeMap<_, _> = inspection
        .tracks
        .iter()
        .map(|track| (&track.source_path, track))
        .collect();
    let failures: BTreeMap<_, _> = inspection
        .errors
        .iter()
        .map(|(path, error)| (path, error))
        .collect();
    let mut plan = BuildPlan {
        metadata_warnings: inspection.errors.len(),
        ..BuildPlan::default()
    };
    let mut keys: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut directories: BTreeMap<String, BTreeMap<Vec<String>, Vec<usize>>> = BTreeMap::new();
    let mut files = discovery.files.clone();
    files.sort();
    for source in files {
        let mut entry = PlanEntry {
            source: source.clone(),
            file_type: classify(&source),
            source_stamp: tracks.get(&source).and_then(|t| t.source_stamp.clone()),
            digest: None,
            action: Action::Pending,
            collision: false,
            destination: None,
            issues: Vec::new(),
            sanitized: false,
            notes: Vec::new(),
        };
        if let Some(track) = tracks.get(&source)
            && track.file_type != FileType::Unknown
            && (track.artist.is_some() || track.album.is_some() || track.title.is_some())
        {
            match destination(track, output) {
                Ok((destination, key, changed)) => {
                    entry.destination = Some(destination);
                    entry.sanitized = changed;
                    if album(track).is_some() && track.track_number.filter(|n| *n > 0).is_none() {
                        entry.notes.push("missing positive track number; retained album folder and omitted filename numbering".into());
                    }
                    let raw_artist = folder_artist(track).unwrap_or("");
                    // Canonicalize all absent/blank album spellings, but keep a
                    // real album called Loose Tracks distinct for alias reporting.
                    let raw_album = album(track).unwrap_or("");
                    if album(track).is_none() {
                        entry.notes.push("album absent; organized as a loose track (not a single classification)".into());
                    }
                    let key_parts: Vec<_> = key.split('/').collect();
                    for (depth, raw) in [
                        (2, vec![raw_artist.to_owned()]),
                        (3, vec![raw_artist.to_owned(), raw_album.to_owned()]),
                    ] {
                        directories
                            .entry(key_parts[..depth].join("/"))
                            .or_default()
                            .entry(raw)
                            .or_default()
                            .push(plan.entries.len());
                    }
                    keys.entry(key).or_default().push(plan.entries.len());
                }
                Err(error) => entry.issues.push(error),
            }
        }
        if let Some(error) = failures.get(&source) {
            entry
                .notes
                .push(format!("metadata/read error: {error}; source retained"));
        }
        if entry.destination.is_none() {
            match fallback(&source, input, output) {
                Ok((destination, key, changed)) => {
                    entry.destination = Some(destination);
                    entry.sanitized = changed;
                    entry
                        .notes
                        .push("fallback path preserves source-relative layout".into());
                    keys.entry(key).or_default().push(plan.entries.len());
                }
                Err(error) => entry.issues.push(error),
            }
        }
        plan.entries.push(entry);
    }
    if let Some(hashes) = hashes {
        attach_hashes(&mut plan, hashes);
    }
    for indices in keys.values() {
        let active: Vec<_> = indices
            .iter()
            .copied()
            .filter(|&i| !matches!(plan.entries[i].action, Action::DuplicateOf(_)))
            .collect();
        if active.len() < 2 {
            continue;
        }
        for &index in &active {
            plan.entries[index].collision = true;
            plan.entries[index].issues.push(format!(
                "destination collision involving {} non-duplicate or unverified source files; no winner selected", active.len()
            ));
        }
    }
    for variants in directories.values().filter(|variants| variants.len() > 1) {
        for indices in variants.values() {
            for &index in indices {
                let issue =
                    "normalized directory alias: different metadata maps to the same folder"
                        .to_owned();
                if !plan.entries[index].issues.contains(&issue) {
                    plan.entries[index].issues.push(issue);
                }
            }
        }
    }
    if hashes.is_some() {
        resolve_naming_conflicts(&mut plan, output);
    }
    finalize_actions(&mut plan);
    plan.discovery_errors = discovery
        .errors
        .iter()
        .map(|error| format!("{:?}: {}", error.path, error.source))
        .collect();
    plan.discovery_errors.sort();
    plan
}

fn attach_hashes(plan: &mut BuildPlan, hashes: &HashCatalog) {
    let failures: BTreeMap<_, _> = hashes.errors.iter().map(|(p, e)| (p, e)).collect();
    let mut groups: BTreeMap<(FileType, [u8; 32]), Vec<usize>> = BTreeMap::new();
    for (index, entry) in plan.entries.iter_mut().enumerate() {
        if let Some(error) = failures.get(&entry.source) {
            entry.issues.push(format!("hash failed: {error}"));
            continue;
        }
        let Some(evidence) = hashes.files.get(&entry.source) else {
            entry
                .issues
                .push("missing hash evidence; rescan required".into());
            continue;
        };
        if evidence.file_type != entry.file_type
            || entry.source_stamp.as_ref() != Some(&evidence.source_stamp)
            || SourceStamp::at(&entry.source).ok().as_ref() != Some(&evidence.source_stamp)
        {
            entry
                .issues
                .push("stale or mismatched hash/source evidence; rescan required".into());
            continue;
        }
        entry.digest = Some(evidence.digest);
        entry.action = Action::Keep;
        // A source with unresolved naming issues cannot represent another file.
        if entry.issues.is_empty()
            && entry.destination.is_some()
            && entry.file_type != FileType::Unknown
        {
            groups
                .entry((entry.file_type, evidence.digest))
                .or_default()
                .push(index);
        }
    }
    for indices in groups.values() {
        let representative = indices[0]; // Plan entries are in native source-path order.
        let source = plan.entries[representative].source.clone();
        let destination = plan.entries[representative].destination.clone();
        for &index in &indices[1..] {
            plan.entries[index].action = Action::DuplicateOf(source.clone());
            if plan.entries[index].destination != destination {
                let original = plan.entries[index].destination.clone();
                plan.entries[index]
                    .notes
                    .retain(|note| note != "fallback path preserves source-relative layout");
                plan.entries[index]
                    .notes
                    .push(format!("original candidate destination: {original:?}"));
            }
            plan.entries[index].destination = destination.clone();
            plan.entries[index].notes.push("exact whole-file BLAKE3 match within file type; first eligible source path retained".into());
        }
    }
}

/// Resolve planned names only; existing output remains subject to no-clobber checks.
fn resolve_naming_conflicts(plan: &mut BuildPlan, output: &Path) {
    use unicode_normalization::UnicodeNormalization;
    let key = |p: &Path| p.to_string_lossy().to_lowercase().nfc().collect::<String>();
    let mut folders = BTreeMap::<String, PathBuf>::new();
    // Entries are source-sorted, so the first spelling is reproducible.
    for entry in &mut plan.entries {
        if entry.digest.is_none() {
            continue;
        }
        let Some(destination) = entry.destination.clone() else {
            continue;
        };
        let Ok(relative) = destination.strip_prefix(output) else {
            continue;
        };
        let Some(parent) = relative.parent() else {
            continue;
        };
        let mut chosen = output.to_path_buf();
        for component in parent.components() {
            chosen.push(component);
            chosen = folders
                .entry(key(&chosen))
                .or_insert_with(|| chosen.clone())
                .clone();
        }
        chosen.push(destination.file_name().unwrap());
        if chosen != destination {
            entry.notes.push(format!(
                "folder spelling unified: {destination:?} -> {chosen:?}"
            ));
        }
        if entry
            .issues
            .iter()
            .any(|issue| issue.starts_with("normalized directory alias:"))
        {
            entry.notes.push(format!(
                "directory alias resolved using folder {:?}",
                chosen.parent()
            ));
        }
        entry.destination = Some(chosen);
        entry.issues.retain(|issue| {
            !issue.starts_with("normalized directory alias:")
                && !issue.starts_with("destination collision involving")
        });
    }
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for (i, entry) in plan.entries.iter().enumerate() {
        if !matches!(entry.action, Action::DuplicateOf(_))
            && let Some(destination) = &entry.destination
        {
            groups.entry(key(destination)).or_default().push(i);
        }
    }
    let mut reserved: std::collections::BTreeSet<String> = groups.keys().cloned().collect();
    for indices in groups.values().filter(|indices| indices.len() > 1) {
        let sources: Vec<_> = indices
            .iter()
            .map(|&i| plan.entries[i].source.clone())
            .collect();
        for &i in indices {
            let entry = &mut plan.entries[i];
            if entry.digest.is_none() {
                continue;
            }
            let original = entry.destination.as_ref().unwrap().clone();
            let id = blake3::hash(entry.source.as_os_str().as_encoded_bytes()).to_hex();
            let extension = original.extension().unwrap_or_default().to_string_lossy();
            let stem = original.file_stem().unwrap_or_default().to_string_lossy();
            let mut attempt = 0;
            loop {
                let suffix = format!(" [{}-{attempt}]", &id[..12]);
                let tail = if extension.is_empty() {
                    suffix
                } else {
                    format!("{suffix}.{extension}")
                };
                let budget = 180.min(
                    240usize.saturating_sub(
                        original
                            .parent()
                            .unwrap()
                            .as_os_str()
                            .as_encoded_bytes()
                            .len()
                            + 1,
                    ),
                );
                if tail.len() >= budget {
                    entry
                        .issues
                        .push("cannot fit collision suffix within path budget".into());
                    break;
                }
                let mut short = stem.to_string();
                while short.len() + tail.len() > budget {
                    short.pop();
                }
                let destination = original.with_file_name(format!("{short}{tail}"));
                if reserved.insert(key(&destination)) {
                    if destination.as_os_str().as_encoded_bytes().len() > 240 {
                        entry.issues.push(
                            "resolved collision path exceeds conservative 240-byte budget".into(),
                        );
                    }
                    entry.notes.push(format!("collision resolved; preserve all sources {sources:?}; original {original:?}; chosen {destination:?}"));
                    entry.destination = Some(destination);
                    break;
                }
                attempt += 1;
            }
        }
    }
    let destinations: BTreeMap<_, _> = plan
        .entries
        .iter()
        .filter(|e| e.action == Action::Keep)
        .map(|e| (e.source.clone(), e.destination.clone()))
        .collect();
    for entry in &mut plan.entries {
        if let Action::DuplicateOf(source) = &entry.action
            && let Some(destination) = destinations.get(source)
        {
            entry.destination = destination.clone();
        }
    }
}

fn finalize_actions(plan: &mut BuildPlan) {
    let keep: std::collections::BTreeSet<_> = plan
        .entries
        .iter()
        .filter(|e| e.action == Action::Keep && e.issues.is_empty() && e.destination.is_some())
        .map(|e| e.source.clone())
        .collect();
    for entry in &mut plan.entries {
        if let Action::DuplicateOf(source) = &entry.action
            && !keep.contains(source)
        {
            entry
                .issues
                .push("duplicate representative is blocked; no omission authorized".into());
        }
        if !entry.issues.is_empty() {
            entry.action = Action::Blocked;
        }
    }
}

/// Fallbacks retain relative directories so unrelated untagged files are not merged.
fn fallback(source: &Path, input: &Path, output: &Path) -> Result<(PathBuf, String, bool), String> {
    use unicode_normalization::UnicodeNormalization;
    let relative = source
        .strip_prefix(input)
        .map_err(|_| "source outside input root")?;
    let kind = classify(source);
    let mut path = PathBuf::from(kind.group());
    if kind != FileType::Unknown {
        path.push("_Unsorted");
    }
    let mut changed = false;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("unsafe relative source path".into());
        };
        if let Some(name) = name.to_str() {
            let clean = sanitize_component(name).map_err(|e| e.to_string())?;
            changed |= clean.changed;
            path.push(clean.as_str());
        } else {
            path.push(name); // Preserve native non-Unicode filenames; never discard them.
        }
    }
    if relative.as_os_str().is_empty() {
        return Err("empty source-relative path".into());
    }
    let key = path
        .components()
        .map(|c| {
            c.as_os_str()
                .to_string_lossy()
                .to_lowercase()
                .nfc()
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/");
    let full = output.join(path);
    if full.as_os_str().as_encoded_bytes().len() > 240 {
        return Err("fallback path exceeds conservative 240-byte budget".into());
    }
    Ok((full, key, changed))
}

/// Read-only snapshot checks. Root must come from canonical path validation.
/// Does not protect against concurrent changes or model every filesystem alias.
#[cfg(test)]
pub fn check_existing_output(plan: &mut BuildPlan) {
    check_existing_output_mode(plan, false);
}

pub fn check_existing_output_mode(plan: &mut BuildPlan, resume: bool) {
    for entry in &mut plan.entries {
        if !matches!(entry.action, Action::DuplicateOf(_))
            && let Some(destination) = &entry.destination
            && let Err(issue) = check_destination(destination, resume)
        {
            entry.issues.push(issue);
        }
    }
    if resume {
        for entry in &mut plan.entries {
            if entry.action == Action::Keep
                && entry.issues.is_empty()
                && entry.destination.as_ref().is_some_and(|p| p.exists())
            {
                #[cfg(target_os = "linux")]
                match crate::execution::verify_existing(entry) {
                    Ok(()) => entry
                        .notes
                        .push("resume: existing bytes verified; execution will recheck".into()),
                    Err(e) => entry
                        .issues
                        .push(format!("resume destination verification failed: {e}")),
                }
                #[cfg(not(target_os = "linux"))]
                entry.issues.push("resume requires Linux".into());
            }
        }
    }
    plan.output_checked = true;
    finalize_actions(plan);
}

fn check_destination(destination: &Path, resume: bool) -> Result<(), String> {
    use unicode_normalization::UnicodeNormalization;
    let key = |name: &std::ffi::OsStr| -> Option<String> {
        name.to_str()
            .map(|name| name.to_lowercase().nfc().collect())
    };
    let mut current = PathBuf::new();
    let parts: Vec<_> = destination.components().collect();
    for (index, component) in parts.iter().enumerate() {
        // Prefix/root components are trusted syntax, not entries to enumerate.
        if !matches!(component, std::path::Component::Normal(_)) {
            current.push(component.as_os_str());
            continue;
        }
        if !current.as_os_str().is_empty() {
            let entries = match fs::read_dir(&current) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => {
                    return Err(format!(
                        "cannot inspect output directory {current:?}: {error}"
                    ));
                }
            };
            let mut children = entries
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<io::Result<Vec<_>>>()
                .map_err(|error| format!("cannot enumerate output {current:?}: {error}"))?;
            children.sort();
            for child in children {
                let name = child
                    .file_name()
                    .ok_or_else(|| format!("invalid output entry {child:?}"))?;
                if name != component.as_os_str()
                    && key(name).is_some()
                    && key(name) == key(component.as_os_str())
                {
                    return Err(format!("existing output name alias: {child:?}"));
                }
            }
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.is_symlink() {
                    return Err(format!("output symlink is not followed: {current:?}"));
                }
                if index == parts.len() - 1 {
                    if resume && metadata.is_file() {
                        return Ok(());
                    }
                    return Err(format!("destination already exists: {current:?}"));
                }
                if !metadata.is_dir() {
                    return Err(format!("output ancestor is not a directory: {current:?}"));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("cannot inspect output {current:?}: {error}")),
        }
    }
    Ok(())
}

pub fn write_plan(writer: &mut impl Write, plan: &BuildPlan) -> io::Result<()> {
    writeln!(
        writer,
        "ADVISORY SORTER PLAN: {} entries, {} unresolved, {} discovery errors.",
        plan.entries.len(),
        plan.unresolved(),
        plan.discovery_errors.len()
    )?;
    let unknown = plan
        .entries
        .iter()
        .filter(|e| e.file_type == FileType::Unknown)
        .count();
    let duplicates = plan
        .entries
        .iter()
        .filter(|e| matches!(e.action, Action::DuplicateOf(_)))
        .count();
    let copies = plan
        .entries
        .iter()
        .filter(|e| e.action == Action::Keep)
        .count();
    let collisions = plan.entries.iter().filter(|e| e.collision).count();
    writeln!(
        writer,
        "Files scanned: {}; Known audio files: {}; Unknown files: {}; Exact duplicates: {}; Planned copies: {}; Collisions (affected files): {}; Metadata warnings: {}.",
        plan.entries.len(),
        plan.entries.len() - unknown,
        unknown,
        duplicates,
        copies,
        collisions,
        plan.metadata_warnings
    )?;
    for entry in &plan.entries {
        writeln!(writer, "SOURCE: {:?}", entry.source)?;
        writeln!(writer, "  ACTION: {:?}", entry.action)?;
        if let Some(digest) = entry.digest {
            writeln!(
                writer,
                "  BLAKE3: {}; source stamp retained: {}",
                blake3::Hash::from(digest).to_hex(),
                entry.source_stamp.is_some()
            )?;
        }
        if let Some(destination) = &entry.destination {
            writeln!(writer, "  PROPOSED: {destination:?}")?;
        }
        if entry.sanitized {
            writeln!(writer, "  NOTE: destination metadata was sanitized.")?;
        }
        for note in &entry.notes {
            writeln!(writer, "  NOTE: {note:?}")?;
        }
        for issue in &entry.issues {
            writeln!(writer, "  UNRESOLVED: {issue:?}")?;
        }
    }
    for error in &plan.discovery_errors {
        writeln!(writer, "DISCOVERY ERROR: {error:?}")?;
    }
    writeln!(
        writer,
        "Existing output snapshot checked: {}.",
        plan.output_checked
    )?;
    writeln!(
        writer,
        "Planning report only. No files were copied while generating this plan."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidates::FileType;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            for n in 0.. {
                let root = std::env::temp_dir()
                    .join(format!("alb-dedupe-plan-{}-{n}", std::process::id()));
                match fs::create_dir(&root) {
                    Ok(()) => {
                        fs::create_dir(root.join("source")).unwrap();
                        return Self(root);
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("{e}"),
                }
            }
            unreachable!()
        }
        fn write(&self, name: &str, bytes: &[u8]) {
            fs::write(self.0.join("source").join(name), bytes).unwrap();
        }
        fn evidence(&self) -> (Catalog, TrackCatalog, HashCatalog) {
            let files = crate::discovery::discover(&self.0.join("source")).unwrap();
            let tracks = crate::inspection::inspect(&files.files);
            let hashes = crate::hashing::analyze(&tracks.tracks);
            (files, tracks, hashes)
        }
        fn plan(&self, files: &Catalog, tracks: &TrackCatalog, hashes: &HashCatalog) -> BuildPlan {
            generate(
                files,
                tracks,
                &self.0.join("source"),
                &self.0.join("output"),
                Some(hashes),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    const TONE: &[u8] = include_bytes!("../tests/fixtures/tone.flac");

    #[test]
    fn plan_dedupe_is_same_type_deterministic_and_preserves_unknowns() {
        let f = Fixture::new();
        for name in ["b.FLAC", "a.flac", "same.mp3", "one.txt", "two.bin"] {
            f.write(name, TONE);
        }
        let (mut files, mut tracks, hashes) = f.evidence();
        let plan = f.plan(&files, &tracks, &hashes);
        assert_eq!(plan.unresolved(), 0);
        assert_eq!(
            plan.entries
                .iter()
                .filter(|e| e.action == Action::Keep)
                .count(),
            4
        );
        assert_eq!(
            plan.entries[1].action,
            Action::DuplicateOf(f.0.join("source/a.flac"))
        );
        assert_eq!(plan.entries[0].destination, plan.entries[1].destination);
        assert!(
            plan.entries
                .iter()
                .all(|e| e.digest.is_some() && e.source_stamp.is_some())
        );
        let mut report = Vec::new();
        write_plan(&mut report, &plan).unwrap();
        files.files.reverse();
        tracks.tracks.reverse();
        let mut reordered = Vec::new();
        write_plan(&mut reordered, &f.plan(&files, &tracks, &hashes)).unwrap();
        assert_eq!(report, reordered);
    }

    #[test]
    fn plan_blocks_stale_missing_failed_and_mismatched_hash_evidence() {
        let f = Fixture::new();
        for name in ["a.flac", "b.flac", "c.flac", "d.flac", "e.flac"] {
            f.write(name, TONE);
        }
        let (files, tracks, mut hashes) = f.evidence();
        f.write("a.flac", b"changed synthetic source");
        hashes.files.remove(&f.0.join("source/b.flac"));
        hashes
            .files
            .get_mut(&f.0.join("source/c.flac"))
            .unwrap()
            .file_type = FileType::Mp3;
        hashes.errors.push((
            f.0.join("source/d.flac"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ));
        // Metadata identity must agree even when the live file matches hash evidence.
        let mut tracks = tracks;
        tracks
            .tracks
            .iter_mut()
            .find(|t| t.source_path.ends_with("e.flac"))
            .unwrap()
            .source_stamp = None;
        let plan = f.plan(&files, &tracks, &hashes);
        assert_eq!(plan.unresolved(), 5);
        assert!(
            plan.entries
                .iter()
                .all(|e| e.action == Action::Blocked && e.digest.is_none())
        );
        assert!(plan.entries[0].issues.iter().any(|s| s.contains("stale")));
        assert!(
            plan.entries[1]
                .issues
                .iter()
                .any(|s| s.contains("missing hash"))
        );
        assert!(
            plan.entries[3]
                .issues
                .iter()
                .any(|s| s.contains("hash failed"))
        );
    }

    #[test]
    fn different_bytes_get_distinct_destinations_and_duplicates_follow_representative() {
        let f = Fixture::new();
        f.write("a.flac", TONE);
        f.write("b.flac", TONE);
        let mut changed = TONE.to_vec();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        f.write("c.flac", &changed); // Same sorting metadata, different whole-file hash.
        let (files, tracks, hashes) = f.evidence();
        let plan = f.plan(&files, &tracks, &hashes);
        assert_eq!(plan.unresolved(), 0);
        assert_eq!(plan.entries.iter().filter(|e| e.collision).count(), 2);
        assert_eq!(plan.entries[0].action, Action::Keep);
        assert_eq!(plan.entries[2].action, Action::Keep);
        assert_eq!(plan.entries[0].destination, plan.entries[1].destination);
        assert_ne!(plan.entries[0].destination, plan.entries[2].destination);
        let repeated = f.plan(&files, &tracks, &hashes);
        assert_eq!(plan.entries[0].destination, repeated.entries[0].destination);
    }

    #[test]
    fn verified_folder_aliases_use_one_spelling_and_preserve_all_files() {
        let f = Fixture::new();
        f.write("a.flac", TONE);
        let mut changed = TONE.to_vec();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        f.write("b.flac", &changed);
        let (files, mut tracks, hashes) = f.evidence();
        tracks.tracks[0].album = Some("Looking In The Shadows".into());
        tracks.tracks[1].album = Some("looking in the shadows".into());
        tracks.tracks[1].track_number = Some(4);
        let plan = f.plan(&files, &tracks, &hashes);
        assert_eq!(plan.unresolved(), 0);
        let a = plan.entries[0].destination.as_ref().unwrap();
        let b = plan.entries[1].destination.as_ref().unwrap();
        assert_eq!(a.parent(), b.parent());
        assert_ne!(a, b);
        assert!(plan.entries.iter().all(|e| e.action == Action::Keep));
        assert!(
            plan.entries[1]
                .notes
                .iter()
                .any(|n| n.contains("directory alias resolved"))
        );
        tracks.tracks.reverse();
        let repeated = f.plan(&files, &tracks, &hashes);
        assert_eq!(plan.entries[1].destination, repeated.entries[1].destination);
    }

    #[test]
    fn occupied_representative_destination_blocks_duplicate_omission() {
        let f = Fixture::new();
        f.write("a.flac", TONE);
        f.write("b.flac", TONE);
        let (files, tracks, hashes) = f.evidence();
        let mut plan = f.plan(&files, &tracks, &hashes);
        let destination = plan.entries[0].destination.as_ref().unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, b"existing output must remain").unwrap();
        check_existing_output(&mut plan);
        assert!(plan.entries.iter().all(|e| e.action == Action::Blocked));
        assert!(
            plan.entries[1]
                .issues
                .iter()
                .any(|s| s.contains("representative is blocked"))
        );
        assert_eq!(
            fs::read(plan.entries[0].destination.as_ref().unwrap()).unwrap(),
            b"existing output must remain"
        );
    }

    fn track(path: &str) -> Track {
        Track {
            source_path: path.into(),
            file_type: FileType::Flac,
            source_stamp: None,
            title: Some("Song".into()),
            artist: Some("Performer".into()),
            album_artist: Some("Various Artists".into()),
            album: Some("Album".into()),
            track_number: Some(3),
            disc_number: Some(2),
            duration: None,
        }
    }
    fn make(tracks: Vec<Track>) -> BuildPlan {
        let discovery = Catalog {
            files: tracks.iter().map(|t| t.source_path.clone()).collect(),
            ..Catalog::default()
        };
        generate(
            &discovery,
            &TrackCatalog {
                tracks,
                errors: Vec::new(),
            },
            Path::new(""),
            Path::new("output"),
            None,
        )
    }
    #[test]
    fn loose_tracks_use_track_artist_and_title_without_numbering() {
        for kind in [
            FileType::Flac,
            FileType::M4a,
            FileType::Mp3,
            FileType::Ogg,
            FileType::Wav,
        ] {
            for missing in [None, Some("".into()), Some(" \t".into())] {
                let mut value = track(&format!("source.{}", kind.extension()));
                value.file_type = kind;
                value.album = missing;
                value.track_number = None;
                value.disc_number = Some(0); // Numbering has no role outside an album.
                let plan = make(vec![value]);
                assert_eq!(plan.unresolved(), 0);
                assert_eq!(
                    plan.entries[0].destination,
                    Some(
                        Path::new("output")
                            .join(kind.group())
                            .join("Performer/Loose Tracks")
                            .join(format!("Song - Performer.{}", kind.extension()))
                    )
                );
                assert!(
                    plan.entries[0]
                        .notes
                        .iter()
                        .any(|s| s.contains("album absent"))
                );
            }
        }
    }

    #[test]
    fn loose_tracks_with_unusable_artist_or_title_retain_relative_fallbacks() {
        for field in ["artist", "title", "unsafe_title"] {
            let mut value = track("nested/source.flac");
            value.album = None;
            match field {
                "artist" => value.artist = Some(" ".into()),
                "title" => value.title = None,
                _ => value.title = Some("...".into()),
            }
            let plan = make(vec![value]);
            assert_eq!(
                plan.entries[0].destination.as_deref(),
                Some(Path::new("output/FLAC/_Unsorted/nested/source.flac"))
            );
            assert_eq!(plan.unresolved(), 1);
        }
    }

    #[test]
    fn loose_track_collisions_and_real_album_aliases_are_reported() {
        let mut a = track("a.flac");
        a.album = None;
        let mut b = track("b.flac");
        b.album = Some(" ".into());
        b.title = Some("Other Song".into());
        assert_eq!(make(vec![a, b]).unresolved(), 0); // Missing and blank album are equivalent.
        let mut a = track("a.flac");
        a.album = None;
        let mut b = track("b.flac");
        b.album = None;
        assert_eq!(make(vec![a, b]).unresolved(), 2); // Same loose destination still collides.
        let mut a = track("a.flac");
        a.album = None;
        let mut b = track("b.flac");
        b.album = Some("Loose Tracks".into());
        b.album_artist = None;
        let plan = make(vec![a, b]);
        assert_eq!(plan.unresolved(), 2);
        assert!(
            plan.entries
                .iter()
                .all(|e| e.issues.iter().any(|s| s.contains("directory alias")))
        );
    }

    #[test]
    fn tagged_destinations_use_generic_file_types() {
        let mut tracks = Vec::new();
        for kind in [
            FileType::Flac,
            FileType::M4a,
            FileType::Mp3,
            FileType::Ogg,
            FileType::Wav,
        ] {
            let mut value = track(&format!("song.{}", kind.extension()));
            value.file_type = kind;
            tracks.push(value);
        }
        let plan = make(tracks);
        assert_eq!(plan.unresolved(), 0);
        assert_eq!(plan.entries.len(), 5);
        for entry in plan.entries {
            let kind = classify(&entry.source);
            let destination = entry.destination.unwrap();
            assert!(destination.starts_with(Path::new("output").join(kind.group())));
            assert_eq!(destination.extension().unwrap(), kind.extension());
        }
    }

    #[test]
    fn unknown_fallbacks_preserve_layout_and_report_name_collisions() {
        let tracks = ["folder/a?.txt", "folder/a*.txt"]
            .iter()
            .map(|name| Track::empty(Path::new(name)))
            .collect();
        let plan = make(tracks);
        assert_eq!(plan.entries.len(), 2);
        assert_eq!(plan.unresolved(), 2);
        for entry in plan.entries {
            assert_eq!(
                entry.destination.unwrap(),
                Path::new("output/UNKNOWN/folder/a_.txt")
            );
            assert!(entry.issues.iter().any(|issue| issue.contains("collision")));
        }
        assert!(
            fallback(
                Path::new("../escape.txt"),
                Path::new(""),
                Path::new("output")
            )
            .is_err()
        );
        assert!(
            fallback(
                Path::new("elsewhere/a.txt"),
                Path::new("input"),
                Path::new("output")
            )
            .is_err()
        );
    }

    #[test]
    fn album_tracks_without_positive_numbers_keep_album_and_warn_for_all_formats() {
        for kind in [
            FileType::Flac,
            FileType::M4a,
            FileType::Mp3,
            FileType::Ogg,
            FileType::Wav,
        ] {
            for number in [None, Some(0)] {
                let mut value = track("source");
                value.file_type = kind;
                value.track_number = number;
                let plan = make(vec![value]);
                assert_eq!(plan.unresolved(), 0);
                assert_eq!(
                    plan.entries[0].destination.as_ref().unwrap(),
                    &PathBuf::from(format!(
                        "output/{}/Various Artists/Album/Song - Performer.{}",
                        kind.group(),
                        kind.extension()
                    ))
                );
                assert!(
                    plan.entries[0]
                        .notes
                        .iter()
                        .any(|n| n.contains("omitted filename numbering"))
                );
            }
        }
    }

    #[test]
    fn uses_album_artist_and_explicit_disc_track() {
        let plan = make(vec![track("source")]);
        assert_eq!(
            plan.entries[0].destination.as_deref(),
            Some(Path::new(
                "output/FLAC/Various Artists/Album/02-03 - Song - Performer.flac"
            ))
        );
        assert_eq!(plan.unresolved(), 0);
    }
    #[test]
    fn falls_back_to_artist_and_rejects_missing_metadata() {
        let mut a = track("a");
        a.album_artist = Some(" ".into());
        a.disc_number = None;
        let mut b = track("b");
        b.title = None;
        let plan = make(vec![b, a]);
        assert_eq!(
            plan.entries[0].destination.as_deref(),
            Some(Path::new(
                "output/FLAC/Performer/Album/03 - Song - Performer.flac"
            ))
        );
        assert!(plan.entries[1].issues[0].contains("missing title"));
    }
    #[test]
    fn blocks_all_colliding_entries_after_sanitization_and_case_changes() {
        let mut a = track("a");
        a.title = Some("a/b".into());
        let mut b = track("b");
        b.title = Some("A\\B".into());
        let plan = make(vec![b, a]);
        assert_eq!(plan.unresolved(), 2);
        for entry in &plan.entries {
            assert!(entry.sanitized);
            assert!(entry.issues.iter().any(|issue| issue.contains("collision")));
        }
    }
    #[test]
    fn rejects_invalid_disc_empty_sanitized_names_and_long_paths() {
        let mut a = track("a");
        a.track_number = Some(0);
        let mut b = track("b");
        b.disc_number = Some(0);
        let mut c = track("c");
        c.title = Some("...".into());
        let mut d = track("d");
        d.title = Some("x".repeat(181));
        assert_eq!(make(vec![a, b, c, d]).unresolved(), 3);
        assert!(destination(&track("a"), Path::new(&"x".repeat(230))).is_err());
    }
    #[test]
    fn compilation_tracks_share_folder_without_false_collision() {
        let a = track("a");
        let mut b = track("b");
        b.artist = Some("Other Performer".into());
        b.track_number = Some(4);
        let plan = make(vec![a, b]);
        assert_eq!(plan.unresolved(), 0);
        assert_eq!(
            plan.entries[0].destination.as_ref().unwrap().parent(),
            plan.entries[1].destination.as_ref().unwrap().parent()
        );
    }

    #[test]
    fn directory_aliases_block_distinct_filenames_and_reports_are_deterministic() {
        let generate = |reverse: bool| {
            let mut a = track("a");
            a.album = Some("A/B".into());
            let mut b = track("b");
            b.album = Some(r"a\b".into());
            b.track_number = Some(4);
            let tracks = if reverse { vec![b, a] } else { vec![a, b] };
            make(tracks)
        };
        let first = generate(false);
        assert_eq!(first.unresolved(), 2);
        assert!(first.entries.iter().all(|e| {
            e.issues
                .iter()
                .any(|issue| issue.contains("directory alias"))
        }));
        let mut a = Vec::new();
        let mut b = Vec::new();
        write_plan(&mut a, &first).unwrap();
        write_plan(&mut b, &generate(true)).unwrap();
        assert_eq!(a, b);
    }
}
