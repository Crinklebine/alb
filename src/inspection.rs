//! Metadata inspection is not audio decoding or validation.
use crate::source::SourceStamp;
use crate::{
    candidates::{FileType, classify},
    catalog::{Track, TrackCatalog},
};
use lofty::{
    config::{ParseOptions, ParsingMode},
    file::{AudioFile, FileType as ReaderType, TaggedFileExt},
    probe::Probe,
    tag::{Accessor, ItemKey},
};
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum InspectionError {
    Io(io::Error),
    Parse(lofty::error::FileParseError),
    NotRegularFile,
    SourceChanged,
}

impl fmt::Display for InspectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "cannot read source: {error}"),
            Self::Parse(error) => write!(f, "metadata parse failed: {error}"),
            Self::NotRegularFile => write!(f, "source is no longer a regular file"),
            Self::SourceChanged => write!(f, "source changed during metadata inspection"),
        }
    }
}

impl std::error::Error for InspectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::NotRegularFile | Self::SourceChanged => None,
        }
    }
}

/// Preserve every discovered file; read basic tags for all five supported types.
#[cfg(test)]
pub fn inspect(files: &[PathBuf]) -> TrackCatalog {
    inspect_with_key(files, None)
}

pub fn inspect_with_key(files: &[PathBuf], key: Option<crate::acoustid::ApiKey>) -> TrackCatalog {
    let mut lookup = key.map(crate::acoustid::AcoustId::new);
    let mut catalog = TrackCatalog::default();
    let progress = crate::progress::Progress::new("Reading metadata", Some(files.len()));
    for (index, path) in files.iter().enumerate() {
        progress.set(index);
        if classify(path) == FileType::Unknown {
            let mut track = Track::empty(path);
            match SourceStamp::at(path) {
                Ok(stamp) => track.source_stamp = Some(stamp),
                Err(error) => catalog
                    .errors
                    .push((path.clone(), InspectionError::Io(error).to_string())),
            }
            catalog.tracks.push(track);
            continue;
        }
        let (mut track, safe_to_lookup) = match inspect_known(path) {
            Ok(track) => (track, true),
            Err(error) => {
                let safe = matches!(error, InspectionError::Parse(_));
                catalog.errors.push((path.clone(), error.to_string()));
                let mut track = if safe {
                    inspect_known_mode(path, ParsingMode::BestAttempt)
                        .or_else(|error| match error {
                            InspectionError::Parse(_) => {
                                inspect_known_mode(path, ParsingMode::Relaxed)
                            }
                            error => Err(error),
                        })
                        .unwrap_or_else(|_| Track::empty(path))
                } else {
                    Track::empty(path)
                };
                if track.source_stamp.is_none() {
                    track.source_stamp = SourceStamp::at(path).ok();
                }
                (track, safe)
            }
        };
        if lookup.is_some() && safe_to_lookup && track.source_stamp.is_some() {
            recover_missing(
                &mut track,
                lookup
                    .as_mut()
                    .map(|l| l as &mut dyn crate::acoustid::Lookup),
            );
            // fpcalc reads the path independently. Reject changed source evidence.
            if SourceStamp::at(path).ok() != track.source_stamp {
                catalog
                    .errors
                    .push((path.clone(), InspectionError::SourceChanged.to_string()));
                track = Track::empty(path);
            }
        }
        catalog.tracks.push(track);
    }
    progress.set(files.len());
    drop(progress);
    if let Some(warning) = lookup.as_mut().and_then(|l| l.take_warning()) {
        eprintln!("{warning}");
    }
    catalog
}

fn recover_missing(track: &mut Track, lookup: Option<&mut dyn crate::acoustid::Lookup>) {
    let missing = |value: &Option<String>| value.as_ref().is_none_or(|s| s.trim().is_empty());
    if track.file_type == FileType::Unknown || (!missing(&track.artist) && !missing(&track.title)) {
        return;
    }
    if let Some(found) = lookup.and_then(|l| l.identify(&track.source_path)) {
        track.fingerprinted = true;
        track.metadata_update = Some(crate::tagging::MetadataUpdate {
            artist: missing(&track.artist).then(|| found.artist.clone()),
            title: missing(&track.title).then(|| found.title.clone()),
        });
        if missing(&track.artist) {
            track.artist = Some(found.artist);
        }
        if missing(&track.title) {
            track.title = Some(found.title);
        }
    }
}

fn inspect_known(path: &Path) -> Result<Track, InspectionError> {
    inspect_known_mode(path, ParsingMode::Strict)
}
fn inspect_known_mode(path: &Path, mode: ParsingMode) -> Result<Track, InspectionError> {
    // Recheck to avoid opening a stale symlink/special entry on a stable tree.
    // This is not race protection against concurrent filesystem replacement.
    if !fs::symlink_metadata(path)
        .map_err(InspectionError::Io)?
        .is_file()
    {
        return Err(InspectionError::NotRegularFile);
    }
    let expected = SourceStamp::at(path).map_err(InspectionError::Io)?;
    let mut file = crate::platform::open_snapshot(path).map_err(InspectionError::Io)?;
    let opened = SourceStamp::from_file(&file).map_err(InspectionError::Io)?;
    if opened != expected {
        return Err(InspectionError::SourceChanged);
    }
    let mut track = read_metadata_mode(path, &mut file, mode)?;
    let after = SourceStamp::from_file(&file).map_err(InspectionError::Io)?;
    if after != expected || SourceStamp::at(path).map_err(InspectionError::Io)? != expected {
        return Err(InspectionError::SourceChanged);
    }
    track.source_stamp = Some(expected);
    Ok(track)
}

#[cfg(test)]
fn read_metadata(
    path: &Path,
    reader: &mut (impl io::Read + io::Seek),
) -> Result<Track, InspectionError> {
    read_metadata_mode(path, reader, ParsingMode::Strict)
}
fn read_metadata_mode(
    path: &Path,
    reader: &mut (impl io::Read + io::Seek),
    mode: ParsingMode,
) -> Result<Track, InspectionError> {
    let options = ParseOptions::new().read_cover_art(false).parsing_mode(mode);
    let isolated = if classify(path) == FileType::Mp3 {
        mp3_blocks(reader)?
    } else {
        Vec::new()
    };
    let probe = match classify(path) {
        FileType::Flac => Probe::with_file_type(reader, ReaderType::Flac),
        FileType::M4a => Probe::with_file_type(reader, ReaderType::Mp4),
        FileType::Mp3 => Probe::with_file_type(reader, ReaderType::Mpeg),
        FileType::Wav => Probe::with_file_type(reader, ReaderType::Wav),
        // Let the metadata library select its Ogg tag reader. This never changes
        // ALB's extension-defined file type or asks for decoded audio.
        FileType::Ogg => Probe::new(reader)
            .guess_file_type()
            .map_err(InspectionError::Io)?,
        FileType::Unknown => return Ok(Track::empty(path)),
    };
    let file = probe
        .options(options)
        .read()
        .map_err(InspectionError::Parse)?;
    let tags: Vec<_> = isolated
        .iter()
        .chain(
            file.primary_tag().into_iter().chain(
                file.tags()
                    .iter()
                    .filter(|tag| tag.tag_type() != file.primary_tag_type()),
            ),
        )
        .collect();
    let mut track = Track::empty(path);
    for tag in tags {
        let text = |value: Option<std::borrow::Cow<'_, str>>| {
            value
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.into_owned())
        };
        for (field, old, new) in [
            ("Artist", &track.artist, text(tag.artist())),
            ("Title", &track.title, text(tag.title())),
            ("Album", &track.album, text(tag.album())),
        ] {
            if let (Some(old), Some(new)) = (old, new)
                && old != &new
            {
                let note =
                    format!("Metadata conflict: {field}: retained {old:?}; alternative {new:?}");
                if !track.metadata_notes.contains(&note) {
                    track.metadata_notes.push(note);
                }
            }
        }
        if track.title.is_none() {
            track.title = text(tag.title());
        }
        if track.artist.is_none() {
            track.artist = text(tag.artist());
        }
        if track.album.is_none() {
            track.album = text(tag.album());
        }
        if track.album_artist.is_none() {
            track.album_artist = tag
                .get_string(ItemKey::AlbumArtist)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned);
        }
        if track.track_number.is_none() {
            track.track_number = tag.track();
        }
        if track.disc_number.is_none() {
            track.disc_number = tag.disk();
        }
    }
    track.duration = Some(file.properties().duration());
    Ok(track)
}

// Read consecutive leading ID3 blocks separately before Lofty merges duplicate frames.
pub(crate) fn mp3_blocks(
    reader: &mut (impl io::Read + io::Seek),
) -> Result<Vec<lofty::tag::Tag>, InspectionError> {
    use io::{Cursor, Read, SeekFrom};
    let mut tags = Vec::new();
    reader
        .seek(SeekFrom::Start(0))
        .map_err(InspectionError::Io)?;
    let mut offset = 0;
    for _ in 0..32 {
        let mut header = [0; 10];
        match reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(InspectionError::Io(e)),
        }
        if &header[..3] != b"ID3" || header[6..].iter().any(|b| b & 128 != 0) {
            break;
        }
        let size = header[6..]
            .iter()
            .fold(0usize, |n, b| (n << 7) | *b as usize);
        if size > 16 * 1024 * 1024 {
            break;
        }
        let mut bytes = header.to_vec();
        let copied = reader
            .take(size as u64)
            .read_to_end(&mut bytes)
            .map_err(InspectionError::Io)?;
        if copied != size {
            break;
        }
        if let Ok(file) = Probe::with_file_type(Cursor::new(bytes), ReaderType::Mpeg)
            .options(
                ParseOptions::new()
                    .read_properties(false)
                    .read_cover_art(false)
                    .parsing_mode(ParsingMode::Relaxed),
            )
            .read()
            && let Some(tag) = file.primary_tag()
        {
            tags.push(tag.clone());
        }
        offset += 10
            + size as u64
            + if header[3] == 4 && header[5] & 16 != 0 {
                10
            } else {
                0
            };
        reader
            .seek(SeekFrom::Start(offset))
            .map_err(InspectionError::Io)?;
    }
    reader
        .seek(SeekFrom::Start(0))
        .map_err(InspectionError::Io)?;
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const OTHER_FORMATS: &[(&str, &[u8], &[u8])] = &[
        (
            "m4a",
            include_bytes!("../tests/fixtures/tone.m4a"),
            include_bytes!("../tests/fixtures/untagged.m4a"),
        ),
        (
            "mp3",
            include_bytes!("../tests/fixtures/tone.mp3"),
            include_bytes!("../tests/fixtures/untagged.mp3"),
        ),
        (
            "ogg",
            include_bytes!("../tests/fixtures/tone.ogg"),
            include_bytes!("../tests/fixtures/untagged.ogg"),
        ),
        (
            "wav",
            include_bytes!("../tests/fixtures/tone.wav"),
            include_bytes!("../tests/fixtures/untagged.wav"),
        ),
    ];
    #[test]
    fn duplicate_id3_blocks_preserve_first_usable_fields_and_report_conflicts() {
        fn block(fields: &[(&[u8; 4], &str)]) -> Vec<u8> {
            let mut frames = Vec::new();
            for (id, value) in fields {
                frames.extend_from_slice(*id);
                frames.extend_from_slice(&((value.len() + 1) as u32).to_be_bytes());
                frames.extend_from_slice(&[0, 0, 0]);
                frames.extend_from_slice(value.as_bytes());
            }
            frames.resize(frames.len() + 2048, 0);
            let mut bytes = b"ID3\x03\0\0".to_vec();
            bytes.extend(
                (0..4)
                    .rev()
                    .map(|i| ((frames.len() >> (i * 7)) & 127) as u8),
            );
            bytes.extend(frames);
            bytes
        }
        let mut bytes = block(&[(b"TPE1", "Artist"), (b"TIT2", "Song")]);
        bytes.extend(block(&[
            (b"TPE1", " "),
            (b"TIT2", "Track 08"),
            (b"TALB", "Album"),
        ]));
        bytes.extend_from_slice(include_bytes!("../tests/fixtures/untagged.mp3"));
        let track = read_metadata(Path::new("song.mp3"), &mut io::Cursor::new(&bytes)).unwrap();
        assert_eq!(track.artist.as_deref(), Some("Artist"));
        assert_eq!(track.title.as_deref(), Some("Song"));
        assert_eq!(track.album.as_deref(), Some("Album"));
        assert!(
            track
                .metadata_notes
                .iter()
                .any(|n| n.contains("Track 08") && n.contains("Song"))
        );
        let path =
            std::env::temp_dir().join(format!("alb-duplicate-tags-{}.mp3", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        crate::tagging::apply(
            &mut file,
            &crate::tagging::MetadataUpdate {
                artist: Some("Recovered Artist".into()),
                title: Some("Recovered Song".into()),
            },
            FileType::Mp3,
        )
        .unwrap();
        drop(file);
        let updated = inspect_known(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(updated.artist.as_deref(), Some("Recovered Artist"));
        assert_eq!(updated.title.as_deref(), Some("Recovered Song"));
    }

    #[test]
    fn malformed_year_does_not_hide_readable_identity_and_album() {
        let mut frames = Vec::new();
        for (id, value) in [
            (b"TPE1", "The Band"),
            (b"TIT2", "Song"),
            (b"TALB", "Album"),
            (b"TYER", "2013\x002013"),
        ] {
            frames.extend_from_slice(id);
            frames.extend_from_slice(&((value.len() + 1) as u32).to_be_bytes());
            frames.extend_from_slice(&[0, 0, 0]);
            frames.extend_from_slice(value.as_bytes());
        }
        let size = frames.len();
        let mut bytes = b"ID3\x03\0\0".to_vec();
        bytes.extend((0..4).rev().map(|i| ((size >> (i * 7)) & 127) as u8));
        bytes.extend(frames);
        bytes.extend_from_slice(include_bytes!("../tests/fixtures/untagged.mp3"));
        assert!(
            read_metadata_mode(
                Path::new("song.mp3"),
                &mut io::Cursor::new(&bytes),
                ParsingMode::BestAttempt
            )
            .is_err()
        );
        let path = std::env::temp_dir().join(format!("alb-year-{}.mp3", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let catalog = inspect(std::slice::from_ref(&path));
        std::fs::remove_file(path).unwrap();
        assert_eq!(catalog.errors.len(), 1);
        let track = &catalog.tracks[0];
        assert_eq!(track.artist.as_deref(), Some("The Band"));
        assert_eq!(track.title.as_deref(), Some("Song"));
        assert_eq!(track.album.as_deref(), Some("Album"));
        assert!(!track.fingerprinted);
    }

    #[test]
    fn reads_sorting_tags_for_every_supported_format_with_extension_only_groups() {
        for &(ext, bytes, _) in OTHER_FORMATS {
            let path = PathBuf::from(format!("tone.{}", ext.to_uppercase()));
            let track = read_metadata(&path, &mut io::Cursor::new(bytes)).unwrap();
            assert_eq!(track.file_type, classify(&path));
            assert_eq!(track.title.as_deref(), Some("Fixture Tone"), "{ext}");
            assert_eq!(track.artist.as_deref(), Some("Test Artist"), "{ext}");
            assert_eq!(
                track.album_artist.as_deref(),
                Some("Test Ensemble"),
                "{ext}"
            );
            assert_eq!(track.album.as_deref(), Some("Test Album"), "{ext}");
            assert_eq!(track.track_number, Some(3), "{ext}");
            assert_eq!(track.disc_number, Some(2), "{ext}");
            assert!(track.duration.unwrap() > Duration::ZERO);
        }
    }
    #[test]
    fn untagged_files_keep_generic_fields_absent_and_malformed_files_fail() {
        for &(ext, _, untagged) in OTHER_FORMATS {
            let path = PathBuf::from(format!("Artist - Song.{ext}"));
            let track = read_metadata(&path, &mut io::Cursor::new(untagged)).unwrap();
            assert!(track.title.is_none() && track.artist.is_none() && track.album.is_none());
            assert!(
                track.album_artist.is_none()
                    && track.track_number.is_none()
                    && track.disc_number.is_none()
            );
            assert!(read_metadata(&path, &mut io::Cursor::new(b"not audio")).is_err());
        }
    }

    #[test]
    fn reads_basic_generated_flac_metadata() {
        let bytes = include_bytes!("../tests/fixtures/tone.flac");
        let track = read_metadata(Path::new("tone.flac"), &mut io::Cursor::new(bytes)).unwrap();
        assert_eq!(track.title.as_deref(), Some("Fixture Tone"));
        assert_eq!(track.artist.as_deref(), Some("Test Artist"));
        assert_eq!(track.album_artist.as_deref(), Some("Test Ensemble"));
        assert_eq!(track.album.as_deref(), Some("Test Album"));
        assert_eq!(track.track_number, Some(3));
        assert_eq!(track.disc_number, Some(2));
        assert_eq!(track.duration, Some(Duration::from_millis(50)));
    }

    #[test]
    fn rejects_non_flac_bytes_even_with_flac_extension() {
        for bytes in [b"not audio".as_slice(), b"", b"fLaC"] {
            assert!(matches!(
                read_metadata(Path::new("fake.flac"), &mut io::Cursor::new(bytes)),
                Err(InspectionError::Parse(_))
            ));
        }
    }
    // Derive variants from the original synthetic recording without an encoder.
    fn without_tags() -> Vec<u8> {
        let original = include_bytes!("../tests/fixtures/tone.flac");
        let mut offset = 4;
        loop {
            let header = &original[offset..offset + 4];
            let last = header[0] & 0x80 != 0;
            let length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
            offset += 4 + length;
            if last {
                break;
            }
        }
        // Keep STREAMINFO and encoded samples; discard comments and padding.
        let mut bytes = original[..42].to_vec();
        bytes[4] = 0x80;
        bytes.extend_from_slice(&original[offset..]);
        bytes
    }

    #[test]
    fn absent_tags_remain_absent_instead_of_being_inferred() {
        let bytes = without_tags();
        let track = read_metadata(
            Path::new("Artist - Title.flac"),
            &mut io::Cursor::new(bytes),
        )
        .unwrap();
        assert_eq!(track.title, None);
        assert_eq!(track.artist, None);
        assert_eq!(track.album_artist, None);
        assert_eq!(track.album, None);
        assert_eq!(track.track_number, None);
        assert_eq!(track.disc_number, None);
        assert_eq!(track.duration, Some(Duration::from_millis(50)));
    }

    #[test]
    fn truncated_streaminfo_and_comment_block_fail() {
        let bytes = include_bytes!("../tests/fixtures/tone.flac");
        for end in 0..42 {
            assert!(
                read_metadata(
                    Path::new("truncated.flac"),
                    &mut io::Cursor::new(&bytes[..end])
                )
                .is_err(),
                "length {end}"
            );
        }
        // STREAMINFO is complete but the next metadata block is incomplete.
        for end in [43, 46, 50] {
            assert!(
                read_metadata(
                    Path::new("truncated.flac"),
                    &mut io::Cursor::new(&bytes[..end])
                )
                .is_err(),
                "length {end}"
            );
        }
    }

    #[test]
    fn display_escapes_control_characters_in_metadata_and_paths() {
        let mut track = read_metadata(
            Path::new("line\nbreak.flac"),
            &mut io::Cursor::new(without_tags()),
        )
        .unwrap();
        track.title = Some("title\n\u{1b}[31m".into());
        let text = track.to_string();
        assert!(!text.contains('\n'));
        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("\\n"));
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::acoustid::{Identification, Lookup};
    struct Fake {
        calls: usize,
        succeeds: bool,
    }
    impl Lookup for Fake {
        fn identify(&mut self, _: &Path) -> Option<Identification> {
            self.calls += 1;
            self.succeeds.then(|| Identification {
                artist: "Recovered Artist".into(),
                title: "Recovered Title".into(),
            })
        }
    }
    #[test]
    fn lookup_is_optional_supported_only_and_never_triggered_by_album_or_numbers() {
        let mut track = Track::empty(Path::new("missing.flac"));
        recover_missing(&mut track, None);
        assert!(track.artist.is_none() && track.title.is_none());
        let mut fake = Fake {
            calls: 0,
            succeeds: true,
        };
        track.artist = Some("Embedded Artist".into());
        track.title = Some("Embedded Title".into());
        recover_missing(&mut track, Some(&mut fake));
        assert_eq!(fake.calls, 0);
        assert!(
            track.album.is_none()
                && track.album_artist.is_none()
                && track.track_number.is_none()
                && track.disc_number.is_none()
        );
        let mut unknown = Track::empty(Path::new("unknown.bin"));
        recover_missing(&mut unknown, Some(&mut fake));
        assert_eq!(fake.calls, 0);
    }
    #[test]
    fn fills_only_missing_or_blank_artist_title_preserving_other_fields() {
        for (artist, title) in [
            (None, Some("Embedded Title")),
            (Some("Embedded Artist"), None),
            (None, None),
            (Some(" \t"), Some("\n")),
        ] {
            let mut track = Track::empty(Path::new("track.m4a"));
            track.artist = artist.map(str::to_owned);
            track.title = title.map(str::to_owned);
            track.album = Some("Embedded Album".into());
            track.track_number = Some(4);
            let mut fake = Fake {
                calls: 0,
                succeeds: true,
            };
            recover_missing(&mut track, Some(&mut fake));
            assert_eq!(fake.calls, 1);
            assert!(track.fingerprinted);
            assert_eq!(
                track.artist.as_deref(),
                Some(
                    artist
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("Recovered Artist")
                )
            );
            assert_eq!(
                track.title.as_deref(),
                Some(
                    title
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("Recovered Title")
                )
            );
            assert_eq!(track.album.as_deref(), Some("Embedded Album"));
            assert_eq!(track.track_number, Some(4));
            assert!(track.album_artist.is_none() && track.disc_number.is_none());
        }
    }
    #[test]
    fn lookup_failures_preserve_existing_missing_metadata_behavior() {
        let mut track = Track::empty(Path::new("track.ogg"));
        track.title = Some("Embedded Title".into());
        let mut fake = Fake {
            calls: 0,
            succeeds: false,
        };
        recover_missing(&mut track, Some(&mut fake));
        assert!(track.artist.is_none());
        assert!(!track.fingerprinted);
        assert_eq!(track.title.as_deref(), Some("Embedded Title"));
    }
}
