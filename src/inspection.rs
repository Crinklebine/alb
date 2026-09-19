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
pub fn inspect(files: &[PathBuf]) -> TrackCatalog {
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
        match inspect_known(path) {
            Ok(track) => catalog.tracks.push(track),
            Err(error) => {
                catalog.errors.push((path.clone(), error.to_string()));
                let mut track = Track::empty(path);
                track.source_stamp = SourceStamp::at(path).ok();
                catalog.tracks.push(track);
            }
        }
    }
    progress.set(files.len());
    catalog
}

fn inspect_known(path: &Path) -> Result<Track, InspectionError> {
    // Recheck to avoid opening a stale symlink/special entry on a stable tree.
    // This is not race protection against concurrent filesystem replacement.
    if !fs::symlink_metadata(path)
        .map_err(InspectionError::Io)?
        .is_file()
    {
        return Err(InspectionError::NotRegularFile);
    }
    let expected = SourceStamp::at(path).map_err(InspectionError::Io)?;
    let mut file = fs::File::open(path).map_err(InspectionError::Io)?;
    let opened = SourceStamp::from_metadata(&file.metadata().map_err(InspectionError::Io)?)
        .map_err(InspectionError::Io)?;
    if opened != expected {
        return Err(InspectionError::SourceChanged);
    }
    let mut track = read_metadata(path, &mut file)?;
    let after = SourceStamp::from_metadata(&file.metadata().map_err(InspectionError::Io)?)
        .map_err(InspectionError::Io)?;
    if after != expected || SourceStamp::at(path).map_err(InspectionError::Io)? != expected {
        return Err(InspectionError::SourceChanged);
    }
    track.source_stamp = Some(expected);
    Ok(track)
}

fn read_metadata(
    path: &Path,
    reader: &mut (impl io::Read + io::Seek),
) -> Result<Track, InspectionError> {
    let options = ParseOptions::new()
        .read_cover_art(false)
        .parsing_mode(ParsingMode::Strict);
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
    let tags: Vec<_> = file
        .primary_tag()
        .into_iter()
        .chain(
            file.tags()
                .iter()
                .filter(|tag| tag.tag_type() != file.primary_tag_type()),
        )
        .collect();
    let mut track = Track::empty(path);
    for tag in tags {
        let text = |value: Option<std::borrow::Cow<'_, str>>| {
            value
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.into_owned())
        };
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
